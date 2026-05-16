use std::{
    process::exit,
    sync::{Arc, OnceLock, Weak, mpsc::Sender},
};

use gstreamer::{
    Element, ElementFactory, Pad, Pipeline, Promise, PromiseError, State, Structure, StructureRef,
    glib::{BoolError, Value, object::ObjectExt},
    prelude::{ElementExt, ElementExtManual, GObjectExtManualGst, GstBinExt},
};
use gstreamer_sdp::SDPMessage;
use gstreamer_webrtc::{WebRTCDataChannel, WebRTCSessionDescription};
use libstrophe::Stanza;
use log::{error, info};
use nanoid::nanoid;
use webrtc_sdp::{
    SdpType,
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

#[derive(Debug, Clone)]
pub struct Room(Arc<RoomInner>);

impl std::ops::Deref for Room {
    type Target = RoomInner;

    fn deref(&self) -> &RoomInner {
        &self.0
    }
}

impl std::fmt::Display for Room {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)
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

        pipeline.add(&webrtcbin)?;

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
                "type" => "set",
            }, [jingle])
            .ok()?;

            match self.tx.send(iq) {
                Ok(_) => {
                    info!("successfully sent candidate for: {} room", &self.name);
                }
                Err(_) => {
                    error!("failed to send candidate for: {} room", &self.name);
                }
            }
        }

        None
    }

    fn on_data_channel(&self, dc: WebRTCDataChannel) {
        let room_name = self.name.clone();
        dc.connect_on_open(move |data_channel| {
            info!(
                "JVB confirmed DataChannel open for:{} room. Sending constraints...",
                room_name
            );

            let colibri_json = serde_json::json!({
                "colibriClass": "ReceiverVideoConstraints",
                "lastN": -1,
                "defaultConstraints": {
                    "maxHeight": 720
                }
            });

            let colibri_message = colibri_json.as_str();

            match data_channel.send_string_full(colibri_message) {
                Ok(_) => info!(
                    "colibri constraints sent successfully for: {} room",
                    room_name
                ),
                Err(err) => error!(
                    "failed to send colibri message for: {} room, err: {:?}",
                    room_name, err
                ),
            }
        });

        let room_name = self.name.clone();
        dc.connect_on_message_string(move |_dc, data| match data {
            Some(data) => {
                info!(
                    "data channel on message for: {} room, data: {:?}",
                    room_name, data
                );
            }
            None => {}
        });

        let room_name = self.name.clone();
        dc.connect_on_error(move |_dc, err| {
            error!("data channel error for: {} room, err: {:?}", room_name, err);
        });
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

        let answer = match reply
            .value("answer")
            .ok()
            .and_then(|v| v.get::<WebRTCSessionDescription>().ok())
        {
            Some(desc) => desc,
            None => {
                return error!(
                    "field answer was missing or wrong type for room:{}",
                    &self.name
                );
            }
        };

        self.webrtcbin
            .emit_by_name::<()>("set-local-description", &[&answer, &None::<Promise>]);

        match self.webrtcbin.emit_by_name::<Option<WebRTCDataChannel>>(
            "create-data-channel",
            &[
                &"JVB data channel",
                &Structure::builder("config")
                    .field("protocol", "http://jitsi.org/protocols/colibri")
                    .build(),
            ],
        ) {
            Some(dc) => {
                self.on_data_channel(dc);
            }
            None => {
                error!(
                    "failed to create data channel object. Check if gst-plugins-bad is installed for: {} room",
                    &self.name
                );
            }
        }

        match answer.sdp().as_text() {
            Ok(sdp_answer) => match parse_sdp(&sdp_answer, true) {
                Ok(sdp) => {
                    let sdp = Sdp::new(&sdp);

                    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
                        for line in sdp_answer.lines() {
                            if let Some(v) = line.strip_prefix("a=ice-ufrag:") {
                                if self.ufrag.get().is_none() {
                                    self.ufrag.set(v.to_string())?;
                                }
                            }
                            if let Some(v) = line.strip_prefix("a=ice-pwd:") {
                                if self.pwd.get().is_none() {
                                    self.pwd.set(v.to_string())?;
                                }
                            }
                        }

                        let jingle = sdp.parse_sdp_to_jingle(initiator, sid, &self.iq.from)?;

                        let iq = make_stanza!("iq", {
                            "id" => nanoid!(),
                            "to" => &self.iq.from,
                            "from" => &self.iq.to,
                            "type" => "set"
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

    pub fn handle_session_initiate(&self, stanza: &Stanza, sdp_message: SDPMessage) {
        let room_clone = self.downgrade();
        let jingle = get_attribute!(stanza, [sid, initiator]);
        self.pipeline.call_async(move |_pipeline| {
            let room = upgrade_weak!(room_clone);
            let sdp_offer =
                WebRTCSessionDescription::new(gstreamer_webrtc::WebRTCSDPType::Offer, sdp_message);

            room.webrtcbin
                .emit_by_name::<()>("set-remote-description", &[&sdp_offer, &None::<Promise>]);

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
