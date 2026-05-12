use std::{
    process::exit,
    sync::{Arc, Weak, mpsc::Sender},
};

use gstreamer::{
    Element, ElementFactory, Pipeline,
    glib::{BoolError, Value, object::ObjectExt},
    prelude::{ElementExt, ElementExtManual, GObjectExtManualGst},
};
use libstrophe::Stanza;
use log::{error, info};
use webrtc_sdp::{
    SdpSession, SdpType,
    attribute_type::{SdpAttribute, parse_attribute},
};

use crate::{config::Webrtc, get_attribute, iq::Iq, sdp::Sdp, upgrade_weak};

#[derive(Debug)]
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

impl RoomWeak {
    fn upgrade(&self) -> Option<Room> {
        self.0.upgrade().map(Room)
    }
}

impl Room {
    fn downgrade(&self) -> RoomWeak {
        RoomWeak(Arc::downgrade(&self.0))
    }

    pub fn new(
        name: String,
        tx: Sender<Stanza>,
        webrtc: &Webrtc,
        iq: Iq,
    ) -> Result<Self, BoolError> {
        let pipeline = Pipeline::new();
        let webrtcbin = ElementFactory::make("webrtcbin").build()?;

        webrtcbin.set_property_from_str("stun-server", &webrtc.stun_server);
        webrtcbin.set_property_from_str("bundle-policy", &webrtc.bundle_policy);

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

        let room = Room(Arc::new(RoomInner {
            name,
            webrtcbin,
            pipeline,
            iq,
            tx,
        }));

        let room_clone = room.downgrade();

        room.webrtcbin
            .connect("on-ice-candidate", false, move |values| {
                let room = upgrade_weak!(room_clone, None);
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
        }

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
