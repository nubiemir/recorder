use crate::{
    config::Webrtc,
    get_attribute,
    iq::{Iq, jingle_action::ParsedSource},
    make_stanza,
    sdp::Sdp,
    timeline::{timeline_engine::TimelineEngine, timeline_handler::TimelineHandler},
    upgrade_weak,
    xep::XEP,
};
use gstreamer::{
    Element, ElementFactory, Pad, PadDirection, PadLinkError, Pipeline, Promise, PromiseError,
    State, StateChangeError, Structure, StructureRef,
    glib::{BoolError, MainLoop, Value, object::ObjectExt},
    prelude::{
        ClockExt, ElementExt, ElementExtManual, GObjectExtManualGst, GstBinExt, GstBinExtManual,
        GstObjectExt, PadExt, PadExtManual,
    },
};
use gstreamer_sdp::SDPMessage;
use gstreamer_webrtc::{WebRTCDataChannel, WebRTCSessionDescription};
use libstrophe::Stanza;
use log::{error, info, warn};
use nanoid::nanoid;
use serde::Deserialize;
use std::{
    collections::HashMap,
    fs::DirBuilder,
    process::exit,
    sync::{Arc, Mutex, OnceLock, Weak, mpsc::Sender},
    thread,
};
use thiserror::Error;
use webrtc_sdp::{
    SdpType,
    attribute_type::{SdpAttribute, parse_attribute},
    parse_sdp,
};

#[derive(Deserialize)]
#[serde(tag = "colibriClass")]
#[allow(unused)]
enum ColibriMessage {
    #[serde(rename = "DominantSpeakerEndpointChangeEvent")]
    DominantSpeakerEndpointChange {
        #[serde(rename = "dominantSpeakerEndpoint")]
        dominant_speaker_endpoint: String,
        #[serde(rename = "previousSpeakers", default)]
        previous_speakers: Vec<String>,
        #[serde(default)]
        silence: bool,
    },

    #[serde(other)]
    Unknown,
}

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
struct Media {
    audio_muted: bool,
    video_muted: bool,
    screenshare_muted: bool,
}

#[derive(Debug)]
struct Branch {
    src_pad: Pad,
    queue: Element,
    depay: Element,
    parse: Option<Element>,
    muxer: Element,
    filesink: Element,
}

#[derive(Debug)]
#[allow(unused)]
pub struct RoomInner {
    name: String,
    webrtcbin: Element,
    pipeline: Pipeline,
    tx: Sender<Stanza>,
    ufrag: OnceLock<String>,
    pwd: OnceLock<String>,
    timeline_handler: TimelineHandler,
    participant_media: Mutex<HashMap<String, Media>>,
    branches: Mutex<HashMap<String, Branch>>,
    main_loop: MainLoop,
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
        self.main_loop.quit();
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

        let output_path = format!("recordings/{}", name);

        match DirBuilder::new().recursive(true).create(&output_path) {
            Err(err) => {
                error!("failed to create directory for: {name} err: {err:?} room");
            }
            _ => {}
        }

        webrtcbin.set_property_from_str("stun-server", &webrtc.stun_server);
        webrtcbin.set_property_from_str("bundle-policy", &webrtc.bundle_policy);

        pipeline.add_many([&webrtcbin])?;

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

        let timeline_engine = TimelineEngine::new(output_path.to_string());
        let timeline_handler = TimelineEngine::spawn(
            timeline_engine.output_path,
            name.clone(),
            timeline_engine.start_instant,
            timeline_engine.start_timestamp,
        );

        let main_loop = MainLoop::new(None, false);
        let room = Room(Arc::new(RoomInner {
            name: name.clone(),
            webrtcbin,
            pipeline: pipeline.clone(),
            tx,
            participant_media: Mutex::new(HashMap::new()),
            branches: Mutex::new(HashMap::new()),
            ufrag: OnceLock::new(),
            pwd: OnceLock::new(),
            timeline_handler: timeline_handler,
            main_loop: main_loop.clone(),
        }));
        thread::spawn(move || {
            main_loop.run();
        });

        let room_clone = room.downgrade();
        room.webrtcbin.connect_pad_added(move |_webrtc, pad| {
            let room = upgrade_weak!(room_clone);
            if let Err(e) = room.on_incoming_stream(pad) {
                error!("on_incoming_stream failed: {e:?}");
            }
        });

        Ok(room)
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

        let ssrc = structure.get::<u32>("ssrc").unwrap_or(0);
        let encoding = structure.get::<String>("encoding-name").unwrap_or_default();

        let endpoint = match self.timeline_handler.endpoint_for_ssrc(ssrc) {
            Some(e) => e,
            None => {
                warn!(
                    "no endpoint mapping for ssrc={} (pad {}), skipping",
                    ssrc,
                    pad.name()
                );
                return Ok(());
            }
        };

        let key = format!("{}-{}", endpoint.endpoint, endpoint.kind);
        {
            let branches = self.branches.lock().unwrap();
            if branches.contains_key(&key) {
                warn!(
                    "branch {} already exists (ssrc {}), skipping duplicate",
                    key, ssrc
                );
                return Ok(());
            }
        }

        let is_video = match encoding.as_str() {
            "VP8" | "VP9" | "H264" | "AV1" => true,
            "OPUS" => false,
            other => {
                warn!("unsupported encoding {} for ssrc {}", other, ssrc);
                return Ok(());
            }
        };

        let queue = ElementFactory::make("queue").build()?;

        let depay = match encoding.as_str() {
            "AV1" => ElementFactory::make("rtpav1depay").build()?,
            "VP8" => ElementFactory::make("rtpvp8depay").build()?,
            "VP9" => ElementFactory::make("rtpvp9depay").build()?,
            "H264" => ElementFactory::make("rtph264depay").build()?,
            "OPUS" => ElementFactory::make("rtpopusdepay").build()?,
            _ => unreachable!(),
        };

        // Parsers needed to produce muxer-acceptable framing/alignment.
        // AV1: av1parse converts OBU-alignment -> TU-alignment that matroskamux wants.
        // H264: h264parse for AVC framing. OPUS: opusparse for Ogg.
        // VP8/VP9: matroskamux accepts depayloader output directly.
        let parse = match encoding.as_str() {
            "AV1" => Some(ElementFactory::make("av1parse").build()?),
            "H264" => Some(ElementFactory::make("h264parse").build()?),
            "OPUS" => Some(ElementFactory::make("opusparse").build()?),
            _ => None,
        };

        // matroskamux for video (AV1/VP8/VP9/H264), oggmux for audio.
        let (muxer, ext) = if is_video {
            (ElementFactory::make("matroskamux").build()?, "mkv")
        } else {
            (ElementFactory::make("oggmux").build()?, "ogg")
        };

        let filesink = ElementFactory::make("filesink").build()?;
        let path = format!(
            "recordings/{}/{}/{}.{}",
            self.name,
            endpoint.endpoint,
            endpoint.kind.to_string(),
            ext
        );
        filesink.set_property("location", &path);

        // Add and link: queue -> depay -> [parse] -> muxer -> filesink
        self.pipeline
            .add_many([&queue, &depay, &muxer, &filesink])?;
        if let Some(parse) = &parse {
            self.pipeline.add(parse)?;
            Element::link_many([&queue, &depay, parse, &muxer])?;
        } else {
            Element::link_many([&queue, &depay, &muxer])?;
        }
        Element::link(&muxer, &filesink)?;

        // Link the webrtcbin src pad into the branch. (This was missing.)
        let qsink = queue
            .static_pad("sink")
            .ok_or(IncomingStreamError::MissingQueueSinkPad)?;
        pad.link(&qsink)?;

        // Bring the new branch up to the pipeline's running state.
        filesink.sync_state_with_parent()?;
        muxer.sync_state_with_parent()?;
        if let Some(parse) = &parse {
            parse.sync_state_with_parent()?;
        }
        depay.sync_state_with_parent()?;
        queue.sync_state_with_parent()?;

        info!("recording {} (ssrc={}) -> {}", encoding, ssrc, path);

        let key = format!("{}-{}", endpoint.endpoint, endpoint.kind);
        self.branches.lock().unwrap().insert(
            key,
            Branch {
                queue,
                src_pad: pad.clone(),
                depay: depay,
                muxer: muxer,
                parse: parse,
                filesink: filesink,
            },
        );

        Ok(())
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
        let room_clone = self.downgrade();
        dc.connect_on_message_string(move |_dc, data| match data {
            Some(data) => {
                info!(
                    "data channel on message for: {} room, data: {:?}",
                    room_name, data
                );

                let room = upgrade_weak!(room_clone);

                let msg: ColibriMessage = match serde_json::from_str(data) {
                    Ok(m) => m,
                    Err(e) => {
                        warn!("failed to parse colibri message: {e}; raw: {data:?}");
                        return;
                    }
                };

                match msg {
                    ColibriMessage::DominantSpeakerEndpointChange {
                        dominant_speaker_endpoint,
                        ..
                    } => {
                        room.timeline_handler
                            .dominant_change(Some(dominant_speaker_endpoint));
                    }
                    _ => {}
                }
            }
            None => {}
        });

        let room_name = self.name.clone();
        dc.connect_on_error(move |_dc, err| {
            error!("data channel error for: {} room, err: {:?}", room_name, err);
        });
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

    pub fn on_meeting_started(&self) {
        self.timeline_handler.meeting_started();
    }

    pub fn on_participant_joined(
        &mut self,
        endpoint_id: &str,
        nickname: &str,
        video_muted: bool,
        audio_muted: bool,
        screenshare_muted: bool,
    ) {
        let mut par_media = self.participant_media.lock().unwrap();

        par_media.insert(
            endpoint_id.to_string(),
            Media {
                audio_muted,
                video_muted,
                screenshare_muted,
            },
        );

        self.timeline_handler.participant_joined(
            Some(endpoint_id.to_string()),
            nickname.to_string(),
            video_muted,
            audio_muted,
        );
        // self.renderer_handler.participant_joined(
        //     endpoint_id,
        //     nickname,
        //     video_muted,
        //     audio_muted,
        //     screenshare_muted,
        // );
    }

    pub fn source_info_updated(
        &mut self,
        endpoint_id: &str,
        video_muted: bool,
        audio_muted: bool,
        screenshare_muted: bool,
    ) {
        let mut par_media = self.participant_media.lock().unwrap();
        let participant = par_media.get(endpoint_id);

        if let Some(participant) = participant {
            if participant.video_muted != video_muted {
                if participant.video_muted {
                    self.timeline_handler
                        .camera_on(Some(endpoint_id.to_string()));
                } else {
                    self.timeline_handler
                        .camera_off(Some(endpoint_id.to_string()));
                }
            }

            if participant.audio_muted != audio_muted {
                if participant.audio_muted {
                    self.timeline_handler
                        .audio_on(Some(endpoint_id.to_string()));
                } else {
                    self.timeline_handler
                        .audio_off(Some(endpoint_id.to_string()));
                }
            }

            if participant.screenshare_muted != screenshare_muted {
                if participant.screenshare_muted {
                    self.timeline_handler
                        .screenshare_on(Some(endpoint_id.to_string()));
                } else {
                    self.timeline_handler
                        .screenshare_off(Some(endpoint_id.to_string()));
                }
            }
        }

        par_media.insert(
            endpoint_id.to_string(),
            Media {
                audio_muted,
                video_muted,
                screenshare_muted,
            },
        );

        // self.renderer_handler.source_info_updated(
        //     endpoint_id,
        //     video_muted,
        //     audio_muted,
        //     screenshare_muted,
        // );
    }

    pub fn on_participant_left(&mut self, endpoint_id: &str) {
        self.timeline_handler
            .participant_left(Some(endpoint_id.to_string()));

        let keys: Vec<String> = {
            let branches = self.branches.lock().unwrap();
            branches
                .keys()
                .filter(|k| k.starts_with(endpoint_id))
                .cloned()
                .collect()
        };
        for key in keys {
            self.finalize_branch(&key);
        }
        // self.renderer_handler.participant_left(endpoint_id);
    }

    pub fn on_meeting_terminated(&self) {
        // let room_clone = self.downgrade();
        // self.pipeline.call_async(move |pipeline| {
        //     let room = upgrade_weak!(room_clone);
        //     let _ = pipeline.set_state(State::Null);
        //     room.timeline_handler.meeting_ended();
        // });
        self.pipeline
            .debug_to_dot_file_with_ts(gstreamer::DebugGraphDetails::all(), "shutdown_start");
        self.shutdown();
    }

    pub fn handle_register_ssrc(&self, parsed_source: ParsedSource) {
        let output_path = format!("recordings/{}/{}", self.name, parsed_source.endpoint_id);
        self.timeline_handler.register_ssrc(parsed_source);

        match DirBuilder::new().recursive(true).create(&output_path) {
            Err(err) => {
                error!(
                    "failed to create directory for: {} err: {err:?}",
                    output_path
                );
            }
            _ => {}
        }
    }

    fn finalize_branch(&self, source_key: &str) {
        let branch = {
            let mut branches = self.branches.lock().unwrap();
            branches.remove(source_key)
        };
        let branch = match branch {
            Some(b) => b,
            None => {
                warn!("finalize_branch: no branch found for key {}", source_key);
                return;
            }
        };

        let room_name = self.name.clone();
        let key = source_key.to_string();
        info!("[finalize:{}] start (room: {})", key, room_name);

        let pipeline = self.pipeline.clone();

        // 1. Block the webrtcbin src pad so no more buffers enter the branch.
        let src_pad = branch.src_pad.clone();
        info!(
            "[finalize:{}] adding block probe on src pad {}",
            key,
            src_pad.name()
        );

        src_pad.add_probe(
        gstreamer::PadProbeType::BLOCK_DOWNSTREAM,
        move |pad, _info| {
            info!("[finalize:{}] src pad blocked, unlinking + sending EOS", key);

            // 2. Unlink from the queue and inject EOS into the queue chain.
            let qsink = match branch.queue.static_pad("sink") {
                Some(p) => p,
                None => {
                    error!("[finalize:{}] queue has no sink pad, aborting", key);
                    return gstreamer::PadProbeReturn::Remove;
                }
            };

            let _ = pad.unlink(&qsink);
            info!("[finalize:{}] unlinked src pad from queue sink", key);

            let eos_sent = qsink.send_event(gstreamer::event::Eos::new());
            info!("[finalize:{}] EOS sent into queue chain: {}", key, eos_sent);

            // 3. When EOS reaches the filesink, tear down the branch elements.
            let fsink_pad = match branch.filesink.static_pad("sink") {
                Some(p) => p,
                None => {
                    error!("[finalize:{}] filesink has no sink pad, aborting", key);
                    return gstreamer::PadProbeReturn::Remove;
                }
            };

            let pipeline = pipeline.clone();
            let mut elements = vec![
                branch.queue.clone(),
                branch.depay.clone(),
                branch.muxer.clone(),
                branch.filesink.clone(),
            ];
            if let Some(p) = branch.parse.clone() {
                elements.push(p);
            }
            info!(
                "[finalize:{}] added filesink EOS probe, waiting for EOS to reach filesink ({} elements queued for removal)",
                key,
                elements.len()
            );

            let probe_key = key.clone();
            fsink_pad.add_probe(
                gstreamer::PadProbeType::EVENT_DOWNSTREAM,
                move |_p, info| {
                    if let Some(gstreamer::PadProbeData::Event(ev)) = &info.data {
                        if ev.type_() == gstreamer::EventType::Eos {
                            info!(
                                "[finalize:{}] EOS reached filesink — footer written, tearing down branch",
                                probe_key
                            );
                            let elements = elements.clone();
                            let pipeline = pipeline.clone();
                            let idle_key = probe_key.clone();
                            gstreamer::glib::idle_add_once(move || {
                                for el in &elements {
                                    let name = el.name();
                                    let _ = el.set_state(State::Null);
                                    match pipeline.remove(el) {
                                        Ok(_) => info!(
                                            "[finalize:{}] removed element {} from pipeline",
                                            idle_key, name
                                        ),
                                        Err(_) => error!(
                                            "[finalize:{}] failed to remove element {} from pipeline",
                                            idle_key, name
                                        ),
                                    }
                                }
                                info!("[finalize:{}] branch teardown complete", idle_key);
                            });
                            return gstreamer::PadProbeReturn::Remove;
                        }
                    }
                    gstreamer::PadProbeReturn::Ok
                },
            );

            // Remove the block probe — we're done with it.
            gstreamer::PadProbeReturn::Remove
        },
    );

        info!(
            "[finalize:{}] block probe installed, returning (finalization continues async)",
            source_key
        );
    }

    fn shutdown(&self) {
        self.timeline_handler.meeting_ended();

        let branches: Vec<(String, Branch)> = {
            let mut b = self.branches.lock().unwrap();
            b.drain().collect()
        };
        if branches.is_empty() {
            let _ = self.pipeline.set_state(State::Null);
            return;
        }

        let total = branches.len();
        let remaining = Arc::new(std::sync::atomic::AtomicUsize::new(total));
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();

        for (key, branch) in branches {
            let fsink_pad = branch.filesink.static_pad("sink").unwrap();
            let remaining_c = remaining.clone();
            let done_tx = done_tx.clone();
            let key_c = key.clone();

            fsink_pad.add_probe(
                gstreamer::PadProbeType::EVENT_DOWNSTREAM,
                move |_p, info| {
                    if let Some(gstreamer::PadProbeData::Event(ev)) = &info.data {
                        if ev.type_() == gstreamer::EventType::Eos {
                            let left =
                                remaining_c.fetch_sub(1, std::sync::atomic::Ordering::SeqCst) - 1;
                            info!(
                                "[shutdown:{}] filesink EOS — footer written ({} left)",
                                key_c, left
                            );
                            if left == 0 {
                                let _ = done_tx.send(());
                            }
                            return gstreamer::PadProbeReturn::Remove;
                        }
                    }
                    gstreamer::PadProbeReturn::Ok
                },
            );

            let qsink = branch.queue.static_pad("sink").unwrap();
            let _ = branch.src_pad.unlink(&qsink);
            let sent = qsink.send_event(gstreamer::event::Eos::new());
            info!("[shutdown:{}] EOS injected: {}", key, sent);
        }
        drop(done_tx);

        // BLOCK until every filesink has seen EOS (footer + Cues fully written).
        match done_rx.recv_timeout(std::time::Duration::from_secs(10)) {
            Ok(_) => info!("all branches finalized for room: {}", self.name),
            Err(_) => warn!("finalize wait timed out for room: {}", self.name),
        }

        // Now it's safe to tear down — the seek-back header rewrite has completed.
        let _ = self.pipeline.set_state(State::Null);
    }
}
