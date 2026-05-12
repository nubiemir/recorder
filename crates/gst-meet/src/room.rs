use std::{process::exit, sync::mpsc::Sender};

use gstreamer::{
    Element, ElementFactory, Pipeline,
    glib::{BoolError, Value, object::ObjectExt},
    prelude::{ElementExt, ElementExtManual, GObjectExtManualGst},
};
use libstrophe::Stanza;
use log::{error, info};
use webrtc_sdp::SdpSession;

use crate::{config::Webrtc, get_attribute, sdp::Sdp};

#[derive(Debug)]
pub struct Room {
    name: String,
    webrtcbin: Element,
    pipeline: Pipeline,
    tx: Sender<Stanza>,
}

impl Room {
    pub fn new(
        name: String,
        tx: Sender<Stanza>,
        webrtc_config: &Webrtc,
    ) -> Result<Self, BoolError> {
        let pipeline = Pipeline::new();
        let webrtcbin = ElementFactory::make("webrtcbin").build()?;

        webrtcbin.set_property_from_str("stun-server", &webrtc_config.stun_server);
        webrtcbin.set_property_from_str("bundle-policy", &webrtc_config.bundle_policy);

        pipeline.call_async(
            |pipeline| match pipeline.set_state(gstreamer::State::Playing) {
                Err(err) => {
                    error!("failed to state change to playing: {err:?}");
                    exit(1);
                }
                Ok(_) => {
                    info!("successfully change state to playing");
                }
            },
        );

        let room = Room {
            name,
            webrtcbin,
            pipeline,
            tx,
        };

        room.webrtcbin
            .connect("on-ice-candidate", false, move |values| {
                room.on_ice_candidate(values)
            });

        Ok(room)
    }

    pub fn get_name(&self) -> &str {
        &self.name
    }

    pub fn get_webrtcbin(&self) -> &Element {
        &self.webrtcbin
    }

    fn on_ice_candidate(&self, values: &[Value]) -> Option<Value> {
        None
    }

    pub fn handle_session_initiate(
        &self,
        responder: &str,
        stanza: &Stanza,
        sdp_session: SdpSession,
    ) {
        let sdp = Sdp::new(&sdp_session);
        let jingle_stanza = get_attribute!(stanza, [sid, initiator, action]);

        let answer =
            sdp.parse_sdp_to_jingle(&jingle_stanza.initiator, &jingle_stanza.sid, responder);
    }
}
