use std::sync::{Arc, mpsc::Sender};

use crate::{
    jingle::{Iq, Jingle},
    sdp::to_jingle,
};
use gstreamer::{
    ElementFactory, Pad, Pipeline, Promise, State, Structure, glib::BoolError, prelude::*,
};
use gstreamer_sdp::SDPMessage;
use gstreamer_webrtc::WebRTCSessionDescription;
use libstrophe::Stanza;
use log::{error, info, warn};
use nanoid::nanoid;
use webrtc_sdp::parse_sdp;

pub fn webrtcbin(offer: &str, jingle: Jingle, iq: Iq, tx: Sender<Stanza>) -> Result<(), BoolError> {
    let pipeline = Pipeline::new();

    let webrtc = ElementFactory::make("webrtcbin").build()?;

    pipeline.add(&webrtc)?;

    webrtc.set_property_from_str("bundle-policy", "max-bundle");

    webrtc.connect("pad-added", false, |args| {
        let pad = args[1].get::<Pad>().unwrap();
        let caps = pad.current_caps().unwrap();
        let media = caps
            .structure(0)
            .unwrap()
            .get::<&str>("media")
            .unwrap_or("unknown");

        info!("New {media} pad — caps: {caps}");
        // Here you would link to fakesink / decoder / filesink etc.
        None
    });

    pipeline.call_async(|pipeline| {
        pipeline
            .set_state(State::Playing)
            .expect("Couldn't set pipeline to Playing");
    });

    let sdp_message = SDPMessage::parse_buffer(offer.as_bytes())?;
    let sdp_offer =
        WebRTCSessionDescription::new(gstreamer_webrtc::WebRTCSDPType::Offer, sdp_message);

    webrtc.emit_by_name::<()>("set-remote-description", &[&sdp_offer, &None::<Promise>]);
    info!("Remote description set");

    let webrtc_ref = Arc::new(webrtc);
    let webrtc_for_answer = Arc::clone(&webrtc_ref);

    let answer_promise = Promise::with_change_func(move |reply| {
        let reply = reply.unwrap_or_else(|err| {
            error!("Answer creation future got no reponse: {:?}", err);
            None
        });

        let answer = reply.unwrap();

        let answer = answer
            .value("answer")
            .unwrap()
            .get::<WebRTCSessionDescription>()
            .expect("Invalid argument");

        webrtc_for_answer.emit_by_name::<()>("set-local-description", &[&answer, &None::<Promise>]);

        match answer.sdp().as_text() {
            Ok(sdp) => match parse_sdp(sdp.as_str(), true) {
                Ok(sdp_sess) => {
                    let jingle_stanza = to_jingle(&sdp_sess, jingle);

                    match jingle_stanza {
                        Ok(stanza) => {
                            let id = nanoid!();
                            info!("id: {}", id);
                            let mut iq_stanza = Stanza::new_iq(Some("set"), Some(id.as_str()));
                            iq_stanza.set_attribute("from", iq.to).unwrap();
                            iq_stanza.set_attribute("to", iq.from).unwrap();
                            iq_stanza.add_child(stanza).unwrap();
                            tx.send(iq_stanza).expect("Failed to send stanza");
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

    let webrtc_for_ice = Arc::clone(&webrtc_ref);
    let _webrtc_for_candidate = Arc::clone(&webrtc_ref);

    webrtc_for_ice.connect("on-ice-candidate", false, move |values| {
        let _mlineindex = values[1].get::<u32>().unwrap();
        let _candidate = values[2].get::<String>().unwrap();
        None
    });

    let webrtc_for_answer = Arc::clone(&webrtc_ref);

    webrtc_for_answer.emit_by_name::<()>("create-answer", &[&None::<Structure>, &answer_promise]);

    info!("Pipeline started");

    Ok(())
}
