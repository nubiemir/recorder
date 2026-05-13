use std::{
    process::exit,
    sync::{Arc, Weak, mpsc::Sender},
};

use gstreamer::{
    Element, ElementFactory, Pad, Pipeline, State,
    glib::{BoolError, Value, object::ObjectExt},
    prelude::{ElementExt, ElementExtManual, GObjectExtManualGst},
};
use libstrophe::Stanza;
use log::{error, info};
use nanoid::nanoid;
use webrtc_sdp::{
    SdpSession, SdpType,
    attribute_type::{SdpAttribute, parse_attribute},
};

use crate::{config::Webrtc, get_attribute, iq::Iq, make_stanza, sdp::Sdp, upgrade_weak, xep::XEP};

#[derive(Debug)]
#[allow(unused)]
pub struct RoomInner {
    iq: Iq,
    name: String,
    webrtcbin: Element,
    pipeline: Pipeline,
    tx: Sender<Stanza>,
}

#[derive(Debug)]
pub struct RoomWeak(Weak<RoomInner>);

#[derive(Debug)]
pub struct Room(Arc<RoomInner>);

impl std::ops::Deref for Room {
    type Target = RoomInner;

    fn deref(&self) -> &RoomInner {
        &self.0
    }
}

impl Drop for RoomInner {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(State::Null);
    }
}

impl RoomWeak {
    fn upgrade(&self) -> Option<Room> {
        self.0.upgrade().map(Room)
    }
}

impl Room {
    pub fn downgrade(&self) -> RoomWeak {
        RoomWeak(Arc::downgrade(&self.0))
    }

    pub fn new(
        name: String,
        tx: Sender<Stanza>,
        webrtc: &Webrtc,
        iq: Iq,
        stanza: &Stanza,
    ) -> Result<Self, BoolError> {
        let pipeline = Pipeline::new();
        let webrtcbin = ElementFactory::make("webrtcbin").build()?;

        webrtcbin.set_property_from_str("stun-server", &webrtc.stun_server);
        webrtcbin.set_property_from_str("bundle-policy", &webrtc.bundle_policy);

        pipeline.call_async(|pipeline| match pipeline.set_state(State::Playing) {
            Err(err) => {
                error!("failed to state change to playing: {err:?}");
                exit(1);
            }
            Ok(_) => {
                info!("successfully change state to playing");
            }
        });

        let room = Room(Arc::new(RoomInner {
            name,
            webrtcbin,
            pipeline,
            iq,
            tx,
        }));

        let jingle_stanza = get_attribute!(stanza, [sid, initiator, action]);

        let room_clone = room.downgrade();
        room.webrtcbin
            .connect("on-ice-candidate", false, move |values| {
                let room = upgrade_weak!(room_clone, None);
                room.on_ice_candidate(values, &jingle_stanza.sid, &jingle_stanza.initiator)
            });

        let room_clone = room.downgrade();
        room.webrtcbin.connect_pad_added(move |_webrtc, pad| {
            let room = upgrade_weak!(room_clone);
            room.on_incoming_stream(pad);
        });

        room.pipeline.call_async(|pipeline| {
            let _ = pipeline.set_state(State::Playing);
        });

        Ok(room)
    }

    pub fn get_name(&self) -> &str {
        &self.name
    }

    pub fn get_webrtcbin(&self) -> &Element {
        &self.webrtcbin
    }

    pub fn get_pipeline(&self) -> &Pipeline {
        &self.pipeline
    }

    fn on_ice_candidate(&self, values: &[Value], sid: &str, initiator: &str) -> Option<Value> {
        let mline_index = values[1].get::<u32>().ok()?;
        let candidate = values[2].get::<String>().ok()?;

        if candidate.is_empty() {
            return None;
        }

        let content_name = match mline_index {
            0 => "audio",
            1 => "video",
            2 => "data",
            _ => {
                error!("Unknown mline index: {}", mline_index);
                return None;
            }
        };

        let parsed_candidate = parse_attribute(&candidate).ok()?;

        if let SdpType::Attribute(SdpAttribute::Candidate(c)) = parsed_candidate {
            let candidate_stanza = self.iq.parse_candidate(&c).ok()?;

            let transport = make_stanza!("transport", {
                "xmlns" => XEP::IceUdpTransport.to_string(),
                "ufrag" => "",
                "pwd" => ""
            }, [candidate_stanza])
            .ok()?;

            let content = make_stanza!("content", {
                "name" => content_name,
                "senders" => "initiator",
                "creator" => "initiator"
            }, [transport])
            .ok()?;

            let jingle = make_stanza!("jingle", {
                "xmlns" => "urn:xmpp:jingle:1",
                "action" => "transport-info",
                "initiator" => initiator,
                "sid" => sid,
                "responder" => &self.iq.to
            }, [content])
            .ok()?;

            let iq = make_stanza!("iq", {
                "id" => nanoid!(),
                "to" => &self.iq.from,
                "from" => &self.iq.to,
            }, [jingle])
            .ok()?;

            match self.tx.send(iq) {
                Ok(_) => {
                    info!("successfully sent candidate");
                }
                Err(_) => {
                    error!("failed to send candidate for: {} room", &self.name);
                }
            }
        }

        None
    }

    fn on_incoming_stream(&self, _pad: &Pad) {
        info!("new pad added for: {}", &self.name);
    }

    pub fn handle_session_initiate(
        &self,
        responder: &str,
        stanza: &Stanza,
        sdp_session: SdpSession,
    ) {
        let sdp = Sdp::new(&sdp_session);
        let jingle_stanza = get_attribute!(stanza, [sid, initiator, action]);

        let _answer =
            sdp.parse_sdp_to_jingle(&jingle_stanza.initiator, &jingle_stanza.sid, responder);
    }
}
