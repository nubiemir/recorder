use std::{
    process::exit,
    sync::{Arc, OnceLock, Weak, mpsc::Sender},
};

use gstreamer::{
    Element, ElementFactory, Pad, PadDirection, PadLinkError, Pipeline, Promise, PromiseError,
    State, StateChangeError, Structure, StructureRef,
    glib::{BoolError, Value, object::ObjectExt},
    prelude::{ElementExt, ElementExtManual, GObjectExtManualGst, GstBinExtManual, PadExt},
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

use crate::{
    config::Webrtc,
    get_attribute,
    iq::Iq,
    make_stanza,
    render::{renderer_engine::RendererEngine, renderer_handle::RendererHandle},
    sdp::Sdp,
    upgrade_weak,
    xep::XEP,
};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum IncomingStreamError {
    #[error("missing caps")]
    MissingCaps,

    #[error("missing caps structure")]
    MissingStructure,

    #[error("failed to get queue sink pad")]
    MissingQueueSinkPad,

    #[error("failed to get compositor sink pad")]
    MissingCompositorSinkPad,

    #[error("glib error: {0}")]
    Bool(#[from] BoolError),

    #[error("pad link error: {0}")]
    PadLink(#[from] PadLinkError),

    #[error("state change error: {0}")]
    StateChange(#[from] StateChangeError),
}

#[derive(Debug)]
#[allow(unused)]
pub struct RoomInner {
    name: String,
    webrtcbin: Element,
    pipeline: Pipeline,
    muxer: Element,
    compositor: Element,
    filesink: Element,
    tx: Sender<Stanza>,
    ufrag: OnceLock<String>,
    pwd: OnceLock<String>,
    renderer_handler: RendererHandle,
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

    pub fn new(name: String, tx: Sender<Stanza>, webrtc: &Webrtc) -> Result<Self, BoolError> {
        let pipeline = Pipeline::new();
        let webrtcbin = ElementFactory::make("webrtcbin").build()?;
        let muxer = ElementFactory::make("matroskamux").build()?;
        let filesink = ElementFactory::make("filesink").build()?;

        let output_location = format!("{}.mkv", name);

        filesink.set_property("location", output_location);

        webrtcbin.set_property_from_str("stun-server", &webrtc.stun_server);
        webrtcbin.set_property_from_str("bundle-policy", &webrtc.bundle_policy);

        let compositor = ElementFactory::make("compositor").build()?;

        let videoconvert = ElementFactory::make("videoconvert").build()?;

        let encoder = ElementFactory::make("x264enc")
            .property_from_str("tune", "zerolatency")
            .build()?;

        pipeline.add_many([
            &webrtcbin,
            &compositor,
            &videoconvert,
            &encoder,
            &muxer,
            &filesink,
        ])?;

        Element::link_many([&compositor, &videoconvert, &encoder, &muxer, &filesink])?;

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
            pipeline: pipeline.clone(),
            tx,
            filesink,
            muxer,
            compositor: compositor.clone(),
            ufrag: OnceLock::new(),
            pwd: OnceLock::new(),
            renderer_handler: RendererEngine::spawn(pipeline, compositor),
        }));

        let room_clone = room.downgrade();
        room.webrtcbin.connect_pad_added(move |_webrtc, pad| {
            let room = upgrade_weak!(room_clone);
            let _ = room.on_incoming_stream(pad);
        });

        Ok(room)
    }

    pub fn handle_ice_candidate(&mut self, from: &str, to: &str, sid: &str, initiator: &str) {
        let room_clone = self.downgrade();

        let sid = sid.to_string();
        let initiator = initiator.to_string();
        let from = from.to_string();
        let to = to.to_string();

        self.webrtcbin
            .connect("on-ice-candidate", false, move |values| {
                let room = upgrade_weak!(room_clone, None);

                room.on_ice_candidate(
                    values,
                    from.as_str(),
                    to.as_str(),
                    sid.as_str(),
                    initiator.as_str(),
                )
            });
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

    fn on_ice_candidate(
        &self,
        values: &[Value],
        from: &str,
        to: &str,
        sid: &str,
        initiator: &str,
    ) -> Option<Value> {
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
            let candidate_stanza = Iq::parse_candidate(&c).ok()?;

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
                "responder" => to
            }, [content])
            .ok()?;

            let iq = make_stanza!("iq", {
                "id" => nanoid!(),
                "to" => from,
                "from" => to,
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

            let colibri_message = r#"{"colibriClass":"ReceiverVideoConstraints","lastN":-1,"defaultConstraints":{"maxHeight":720}}"#;


            match data_channel.send_string_full(Some(colibri_message)) {
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

    pub fn on_incoming_stream(&self, pad: &Pad) -> Result<(), IncomingStreamError> {
        if pad.direction() != PadDirection::Src {
            return Ok(());
        }

        let caps = pad
            .current_caps()
            .or_else(|| Some(pad.query_caps(None)))
            .ok_or(IncomingStreamError::MissingCaps)?;

        let structure = caps
            .structure(0)
            .ok_or(IncomingStreamError::MissingStructure)?;

        if !structure.name().starts_with("application/x-rtp") {
            return Ok(());
        }

        let encoding_name = structure.get::<String>("encoding-name").unwrap_or_default();

        if encoding_name != "AV1" {
            info!("skipping non-AV1 stream: {}", encoding_name);
            return Ok(());
        }

        let ssrc = structure.get::<u32>("ssrc").unwrap_or(0);

        let endpoint_id = match self.renderer_handler.endpoint_for_ssrc(ssrc) {
            Some(id) => id,
            None => {
                info!("no endpoint for ssrc {}, ignoring", ssrc);
                return Ok(());
            }
        };

        let is_screenshare = self.renderer_handler.is_screenshare_ssrc(ssrc);

        let compositor_sink_pad = match self
            .renderer_handler
            .video_stream_arrived(&endpoint_id, is_screenshare)
        {
            Some(pad) => pad,
            None => {
                error!(
                    "renderer could not provide compositor pad for {}",
                    endpoint_id
                );
                return Ok(());
            }
        };

        let queue = ElementFactory::make("queue").build()?;
        let depay = ElementFactory::make("rtpav1depay").build()?;
        let parser = ElementFactory::make("av1parse").build()?;
        let decoder = ElementFactory::make("dav1ddec").build()?;
        let convert = ElementFactory::make("videoconvert").build()?;

        self.pipeline
            .add_many([&queue, &depay, &parser, &decoder, &convert])?;
        Element::link_many([&queue, &depay, &parser, &decoder, &convert])?;

        let queue_sink_pad = queue
            .static_pad("sink")
            .ok_or(IncomingStreamError::MissingQueueSinkPad)?;

        pad.link(&queue_sink_pad)?;

        convert
            .static_pad("src")
            .unwrap()
            .link(&compositor_sink_pad)?;

        queue.sync_state_with_parent()?;
        depay.sync_state_with_parent()?;
        parser.sync_state_with_parent()?;
        decoder.sync_state_with_parent()?;
        convert.sync_state_with_parent()?;

        info!(
            "stream linked for endpoint={} screenshare={}",
            endpoint_id, is_screenshare
        );

        Ok(())
    }
    fn on_answer_created(
        &self,
        from: String,
        to: String,
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

        self.webrtcbin
            .emit_by_name::<()>("set-local-description", &[&answer, &None::<Promise>]);

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

                        let jingle = sdp.parse_sdp_to_jingle(initiator, sid, &to)?;

                        let iq = make_stanza!("iq", {
                            "id" => nanoid!(),
                            "to" => &from,
                            "from" => &to,
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

    pub fn handle_session_initiate(
        &self,
        stanza: &Stanza,
        from: &str,
        to: &str,
        sdp_message: SDPMessage,
    ) {
        let room_clone = self.downgrade();
        let jingle = get_attribute!(stanza, [sid, initiator]);
        let from = from.to_string();
        let to = to.to_string();
        self.pipeline.call_async(move |_pipeline| {
            let room = upgrade_weak!(room_clone);
            let sdp_offer =
                WebRTCSessionDescription::new(gstreamer_webrtc::WebRTCSDPType::Offer, sdp_message);

            room.webrtcbin
                .emit_by_name::<()>("set-remote-description", &[&sdp_offer, &None::<Promise>]);

            let room_clone = room.downgrade();
            let promise = Promise::with_change_func(move |reply| {
                let room = upgrade_weak!(room_clone);
                room.on_answer_created(from, to, &jingle.sid, &jingle.initiator, reply);
            });

            room.webrtcbin
                .emit_by_name::<()>("create-answer", &[&None::<Structure>, &promise]);
        });
    }

    pub fn on_participant_joined(
        &mut self,
        endpoint_id: &str,
        nickname: &str,
        video_muted: bool,
        audio_muted: bool,
    ) {
        self.renderer_handler
            .participant_joined(endpoint_id, nickname, video_muted, audio_muted);
    }

    pub fn source_info_updated(
        &mut self,
        endpoint_id: &str,
        video_muted: bool,
        audio_muted: bool,
        has_screenshare: bool,
    ) {
        self.renderer_handler.source_info_updated(
            endpoint_id,
            video_muted,
            audio_muted,
            has_screenshare,
        );
    }

    pub fn on_participant_left(&mut self, endpoint_id: &str) {
        self.renderer_handler.participant_left(endpoint_id);
    }

    pub fn on_meeting_terminated(&self) {
        if !self.pipeline.send_event(gstreamer::event::Eos::new()) {
            error!("failed to send EOS for room: {}", self.name);
        }

        if let Some(bus) = self.pipeline.bus() {
            let _ = bus.timed_pop_filtered(
                gstreamer::ClockTime::from_seconds(5),
                &[gstreamer::MessageType::Eos, gstreamer::MessageType::Error],
            );
        }

        let _ = self.pipeline.set_state(State::Null);
        info!("pipeline stopped, file finalized for room: {}", self.name);
    }

    pub fn handle_register_ssrc(&mut self, ssrc: u32, endpoint_id: &str, source_name: &str) {
        self.renderer_handler
            .register_ssrc(ssrc, endpoint_id, source_name);
    }
}
