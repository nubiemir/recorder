use std::{
    sync::{Arc, Mutex, mpsc::Sender},
    thread,
    time::Duration,
};

use crate::{
    jingle::{Iq, Jingle},
    sdp::{sdp_util::parse_candidate, to_jingle},
    xmpp::xep::XEP,
};
use gstreamer::{
    Caps, Element, ElementFactory, Pipeline, Promise, State, Structure, glib::BoolError, prelude::*,
};
use gstreamer_sdp::SDPMessage;
use gstreamer_webrtc::{
    WebRTCDataChannel, WebRTCRTPTransceiver, WebRTCRTPTransceiverDirection,
    WebRTCSessionDescription,
};

use libstrophe::Stanza;
use log::{debug, error, info, warn};
use nanoid::nanoid;
use webrtc_sdp::{
    SdpType,
    attribute_type::{SdpAttribute, parse_attribute},
    parse_sdp,
};

pub fn webrtcbin(
    offer: &str,
    jingle: Arc<Jingle>,
    iq: Arc<Iq>,
    tx: Sender<Stanza>,
) -> Result<(), BoolError> {
    let pipeline = Pipeline::new();

    let webrtc = ElementFactory::make("webrtcbin").build()?;

    pipeline.add(&webrtc)?;
    webrtc.set_property("stun-server", "stun://stun.l.google.com:19302");
    webrtc.set_property_from_str("bundle-policy", "max-bundle");

    webrtc.emit_by_name::<WebRTCRTPTransceiver>(
        "add-transceiver",
        &[
            &WebRTCRTPTransceiverDirection::Recvonly,
            &Caps::builder("application/x-rtp")
                .field("media", "audio")
                .build(),
        ],
    );

    webrtc.emit_by_name::<WebRTCRTPTransceiver>(
        "add-transceiver",
        &[
            &WebRTCRTPTransceiverDirection::Recvonly,
            &Caps::builder("application/x-rtp")
                .field("media", "video")
                .build(),
        ],
    );

    webrtc.connect("on-data-channel", false, move |values| {
        match values[1].get::<WebRTCDataChannel>() {
            Ok(dc) => {
                info!("Received remote data channel: {:?}", dc.label());

                dc.connect_notify(Some("ready-state"), |dc, _| {
                    let state = dc.property_value("ready-state");
                    info!("Remote data channel ready-state changed: {:?}", state);
                });

                dc.connect_notify(Some("buffered-amount"), |dc, _| {
                    let amt = dc.property_value("buffered-amount");
                    info!("Remote data channel buffered-amount: {:?}", amt);
                });
            }
            Err(err) => {
                warn!("Failed to extract on-data-channel value: {:?}", err);
            }
        }

        None
    });

    let mux = ElementFactory::make("mp4mux").build()?;
    let sink = ElementFactory::make("filesink").build()?;
    sink.set_property("location", "recording.mp4");

    pipeline.add_many([&mux, &sink].as_slice())?;
    mux.link(&sink)?;
    mux.sync_state_with_parent()?;
    sink.sync_state_with_parent()?;

    // avoid linking the same kind twice
    let linked_streams = Arc::new(Mutex::new((false, false))); // (audio_linked, video_linked)

    let mux = mux.clone();
    let linked_streams = Arc::clone(&linked_streams);

    webrtc.connect_pad_added(move |webrtc, pad| {
        let caps = match pad.current_caps() {
            Some(c) => c,
            None => {
                warn!("pad-added with no caps");
                return;
            }
        };

        let s = match caps.structure(0) {
            Some(s) => s,
            None => {
                warn!("caps has no structure");
                return;
            }
        };

        let media = s.get::<&str>("media").unwrap_or("unknown");
        let encoding = s.get::<&str>("encoding-name").unwrap_or("unknown");

        info!("New pad: media={media}, encoding={encoding}, caps={caps}");

        let pipeline = match webrtc.parent().and_then(|p| p.downcast::<Pipeline>().ok()) {
            Some(p) => p,
            None => {
                warn!("Failed to get parent pipeline from webrtcbin");
                return;
            }
        };

        let mut linked = linked_streams.lock().unwrap();

        match (media, encoding) {
            ("audio", "OPUS") => {
                if linked.0 {
                    info!("Audio already linked, ignoring extra audio pad");
                    return;
                }

                let depay = match ElementFactory::make("rtpopusdepay").build() {
                    Ok(e) => e,
                    Err(err) => {
                        warn!("Failed to create rtpopusdepay: {:?}", err);
                        return;
                    }
                };

                // let parse = match ElementFactory::make("opusparse").build() {
                //     Ok(e) => e,
                //     Err(err) => {
                //         warn!("Failed to create opusparse: {:?}", err);
                //         return;
                //     }
                // };

                let dec = match ElementFactory::make("opusdec").build() {
                    Ok(e) => e,
                    Err(err) => {
                        warn!("Failed to create opusdec: {:?}", err);
                        return;
                    }
                };

                let conv = match ElementFactory::make("audioconvert").build() {
                    Ok(e) => e,
                    Err(err) => {
                        warn!("Failed to create conv: {:?}", err);
                        return;
                    }
                };

                let resample = match ElementFactory::make("audioresample").build() {
                    Ok(e) => e,
                    Err(err) => {
                        warn!("Failed to create audioresample: {:?}", err);
                        return;
                    }
                };

                let enc = match ElementFactory::make("avenc_aac").build() {
                    Ok(e) => e,
                    Err(err) => {
                        warn!("Failed to create enc: {:?}", err);
                        return;
                    }
                };

                let queue = match ElementFactory::make("queue").build() {
                    Ok(e) => e,
                    Err(err) => {
                        warn!("Failed to create audio queue: {:?}", err);
                        return;
                    }
                };

                if let Err(err) =
                    pipeline.add_many([&depay, &dec, &conv, &resample, &enc, &queue].as_slice())
                {
                    warn!("Failed to add audio elements: {:?}", err);
                    return;
                }

                for e in [&depay, &dec, &conv, &resample, &enc, &queue] {
                    if let Err(err) = e.sync_state_with_parent() {
                        warn!("Failed to sync audio element state: {:?}", err);
                        return;
                    }
                }

                if let Err(err) =
                    Element::link_many([&depay, &dec, &conv, &resample, &enc, &queue].as_slice())
                {
                    warn!("Failed to link audio chain: {:?}", err);
                    return;
                }

                let depay_sink = match depay.static_pad("sink") {
                    Some(p) => p,
                    None => {
                        warn!("Audio depay has no sink pad");
                        return;
                    }
                };

                if let Err(err) = pad.link(&depay_sink) {
                    warn!("Failed to link webrtc audio pad: {:?}", err);
                    return;
                }

                let mux_pad = match mux.request_pad_simple("audio_%u") {
                    Some(p) => p,
                    None => {
                        warn!("Failed to request audio_%u pad from matroskamux");
                        return;
                    }
                };

                let queue_src = match queue.static_pad("src") {
                    Some(p) => p,
                    None => {
                        warn!("Audio queue has no src pad");
                        return;
                    }
                };

                if let Err(err) = queue_src.link(&mux_pad) {
                    warn!("Failed to link audio queue to mux: {:?}", err);
                    return;
                }

                linked.0 = true;
                info!("Audio recording branch linked");
            }

            ("video", "AV1") => {
                if linked.1 {
                    info!("Video already linked, ignoring extra video pad");
                    return;
                }

                let depay = match ElementFactory::make("rtpav1depay").build() {
                    Ok(e) => e,
                    Err(err) => {
                        warn!("Failed to create rtpav1depay: {:?}", err);
                        return;
                    }
                };

                let parse = match ElementFactory::make("av1parse").build() {
                    Ok(e) => e,
                    Err(err) => {
                        warn!("Failed to create av1parse: {:?}", err);
                        return;
                    }
                };

                let dec = match ElementFactory::make("dav1ddec").build() {
                    Ok(e) => e,
                    Err(err) => {
                        warn!("Failed to create dav1ddec: {:?}", err);
                        return;
                    }
                };

                let conv = match ElementFactory::make("videoconvert").build() {
                    Ok(e) => e,
                    Err(err) => {
                        warn!("Failed to create conv: {:?}", err);
                        return;
                    }
                };
                let enc = match ElementFactory::make("x264enc").build() {
                    Ok(e) => e,
                    Err(err) => {
                        warn!("Failed to create enc: {:?}", err);
                        return;
                    }
                };

                let queue = match ElementFactory::make("queue").build() {
                    Ok(e) => e,
                    Err(err) => {
                        warn!("Failed to create video queue: {:?}", err);
                        return;
                    }
                };

                if let Err(err) =
                    pipeline.add_many([&depay, &parse, &dec, &conv, &enc, &queue].as_slice())
                {
                    warn!("Failed to add video elements: {:?}", err);
                    return;
                }

                for e in [&depay, &parse, &dec, &conv, &enc, &queue] {
                    if let Err(err) = e.sync_state_with_parent() {
                        warn!("Failed to sync video element state: {:?}", err);
                        return;
                    }
                }

                if let Err(err) =
                    Element::link_many([&depay, &parse, &dec, &conv, &enc, &queue].as_slice())
                {
                    warn!("Failed to link video chain: {:?}", err);
                    return;
                }

                let depay_sink = match depay.static_pad("sink") {
                    Some(p) => p,
                    None => {
                        warn!("Video depay has no sink pad");
                        return;
                    }
                };

                if let Err(err) = pad.link(&depay_sink) {
                    warn!("Failed to link webrtc video pad: {:?}", err);
                    return;
                }

                let mux_pad = match mux.request_pad_simple("video_%u") {
                    Some(p) => p,
                    None => {
                        warn!("Failed to request video_%u pad from matroskamux");
                        return;
                    }
                };

                let queue_src = match queue.static_pad("src") {
                    Some(p) => p,
                    None => {
                        warn!("Video queue has no src pad");
                        return;
                    }
                };

                if let Err(err) = queue_src.link(&mux_pad) {
                    warn!("Failed to link video queue to mux: {:?}", err);
                    return;
                }

                linked.1 = true;
                info!("AV1 video recording branch linked");
            }

            _ => {
                warn!("Unsupported pad for recording: media={media}, encoding={encoding}");
            }
        }
    });

    let sdp_message = SDPMessage::parse_buffer(offer.as_bytes())?;
    let sdp_offer =
        WebRTCSessionDescription::new(gstreamer_webrtc::WebRTCSDPType::Offer, sdp_message);

    let webrtc_ref = Arc::new(webrtc);
    let webrtc_for_answer = Arc::clone(&webrtc_ref);
    let iq_ref = Arc::clone(&iq);
    let jingle_ref = Arc::clone(&jingle);
    let tx_ref = tx.clone();

    let local_credentials = Arc::new(std::sync::Mutex::new((String::new(), String::new())));
    let creds_for_answer = Arc::clone(&local_credentials);

    let bus = pipeline.bus().unwrap();
    let pipeline_clone = pipeline.clone();

    thread::spawn(move || {
        thread::sleep(Duration::from_secs(50));

        println!("⏹ Stopping pipeline after 20 seconds...");

        if !pipeline_clone.send_event(gstreamer::event::Eos::new()) {
            eprintln!("Failed to send EOS");
        }
    });

    let pipeline_clone = pipeline.clone();
    thread::spawn(move || {
        for msg in bus.iter_timed(gstreamer::ClockTime::NONE) {
            use gstreamer::MessageView;

            match msg.view() {
                MessageView::Eos(..) => {
                    println!("✅ EOS received, shutting down pipeline");
                    let _ = pipeline_clone.set_state(gstreamer::State::Null);
                    break;
                }
                MessageView::Error(err) => {
                    eprintln!("❌ Error: {:?}", err);
                    let _ = pipeline_clone.set_state(gstreamer::State::Null);
                    break;
                }
                _ => {}
            }
        }
    });

    webrtc_for_answer
        .parent()
        .and_then(|p| p.downcast::<Pipeline>().ok())
        .unwrap()
        .set_state(State::Playing)
        .expect("Failed to start pipeline");

    let answer_promise = Promise::with_change_func(move |reply| {
        let reply = match reply {
            Ok(Some(structure)) => structure,
            Ok(None) => {
                error!("Promise replied with no structure (None)");
                return;
            }
            Err(e) => {
                error!("Promise failed: {:?}", e);
                return;
            }
        };

        let answer = match reply.get::<WebRTCSessionDescription>("answer") {
            Ok(desc) => desc,
            Err(e) => {
                error!("Field 'answer' was missing or wrong type: {:?}", e);
                return;
            }
        };

        webrtc_for_answer.emit_by_name::<()>("set-local-description", &[&answer, &None::<Promise>]);

        match answer.sdp().as_text() {
            Ok(sdp) => match parse_sdp(sdp.as_str(), true) {
                Ok(sdp_sess) => {
                    debug!("sdp_answer: {}", sdp);
                    let jingle_stanza = to_jingle(&sdp_sess, jingle.clone());

                    let mut ice_ufrag = String::new();
                    let mut ice_pwd = String::new();

                    for line in sdp.lines() {
                        if let Some(value) = line.strip_prefix("a=ice-ufrag:") {
                            ice_ufrag = value.trim().to_string();
                        }
                        if let Some(value) = line.strip_prefix("a=ice-pwd:") {
                            ice_pwd = value.trim().to_string();
                        }
                    }

                    let mut creds = creds_for_answer.lock().unwrap();
                    *creds = (ice_ufrag, ice_pwd);

                    match jingle_stanza {
                        Ok(stanza) => {
                            let id = nanoid!();
                            let mut iq_stanza = Stanza::new_iq(Some("set"), Some(id.as_str()));
                            iq_stanza.set_attribute("from", iq_ref.to.as_str()).unwrap();
                            iq_stanza.set_attribute("to", iq_ref.from.as_str()).unwrap();
                            iq_stanza.add_child(stanza).unwrap();
                            tx_ref.send(iq_stanza).expect("Failed to send stanza");
                        }

                        Err(err) => {
                            warn!("jingle stanza error: {}", err);
                        }
                    }
                }
                Err(err) => {
                    error!("Error Parsing Sdp: {}", err)
                }
            },
            Err(e) => error!("Failed to get SDP text: {e:?}"),
        }
    });

    let webrtc_for_answer = Arc::clone(&webrtc_ref);
    let set_description_promise = Promise::with_change_func(move |reply| {
        match reply {
            Ok(_) => {
                info!("Remote description successfully applied. Now creating answer...");

                let data_channel = webrtc_for_answer.emit_by_name::<Option<WebRTCDataChannel>>(
                    "create-data-channel",
                    &[
                        &"JVB data",
                        &Structure::builder("config")
                            .field("protocol", "http://jitsi.org/protocols/colibri")
                            .build(),
                    ],
                );

                if let Some(dc) = data_channel {
                    dc.connect_on_open(move |dc| {
                        info!("🚀 JVB confirmed DataChannel open. Sending constraints...");

                        let msg = r#"{
    "colibriClass":"ReceiverVideoConstraints",
    "lastN":-1,
    "defaultConstraints":{"maxHeight":720}
    }"#;

                        // let bytes = Bytes::from(msg.as_bytes());

                        match dc.send_string_full(Some(msg)) {
                            Ok(_) => info!("✅ Constraints sent successfully via send_data_full"),
                            Err(e) => error!("❌ Failed to send: {:?}", e),
                        }
                    });
                    dc.connect_on_error(move |_dc, err| {
                        warn!("Channel error: {:?}", err);
                    });
                    dc.connect_on_message_string(move |_dc, data| match data {
                        Some(data) => {
                            info!("data on message: {:?}", data);
                        }
                        None => {}
                    });
                } else {
                    error!(
                        "Failed to create data channel object. Check if gst-plugins-bad is installed."
                    );
                }
                // Trigger create-answer ONLY now
                webrtc_for_answer
                    .emit_by_name::<()>("create-answer", &[&None::<Structure>, &answer_promise]);
            }
            Err(e) => {
                error!("Failed to set remote description: {:?}", e);
            }
        }
    });

    let webrtc_for_remote_description = Arc::clone(&webrtc_ref);
    webrtc_for_remote_description.emit_by_name::<()>(
        "set-remote-description",
        &[&sdp_offer, &set_description_promise],
    );
    info!("Remote description set");

    let webrtc_for_ice = Arc::clone(&webrtc_ref);
    let iq_webrtc_for_ice = Arc::clone(&iq);
    let jingle_webrtc_for_ice = Arc::clone(&jingle_ref);
    let tx_webrtc_for_ice = tx.clone();
    let creds_for_ice = Arc::clone(&local_credentials);

    webrtc_for_ice.connect("on-ice-candidate", false, move |values| {
        let mline_index = values[1].get::<u32>().unwrap_or(0);
        let candidate = values[2].get::<String>().unwrap_or_default();

        if candidate.is_empty() {
            return None;
        }

        let content_name = match mline_index {
            0 => "audio",
            1 => "video",
            2 => "data",
            _ => {
                warn!("Unknown mline index: {}", mline_index);
                return None;
            }
        };

        let parsed_candidate = parse_attribute(candidate.as_str());
        if let Ok(SdpType::Attribute(SdpAttribute::Candidate(candidate))) = parsed_candidate {
            let mut candidate_stanza = Stanza::new();
            if let Ok(()) = parse_candidate(&mut candidate_stanza, &candidate) {
                let mut iq_stanza = Stanza::new_iq(Some("set"), Some(nanoid!().as_str()));

                iq_stanza
                    .set_attribute("to", iq_webrtc_for_ice.from.as_str())
                    .unwrap();

                iq_stanza
                    .set_attribute("from", iq_webrtc_for_ice.to.as_str())
                    .unwrap();

                let mut jingle_stanza = Stanza::new();
                jingle_stanza.set_name("jingle").unwrap();
                jingle_stanza.set_ns("urn:xmpp:jingle:1").unwrap();
                jingle_stanza
                    .set_attribute("action", "transport-info")
                    .unwrap();
                jingle_stanza
                    .set_attribute("initiator", jingle_webrtc_for_ice.get_initiator())
                    .unwrap();
                jingle_stanza
                    .set_attribute("sid", jingle_webrtc_for_ice.get_sid())
                    .unwrap();
                jingle_stanza
                    .set_attribute("responder", jingle_webrtc_for_ice.get_responder())
                    .unwrap();

                let mut content_stanza = Stanza::new();
                content_stanza.set_name("content").unwrap();
                content_stanza.set_attribute("name", content_name).unwrap();

                content_stanza
                    .set_attribute("senders", "initiator")
                    .unwrap();

                content_stanza
                    .set_attribute("creator", "initiator")
                    .unwrap();

                let mut transport_stanza = Stanza::new();
                transport_stanza.set_name("transport").unwrap();
                transport_stanza
                    .set_ns(XEP::IceUdpTransport.to_string())
                    .unwrap();

                let creds = creds_for_ice.lock().unwrap();
                if !creds.0.is_empty() {
                    transport_stanza.set_attribute("ufrag", &creds.0).unwrap();
                    transport_stanza.set_attribute("pwd", &creds.1).unwrap();
                }

                transport_stanza.add_child(candidate_stanza).unwrap();

                content_stanza.add_child(transport_stanza).unwrap();

                jingle_stanza.add_child(content_stanza).unwrap();

                iq_stanza.add_child(jingle_stanza).unwrap();

                tx_webrtc_for_ice
                    .send(iq_stanza)
                    .expect("couldn't send transport-info");
            }
        }

        None
    });

    info!("Pipeline started");

    Ok(())
}
