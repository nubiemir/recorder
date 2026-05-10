use std::sync::mpsc::Sender;

use gstreamer::{Element, ElementFactory, glib::BoolError, prelude::GObjectExtManualGst};
use libstrophe::Stanza;
use webrtc_sdp::SdpSession;

use crate::{config::Webrtc, get_attribute, sdp::Sdp};

#[derive(Debug)]
pub struct Room {
    name: String,
    webrtcbin: Element,
    tx: Sender<Stanza>,
}

impl Room {
    pub fn new(
        name: String,
        tx: Sender<Stanza>,
        webrtc_config: &Webrtc,
    ) -> Result<Self, BoolError> {
        let webrtcbin = ElementFactory::make("webrtcbin").build()?;

        webrtcbin.set_property_from_str("stun-server", &webrtc_config.stun_server);
        webrtcbin.set_property_from_str("bundle-policy", &webrtc_config.bundle_policy);

        let room = Room {
            name,
            webrtcbin,
            tx,
        };

        Ok(room)
    }

    pub fn get_name(&self) -> &str {
        &self.name
    }

    pub fn get_webrtcbin(&self) -> &Element {
        &self.webrtcbin
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
