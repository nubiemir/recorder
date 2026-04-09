use std::sync::{Arc, mpsc::Sender};

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
use log::{error, info, warn};
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

    let data_channel = webrtc.emit_by_name::<Option<WebRTCDataChannel>>(
        "create-data-channel",
        &[&"JVB data", &None::<Structure>],
    );

    if let Some(dc) = data_channel {
        info!("Label: {:?}", dc.label());
    } else {
        error!("Failed to create data channel object. Check if gst-plugins-bad is installed.");
    }

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

    fn add_and_sync(pipeline: &Pipeline, elem: &Element) {
        pipeline.add(elem).unwrap();
        elem.sync_state_with_parent().unwrap();
    }

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

        if media != "audio" {
            info!("Ignoring non-audio pad");
            return;
        }

        if encoding != "OPUS" {
            warn!("Unsupported audio encoding: {encoding}");
            return;
        }

        let pipeline = webrtc
            .parent()
            .and_then(|p| p.downcast::<Pipeline>().ok())
            .unwrap();

        let depay = ElementFactory::make("rtpopusdepay").build().unwrap();
        let dec = ElementFactory::make("opusdec").build().unwrap();
        let conv = ElementFactory::make("audioconvert").build().unwrap();
        let resample = ElementFactory::make("audioresample").build().unwrap();
        let sink = ElementFactory::make("autoaudiosink").build().unwrap();

        for e in [&depay, &dec, &conv, &resample, &sink] {
            add_and_sync(&pipeline, e);
        }

        Element::link_many([&depay, &dec, &conv, &resample, &sink].as_slice()).unwrap();

        let sink_pad = depay.static_pad("sink").unwrap();
        if let Err(err) = pad.link(&sink_pad) {
            warn!("Failed to link webrtc pad to depayloader: {:?}", err);
            return;
        }

        info!("Audio playback pipeline linked");
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
                    info!("sdp_answer: {}", sdp);
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
