use std::{
    process::exit,
    sync::{Arc, OnceLock, Weak, mpsc::Sender},
};

use gstreamer::{
    Element, ElementFactory, Pad, Pipeline, Promise, PromiseError, State, Structure, StructureRef,
    glib::{BoolError, Value, object::ObjectExt},
    prelude::{ElementExt, ElementExtManual, GObjectExtManualGst},
};
use gstreamer_webrtc::WebRTCSessionDescription;
use libstrophe::Stanza;
use log::{error, info};
use nanoid::nanoid;
use webrtc_sdp::{
    SdpSession, SdpType,
    attribute_type::{SdpAttribute, parse_attribute},
    parse_sdp,
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
    ufrag: OnceLock<String>,
    pwd: OnceLock<String>,
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

        let room_name_clone = name.clone();
        pipeline.call_async(move |pipeline| match pipeline.set_state(State::Playing) {
            Err(err) => {
                error!(
                    "failed to state change to playing: {err:?} for room:{}",
                    room_name_clone
                );
                exit(1);
            }
            Ok(_) => {
                info!(
                    "successfully change state to playing for room:{}",
                    room_name_clone
                );
            }
        });

        let room = Room(Arc::new(RoomInner {
            name,
            webrtcbin,
            pipeline,
            iq,
            tx,
            ufrag: OnceLock::new(),
            pwd: OnceLock::new(),
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
                "ufrag" => self.ufrag.get().map(|s| s.as_str()).unwrap_or(""),
                "pwd" =>self.pwd.get().map(|s| s.as_str()).unwrap_or("") 
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

    fn on_answer_created(
        &self,
        sid: &str,
        initiator: &str,
        reply: Result<Option<&StructureRef>, PromiseError>,
    ) {
        let reply = match reply {
            Ok(Some(reply)) => reply,
            Ok(None) => {
                return error!("promise replied with no structure room:{}", &self.name);
            }
            Err(err) => {
                return error!("failed to get a reply: {err:?} for room:{}", &self.name);
            }
        };

        let answer = match reply.get::<WebRTCSessionDescription>("answer") {
            Ok(desc) => desc,
            Err(e) => {
                return error!(
                    "field answer was missing or wrong type: {:?} for room:{}",
                    e, &self.name
                );
            }
        };

        self.webrtcbin
            .emit_by_name::<()>("set-local-description", &[&answer, &None::<Promise>]);

        match answer.sdp().as_text() {
            Ok(sdp_answer) => match parse_sdp(&sdp_answer, true) {
                Ok(sdp) => {
                    let sdp = Sdp::new(&sdp);

                    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
                        for line in sdp_answer.lines() {
                            if let Some(v) = line.strip_prefix("a=ice-ufrag:") {
                                self.ufrag.set(v.to_string())?;
                            }
                            if let Some(v) = line.strip_prefix("a=ice-pwd:") {
                                self.pwd.set(v.to_string())?;
                            }
                        }
                        let jingle = sdp.parse_sdp_to_jingle(initiator, sid, &self.iq.from)?;
                        let iq = make_stanza!("iq", {
                            "id" => nanoid!(),
                            "to" => &self.iq.from,
                            "from" => &self.iq.to,
                        }, [jingle])?;
                        self.tx.send(iq)?;
                        Ok(())
                    })();

                    match result {
                        Ok(_) => info!(
                            "successfully sent iq for session accept room: {}",
                            &self.name
                        ),
                        Err(err) => error!(
                            "failed to send session accept iq for room {}: {err:?}",
                            &self.name
                        ),
                    }
                }
                Err(err) => {
                    error!(
                        "failed to parse to sdp session: {err:?} for room:{}",
                        &self.name
                    );
                }
            },
            Err(err) => {
                error!(
                    "failed to parse sdp answer: {err:?} for room:{}",
                    &self.name
                );
            }
        }
    }

    pub fn handle_session_initiate(&self, stanza: &Stanza, sdp_offer: SdpSession) {
        let room_clone = self.downgrade();
        let jingle = get_attribute!(stanza, [sid, initiator]);
        self.pipeline.call_async(move |_pipeline| {
            let room = upgrade_weak!(room_clone);
            let offer_string = sdp_offer.to_string();

            room.webrtcbin
                .emit_by_name::<()>("set-remote-description", &[&offer_string, &None::<Promise>]);

            let room_clone = room.downgrade();
            let promise = Promise::with_change_func(move |reply| {
                let room = upgrade_weak!(room_clone);
                room.on_answer_created(&jingle.sid, &jingle.initiator, reply);
            });

            room.webrtcbin
                .emit_by_name::<()>("create-answer", &[&None::<Structure>, &promise]);
        });
    }
}
