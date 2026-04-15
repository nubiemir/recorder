use std::{
    sync::{Arc, Mutex, mpsc::Sender},
    thread,
};

use crate::{
    jingle::{Iq, Jingle},
    sdp::{sdp_util::parse_candidate, to_jingle},
    xmpp::xep::XEP,
};

use gstreamer::{
    self as gst, Caps, Element, ElementFactory, Pipeline, Promise, State, Structure,
    glib::{self, BoolError},
    prelude::*,
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

#[derive(Clone)]
struct RecordingHandles {
    audio_selector: Element,
    audio_fallback_sink_pad: gst::Pad,
    audio_live_sink_pad: gst::Pad,
    audio_live_linked: bool,

    video_selector: Element,
    video_fallback_sink_pad: gst::Pad,
    video_live_sink_pad: gst::Pad,
    video_live_linked: bool,

    audio_live_queue: Element,
    video_live_queue: Element,
}

fn make(name: &str) -> Result<Element, BoolError> {
    ElementFactory::make(name).build().map_err(|e| {
        error!("failed to create element {name}: {e:?}");
        glib::bool_error!("failed to create element {name}")
    })
}

fn sync_all(elements: &[&Element]) -> Result<(), BoolError> {
    for e in elements {
        e.sync_state_with_parent()?;
    }
    Ok(())
}

fn link_many(elements: &[&Element]) -> Result<(), BoolError> {
    Element::link_many(elements).map_err(|e| glib::bool_error!("failed to link elements: {e:?}"))
}

fn build_recording_graph(
    pipeline: &Pipeline,
    output_path: &str,
) -> Result<Arc<Mutex<RecordingHandles>>, BoolError> {
    // ---------------- MUX + SINK ----------------
    let mux = make("matroskamux")?;
    mux.set_property_from_str("writing-app", "gst-webrtc-recorder");

    let sink = make("filesink")?;
    sink.set_property("location", output_path);
    sink.set_property("sync", false);

    // ---------------- AUDIO FALLBACK ----------------
    let audio_fallback_src = make("audiotestsrc")?;
    audio_fallback_src.set_property_from_str("wave", "silence");
    audio_fallback_src.set_property("is-live", true);

    let audio_fallback_convert = make("audioconvert")?;
    let audio_fallback_resample = make("audioresample")?;
    let audio_fallback_queue = make("queue")?;
    audio_fallback_queue.set_property("max-size-time", 0u64);
    audio_fallback_queue.set_property("max-size-bytes", 0u32);
    audio_fallback_queue.set_property("max-size-buffers", 0u32);

    let audio_fallback_capsfilter = make("capsfilter")?;
    audio_fallback_capsfilter.set_property(
        "caps",
        Caps::builder("audio/x-raw")
            .field("rate", 48000i32)
            .field("channels", 2i32)
            .build(),
    );

    // ---------------- AUDIO LIVE ENTRY ----------------
    let audio_live_queue = make("queue")?;
    audio_live_queue.set_property("max-size-time", 0u64);
    audio_live_queue.set_property("max-size-bytes", 0u32);
    audio_live_queue.set_property("max-size-buffers", 0u32);

    let audio_live_convert = make("audioconvert")?;
    let audio_live_resample = make("audioresample")?;

    let audio_live_capsfilter = make("capsfilter")?;
    audio_live_capsfilter.set_property(
        "caps",
        Caps::builder("audio/x-raw")
            .field("rate", 48000i32)
            .field("channels", 2i32)
            .build(),
    );

    // ---------------- AUDIO OUTPUT ----------------
    let audio_selector = make("input-selector")?;
    let audio_post_convert = make("audioconvert")?;
    let audio_post_resample = make("audioresample")?;
    let audio_enc = make("avenc_aac")?;
    let audio_out_queue = make("queue")?;
    audio_out_queue.set_property("max-size-time", 0u64);
    audio_out_queue.set_property("max-size-bytes", 0u32);
    audio_out_queue.set_property("max-size-buffers", 0u32);

    let audio_post_capsfilter = make("capsfilter")?;
    audio_post_capsfilter.set_property(
        "caps",
        Caps::builder("audio/x-raw")
            .field("rate", 48000i32)
            .field("channels", 2i32)
            .build(),
    );

    // ---------------- VIDEO FALLBACK ----------------
    let video_fallback_src = make("videotestsrc")?;
    video_fallback_src.set_property_from_str("pattern", "black");
    video_fallback_src.set_property("is-live", true);

    let video_fallback_convert = make("videoconvert")?;
    let video_fallback_rate = make("videorate")?;
    let video_fallback_capsfilter = make("capsfilter")?;
    video_fallback_capsfilter.set_property(
        "caps",
        Caps::builder("video/x-raw")
            .field("framerate", gst::Fraction::new(30, 1))
            .build(),
    );
    let video_fallback_queue = make("queue")?;
    video_fallback_queue.set_property("max-size-time", 0u64);
    video_fallback_queue.set_property("max-size-bytes", 0u32);
    video_fallback_queue.set_property("max-size-buffers", 0u32);

    // ---------------- VIDEO LIVE ENTRY ----------------
    let video_live_queue = make("queue")?;
    video_live_queue.set_property("max-size-time", 0u64);
    video_live_queue.set_property("max-size-bytes", 0u32);
    video_live_queue.set_property("max-size-buffers", 0u32);

    let video_live_convert = make("videoconvert")?;
    let video_live_rate = make("videorate")?;
    let video_live_capsfilter = make("capsfilter")?;
    video_live_capsfilter.set_property(
        "caps",
        Caps::builder("video/x-raw")
            .field("framerate", gst::Fraction::new(30, 1))
            .build(),
    );

    // ---------------- VIDEO OUTPUT ----------------
    let video_selector = make("input-selector")?;
    let video_post_convert = make("videoconvert")?;
    let video_post_rate = make("videorate")?;
    let video_post_capsfilter = make("capsfilter")?;
    video_post_capsfilter.set_property(
        "caps",
        Caps::builder("video/x-raw")
            .field("framerate", gst::Fraction::new(30, 1))
            .build(),
    );

    // Use VP8 for portability. If you prefer x264enc and it exists, swap this out.
    let video_enc = make("vp8enc")?;
    video_enc.set_property("deadline", 1i64);
    video_enc.set_property("cpu-used", 8i32);

    let video_out_queue = make("queue")?;
    video_out_queue.set_property("max-size-time", 0u64);
    video_out_queue.set_property("max-size-bytes", 0u32);
    video_out_queue.set_property("max-size-buffers", 0u32);

    pipeline.add_many(&[
        &mux,
        &sink,
        &audio_fallback_src,
        &audio_fallback_convert,
        &audio_fallback_resample,
        &audio_fallback_capsfilter,
        &audio_fallback_queue,
        &audio_live_queue,
        &audio_live_convert,
        &audio_live_resample,
        &audio_live_capsfilter,
        &audio_selector,
        &audio_post_convert,
        &audio_post_resample,
        &audio_post_capsfilter,
        &audio_enc,
        &audio_out_queue,
        &video_fallback_src,
        &video_fallback_convert,
        &video_fallback_rate,
        &video_fallback_capsfilter,
        &video_fallback_queue,
        &video_live_queue,
        &video_live_convert,
        &video_live_rate,
        &video_live_capsfilter,
        &video_selector,
        &video_post_convert,
        &video_post_rate,
        &video_post_capsfilter,
        &video_enc,
        &video_out_queue,
    ])?;

    link_many(&[&mux, &sink])?;

    // ---------------- AUDIO FALLBACK -> SELECTOR ----------------
    link_many(&[
        &audio_fallback_src,
        &audio_fallback_convert,
        &audio_fallback_resample,
        &audio_fallback_queue,
    ])?;

    let audio_fallback_sink_pad = audio_selector
        .request_pad_simple("sink_%u")
        .ok_or_else(|| glib::bool_error!("failed to request audio fallback selector pad"))?;
    audio_fallback_queue
        .static_pad("src")
        .ok_or_else(|| glib::bool_error!("audio fallback queue has no src pad"))?
        .link(&audio_fallback_sink_pad)
        .map_err(|e| glib::bool_error!("failed to link audio fallback to selector: {e:?}"))?;

    // ---------------- AUDIO LIVE -> SELECTOR ----------------
    link_many(&[
        &audio_live_queue,
        &audio_live_convert,
        &audio_live_resample,
        &audio_live_capsfilter,
    ])?;

    let audio_live_sink_pad = audio_selector
        .request_pad_simple("sink_%u")
        .ok_or_else(|| glib::bool_error!("failed to request audio live selector pad"))?;
    audio_live_capsfilter
        .static_pad("src")
        .ok_or_else(|| glib::bool_error!("audio live resample has no src pad"))?
        .link(&audio_live_sink_pad)
        .map_err(|e| glib::bool_error!("failed to link audio live to selector: {e:?}"))?;

    // ---------------- AUDIO SELECTOR -> ENCODER -> MUX ----------------
    link_many(&[
        &audio_selector,
        &audio_post_convert,
        &audio_post_resample,
        &audio_post_capsfilter,
        &audio_enc,
        &audio_out_queue,
    ])?;

    let mux_audio_pad = mux
        .request_pad_simple("audio_%u")
        .ok_or_else(|| glib::bool_error!("failed to request mux audio pad"))?;
    audio_out_queue
        .static_pad("src")
        .ok_or_else(|| glib::bool_error!("audio_out_queue has no src pad"))?
        .link(&mux_audio_pad)
        .map_err(|e| glib::bool_error!("failed to link encoded audio to mux: {e:?}"))?;

    // ---------------- VIDEO FALLBACK -> SELECTOR ----------------
    link_many(&[
        &video_fallback_src,
        &video_fallback_convert,
        &video_fallback_rate,
        &video_fallback_capsfilter,
        &video_fallback_queue,
    ])?;

    let video_fallback_sink_pad = video_selector
        .request_pad_simple("sink_%u")
        .ok_or_else(|| glib::bool_error!("failed to request video fallback selector pad"))?;
    video_fallback_queue
        .static_pad("src")
        .ok_or_else(|| glib::bool_error!("video fallback queue has no src pad"))?
        .link(&video_fallback_sink_pad)
        .map_err(|e| glib::bool_error!("failed to link video fallback to selector: {e:?}"))?;

    // ---------------- VIDEO LIVE -> SELECTOR ----------------
    link_many(&[
        &video_live_queue,
        &video_live_convert,
        &video_live_rate,
        &video_live_capsfilter,
    ])?;

    let video_live_sink_pad = video_selector
        .request_pad_simple("sink_%u")
        .ok_or_else(|| glib::bool_error!("failed to request video live selector pad"))?;
    video_live_capsfilter
        .static_pad("src")
        .ok_or_else(|| glib::bool_error!("video live capsfilter has no src pad"))?
        .link(&video_live_sink_pad)
        .map_err(|e| glib::bool_error!("failed to link video live to selector: {e:?}"))?;

    // ---------------- VIDEO SELECTOR -> ENCODER -> MUX ----------------
    link_many(&[
        &video_selector,
        &video_post_convert,
        &video_post_rate,
        &video_post_capsfilter,
        &video_enc,
        &video_out_queue,
    ])?;

    let mux_video_pad = mux
        .request_pad_simple("video_%u")
        .ok_or_else(|| glib::bool_error!("failed to request mux video pad"))?;
    video_out_queue
        .static_pad("src")
        .ok_or_else(|| glib::bool_error!("video_out_queue has no src pad"))?
        .link(&mux_video_pad)
        .map_err(|e| glib::bool_error!("failed to link encoded video to mux: {e:?}"))?;

    sync_all(&[
        &mux,
        &sink,
        &audio_fallback_src,
        &audio_fallback_convert,
        &audio_fallback_resample,
        &audio_fallback_capsfilter,
        &audio_fallback_queue,
        &audio_live_queue,
        &audio_live_convert,
        &audio_live_resample,
        &audio_live_capsfilter,
        &audio_selector,
        &audio_post_convert,
        &audio_post_resample,
        &audio_post_capsfilter,
        &audio_enc,
        &audio_out_queue,
        &video_fallback_src,
        &video_fallback_convert,
        &video_fallback_rate,
        &video_fallback_capsfilter,
        &video_fallback_queue,
        &video_live_queue,
        &video_live_convert,
        &video_live_rate,
        &video_live_capsfilter,
        &video_selector,
        &video_post_convert,
        &video_post_rate,
        &video_post_capsfilter,
        &video_enc,
        &video_out_queue,
    ])?;

    // Start with fallback pads active.
    audio_selector.set_property("active-pad", &audio_fallback_sink_pad);
    video_selector.set_property("active-pad", &video_fallback_sink_pad);

    Ok(Arc::new(Mutex::new(RecordingHandles {
        audio_selector,
        audio_fallback_sink_pad,
        audio_live_sink_pad,
        audio_live_linked: false,
        video_selector,
        video_fallback_sink_pad,
        video_live_sink_pad,
        video_live_linked: false,
        audio_live_queue,
        video_live_queue,
    })))
}

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

    // Build both tracks up front so one file always has audio + video tracks.
    let recording = build_recording_graph(&pipeline, "recording.mkv")?;

    let recording_for_pad = Arc::clone(&recording);
    webrtc.connect_pad_added(move |webrtc, pad| {
        let caps = {
            let mut c = pad.current_caps();
            for _ in 0..10 {
                if c.is_some() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
                c = pad.current_caps();
            }
            match c {
                Some(c) => c,
                None => {
                    warn!("pad-added with no caps after retry");
                    return;
                }
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

        let mut rec = recording_for_pad.lock().unwrap();

        match (media, encoding) {
            ("audio", "OPUS") => {
                if rec.audio_live_linked {
                    info!("Audio live path already linked");
                    return;
                }

                let depay = match make("rtpopusdepay") {
                    Ok(e) => e,
                    Err(err) => {
                        warn!("Failed to create rtpopusdepay: {:?}", err);
                        return;
                    }
                };

                let dec = match make("opusdec") {
                    Ok(e) => e,
                    Err(err) => {
                        warn!("Failed to create opusdec: {:?}", err);
                        return;
                    }
                };

                if let Err(err) = pipeline.add_many(&[&depay, &dec]) {
                    warn!("Failed to add audio live elements: {:?}", err);
                    return;
                }

                if let Err(err) = sync_all(&[&depay, &dec]) {
                    warn!("Failed to sync audio live elements: {:?}", err);
                    return;
                }

                if let Err(err) = link_many(&[&depay, &dec]) {
                    warn!("Failed to link audio depay/dec: {:?}", err);
                    return;
                }

                let depay_sink = match depay.static_pad("sink") {
                    Some(p) => p,
                    None => {
                        warn!("rtpopusdepay has no sink pad");
                        return;
                    }
                };

                if let Err(err) = pad.link(&depay_sink) {
                    warn!("Failed to link webrtc audio pad: {:?}", err);
                    return;
                }

                let dec_src = match dec.static_pad("src") {
                    Some(p) => p,
                    None => {
                        warn!("opusdec has no src pad");
                        return;
                    }
                };

                let live_queue_sink = match rec.audio_live_queue.static_pad("sink") {
                    Some(p) => p,
                    None => {
                        warn!("audio_live_queue has no sink pad");
                        return;
                    }
                };

                if let Err(err) = dec_src.link(&live_queue_sink) {
                    warn!("Failed to link decoded audio to live queue: {:?}", err);
                    return;
                }

                rec.audio_selector
                    .set_property("active-pad", &rec.audio_live_sink_pad);
                rec.audio_live_linked = true;
                info!("Switched audio selector to live audio");
            }

            ("video", "AV1") => {
                if rec.video_live_linked {
                    info!("Video live path already linked");
                    return;
                }

                let depay = match make("rtpav1depay") {
                    Ok(e) => e,
                    Err(err) => {
                        error!("Failed to create rtpav1depay: {:?}", err);
                        return;
                    }
                };

                let decodebin = match make("decodebin") {
                    Ok(e) => e,
                    Err(err) => {
                        error!("Failed to create decodebin: {:?}", err);
                        return;
                    }
                };

                if let Err(err) = pipeline.add_many(&[&depay, &decodebin]) {
                    error!("Failed to add video live elements: {:?}", err);
                    return;
                }

                if let Err(err) = sync_all(&[&depay, &decodebin]) {
                    error!("Failed to sync video live elements: {:?}", err);
                    return;
                }

                let depay_sink = match depay.static_pad("sink") {
                    Some(p) => p,
                    None => {
                        error!("rtpav1depay has no sink pad");
                        return;
                    }
                };

                if let Err(err) = pad.link(&depay_sink) {
                    error!("Failed to link webrtc video pad to depay: {:?}", err);
                    return;
                }

                if let Err(err) = depay.link(&decodebin) {
                    error!("Failed to link rtpav1depay to decodebin: {:?}", err);
                    return;
                }

                let video_live_queue = rec.video_live_queue.clone();
                let video_live_sink_pad = rec.video_live_sink_pad.clone();
                let recording_for_decode = Arc::clone(&recording_for_pad);

                decodebin.connect_pad_added(move |_decodebin, src_pad| {
                    let caps = match src_pad.current_caps() {
                        Some(c) => c,
                        None => {
                            error!("decodebin video src pad has no caps");
                            return;
                        }
                    };

                    let s = match caps.structure(0) {
                        Some(s) => s,
                        None => {
                            error!("decodebin video caps have no structure");
                            return;
                        }
                    };

                    let media_type = s.name();

                    if media_type != "video/x-raw" {
                        info!("Ignoring non-raw decodebin pad with caps {}", caps);
                        return;
                    }

                    let live_queue_sink = match video_live_queue.static_pad("sink") {
                        Some(p) => p,
                        None => {
                            error!("video_live_queue has no sink pad");
                            return;
                        }
                    };

                    if live_queue_sink.is_linked() {
                        info!("video_live_queue sink already linked");
                        return;
                    }

                    if let Err(err) = src_pad.link(&live_queue_sink) {
                        error!(
                            "Failed to link decodebin src to video_live_queue: {:?}",
                            err
                        );
                        return;
                    }

                    let mut rec = recording_for_decode.lock().unwrap();
                    rec.video_selector
                        .set_property("active-pad", &video_live_sink_pad);
                    rec.video_live_linked = true;
                    info!("Switched video selector to live video");
                });
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
        for msg in bus.iter_timed(gst::ClockTime::NONE) {
            use gst::MessageView;
            match msg.view() {
                MessageView::Eos(..) => {
                    info!("EOS received, shutting down pipeline");
                    let _ = pipeline_clone.set_state(gst::State::Null);
                    break;
                }
                MessageView::Error(err) => {
                    error!("Pipeline error: {:?}", err);
                    let _ = pipeline_clone.set_state(gst::State::Null);
                    break;
                }
                MessageView::Warning(w) => {
                    warn!("Pipeline warning: {:?}", w);
                }
                _ => {}
            }
        }
    });

    // Start pipeline before remote description.
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
                    let jingle_stanza = to_jingle(&sdp_sess, jingle_ref.clone());

                    let mut ice_ufrag = String::new();
                    let mut ice_pwd = String::new();
                    for line in sdp.lines() {
                        if let Some(v) = line.strip_prefix("a=ice-ufrag:") {
                            ice_ufrag = v.trim().to_string();
                        }
                        if let Some(v) = line.strip_prefix("a=ice-pwd:") {
                            ice_pwd = v.trim().to_string();
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
                        Err(err) => warn!("jingle stanza error: {}", err),
                    }
                }
                Err(err) => error!("Error parsing SDP: {}", err),
            },
            Err(e) => error!("Failed to get SDP text: {e:?}"),
        }
    });

    let webrtc_for_answer = Arc::clone(&webrtc_ref);
    let set_description_promise = Promise::with_change_func(move |reply| match reply {
        Ok(_) => {
            info!("Remote description applied. Creating answer...");

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
                    info!("JVB DataChannel open. Sending constraints...");
                    let msg = r#"{"colibriClass":"ReceiverVideoConstraints","lastN":-1,"defaultConstraints":{"maxHeight":720}}"#;
                    match dc.send_string_full(Some(msg)) {
                        Ok(_) => info!("Constraints sent"),
                        Err(e) => error!("Failed to send constraints: {:?}", e),
                    }
                });

                dc.connect_on_error(|_dc, err| warn!("Channel error: {:?}", err));
                dc.connect_on_message_string(|_dc, data| {
                    if let Some(data) = data {
                        info!("data channel message: {:?}", data);
                    }
                });
            } else {
                error!("Failed to create data channel – is gst-plugins-bad installed?");
            }

            webrtc_for_answer
                .emit_by_name::<()>("create-answer", &[&None::<Structure>, &answer_promise]);
        }
        Err(e) => error!("Failed to set remote description: {:?}", e),
    });

    let webrtc_for_remote = Arc::clone(&webrtc_ref);
    webrtc_for_remote.emit_by_name::<()>(
        "set-remote-description",
        &[&sdp_offer, &set_description_promise],
    );
    info!("Remote description set");

    // ICE candidate forwarding
    let webrtc_for_ice = Arc::clone(&webrtc_ref);
    let iq_for_ice = Arc::clone(&iq);
    let jingle_for_ice = Arc::clone(&jingle);
    let tx_for_ice = tx.clone();
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

        let parsed = parse_attribute(candidate.as_str());
        if let Ok(SdpType::Attribute(SdpAttribute::Candidate(c))) = parsed {
            let mut candidate_stanza = Stanza::new();
            if parse_candidate(&mut candidate_stanza, &c).is_ok() {
                let mut iq_stanza = Stanza::new_iq(Some("set"), Some(nanoid!().as_str()));
                iq_stanza
                    .set_attribute("to", iq_for_ice.from.as_str())
                    .unwrap();
                iq_stanza
                    .set_attribute("from", iq_for_ice.to.as_str())
                    .unwrap();

                let mut jingle_stanza = Stanza::new();
                jingle_stanza.set_name("jingle").unwrap();
                jingle_stanza.set_ns("urn:xmpp:jingle:1").unwrap();
                jingle_stanza
                    .set_attribute("action", "transport-info")
                    .unwrap();
                jingle_stanza
                    .set_attribute("initiator", jingle_for_ice.get_initiator())
                    .unwrap();
                jingle_stanza
                    .set_attribute("sid", jingle_for_ice.get_sid())
                    .unwrap();
                jingle_stanza
                    .set_attribute("responder", jingle_for_ice.get_responder())
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

                tx_for_ice
                    .send(iq_stanza)
                    .expect("couldn't send transport-info");
            }
        }

        None
    });

    info!("Pipeline started");
    Ok(())
}

