//! A single meeting: its WebRTC connection, its GStreamer pipeline, and the
//! recordings running inside it.
//!
//! [`Room`] is an `Arc` handle so the same room can be reached from the XMPP
//! thread, GStreamer's callbacks, and its own bus-watcher thread; callbacks
//! hold a [`RoomWeak`] so a finished room can still be dropped. All mutable
//! state sits behind mutexes on [`RoomInner`].
//!
//! # Recording lifecycle
//!
//! Jingle announces which SSRC belongs to which participant source, then
//! `webrtcbin` produces a pad per SSRC. A pad that arrives before its
//! announcement is blocked and parked until [`Room::handle_register_ssrc`]
//! can route it. Once routed, the pad gets a
//! [`Branch`] that writes one file.
//!
//! Teardown is EOS-driven, because a muxer only writes its index when it sees
//! EOS: [`Room::on_meeting_terminated`] detaches every branch and posts a
//! drain message, the bus watcher removes each branch as its EOS arrives, and
//! only then is the pipeline stopped and the timeline flushed.

use crate::{
    avatar::generate_avatar,
    config::Webrtc,
    get_attribute,
    iq::{Iq, jingle_action::ParsedSource},
    make_stanza,
    participant::{Participant, branch::Branch},
    presence::Presence,
    sdp::Sdp,
    timeline::{timeline_engine::TimelineEngine, timeline_handler::TimelineHandler},
    upgrade_weak,
    util::dir_builder,
    xep::XEP,
};
use gstreamer::{
    Bus, ClockTime, Element, ElementFactory, MessageView, Pad, PadDirection, PadLinkError,
    PadProbeReturn, PadProbeType, Pipeline, Promise, PromiseError, State, StateChangeError,
    Structure, StructureRef,
    glib::{BoolError, Value, object::ObjectExt},
    message::Application,
    prelude::{
        ElementExt, ElementExtManual, GObjectExtManualGst, GstBinExt, GstBinExtManual,
        GstObjectExt, PadExt, PadExtManual,
    },
};
use gstreamer_sdp::{SDPMessage, SDPMessageRef};
use gstreamer_webrtc::{WebRTCDataChannel, WebRTCSessionDescription};
use libstrophe::Stanza;
use log::{error, info, warn};
use nanoid::nanoid;
use serde::Deserialize;
use std::{
    collections::HashMap,
    process::exit,
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
    },
    time::{Duration, Instant},
};
use thiserror::Error;
use webrtc_sdp::{
    SdpType,
    attribute_type::{SdpAttribute, parse_attribute},
    parse_sdp,
};

/// Messages the JVB sends over the Colibri data channel.
///
/// Only dominant-speaker changes are acted on; anything else deserializes to
/// [`ColibriMessage::Unknown`] so an unfamiliar message is ignored rather than
/// failing the parse.
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

/// Failures while attaching a recording branch to a new `webrtcbin` pad.
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

/// Longest the bus watcher waits after meeting termination for every muxer to
/// finalize its file before the pipeline is forced to Null anyway.
const FINALIZE_TIMEOUT: Duration = Duration::from_secs(10);

/// Longest the bus watcher waits for timeline.json/metadata.json to be
/// written after the pipeline has drained.
const TIMELINE_WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// Application message posted on the bus when the meeting terminates.
const DRAIN_MESSAGE: &str = "room-draining";

#[derive(Debug)]
#[allow(unused)]
pub struct RoomInner {
    pub name: String,
    webrtcbin: Element,
    pipeline: Pipeline,
    tx: Sender<Stanza>,
    ufrag: OnceLock<String>,
    pwd: OnceLock<String>,
    recorder: Presence,
    timeline_handler: TimelineHandler,
    /// Everything scoped to one endpoint — media state, sources, recordings.
    participants: Mutex<HashMap<String, Participant>>,
    /// Pads that arrived before their SSRC was announced, so we don't yet know
    /// which participant owns them. Keyed by ssrc, blocked until routed.
    pending_pads: Mutex<HashMap<u32, (Pad, gstreamer::PadProbeId)>>,
    draining: AtomicBool,
}

/// Weak handle held by GStreamer callbacks, which outlive the room.
#[derive(Debug)]
pub struct RoomWeak(Weak<RoomInner>);

/// Shared handle to a meeting. Cloning is cheap and every clone refers to the
/// same [`RoomInner`].
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
    /// Stops the pipeline when the last handle goes away — a backstop for
    /// rooms that never drained normally.
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
    /// Weak handle for use inside GStreamer callbacks.
    pub fn downgrade(&self) -> RoomWeak {
        RoomWeak(Arc::downgrade(&self.0))
    }

    /// Creates the room, its output directory, its pipeline, and the threads
    /// that serve them.
    ///
    /// The pipeline goes to Playing asynchronously and the bus watcher and
    /// timeline collector start immediately, so the room is ready before the
    /// first Jingle stanza arrives. A failure to reach Playing is fatal and
    /// exits the process.
    pub fn new(
        name: String,
        tx: Sender<Stanza>,
        webrtc: &Webrtc,
        presence: Presence,
    ) -> Result<Self, BoolError> {
        let pipeline = Pipeline::new();
        let webrtcbin = ElementFactory::make("webrtcbin").build()?;

        let output_path = format!("recordings/{}", name);
        dir_builder(&output_path);

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
        let timeline_handler = Self::initialize_timeline(&name, &output_path);

        let room = Room(Arc::new(RoomInner {
            name: name.clone(),
            webrtcbin,
            pipeline: pipeline.clone(),
            tx,
            recorder: presence,
            participants: Mutex::new(HashMap::new()),
            ufrag: OnceLock::new(),
            pwd: OnceLock::new(),
            timeline_handler: timeline_handler,
            pending_pads: Mutex::new(HashMap::new()),
            draining: AtomicBool::new(false),
        }));

        room.on_meeting_started();
        room.spawn_bus_watcher();

        let room_clone = room.downgrade();
        room.webrtcbin.connect_pad_added(move |_webrtc, pad| {
            let room = upgrade_weak!(room_clone);
            if let Err(e) = room.on_incoming_stream(pad) {
                error!("on_incoming_stream failed: {e:?}");
            }
        });

        Ok(room)
    }

    /// Starts the timeline collector and returns the handle events are posted
    /// to. This also sets the meeting's time zero.
    fn initialize_timeline(room_name: &str, output_path: &str) -> TimelineHandler {
        let timeline_engine = TimelineEngine::new(output_path.to_string());
        TimelineEngine::spawn(
            timeline_engine.output_path,
            room_name.to_string(),
            timeline_engine.start_instant,
            timeline_engine.start_timestamp,
        )
    }

    /// Starts recording a newly added `webrtcbin` pad.
    ///
    /// Called from the `pad-added` signal and again from
    /// [`Room::handle_register_ssrc`] when a parked pad is released. Non-RTP
    /// pads and pads arriving during drain are ignored; a pad whose SSRC has
    /// no owner yet is parked rather than dropped.
    pub fn on_incoming_stream(&self, pad: &Pad) -> Result<(), IncomingStreamError> {
        if pad.direction() != PadDirection::Src {
            return Ok(());
        }

        if self.draining.load(Ordering::SeqCst) {
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

        let mut participants = self.participants.lock().unwrap();

        // The SSRC is the only thing the pad tells us; the owning participant
        // is whoever announced it over Jingle.
        let owner = participants
            .iter()
            .find(|(_, participant)| participant.source_for_ssrc(ssrc).is_some())
            .map(|(endpoint_id, _)| endpoint_id.clone());

        let Some(endpoint_id) = owner else {
            drop(participants);
            self.park_pad(ssrc, pad);
            return Ok(());
        };

        let participant = participants
            .get_mut(&endpoint_id)
            .expect("owner was just looked up");
        let source = participant
            .source_for_ssrc(ssrc)
            .expect("owner was matched on this ssrc")
            .clone();

        let branch = Branch::build(ssrc, pad.clone(), &encoding, &source, &self.name)?;
        self.pipeline.add(&branch.bin)?;
        branch.bin.sync_state_with_parent()?;
        pad.link(&branch.entry_pad)?;

        info!("recording {encoding} (ssrc={ssrc}) -> {}", &branch.path);
        participant.add_branch(ssrc, branch);

        Ok(())
    }

    /// Blocks a pad whose SSRC has not been announced yet and holds it until
    /// `handle_register_ssrc` can route it to a participant.
    fn park_pad(&self, ssrc: u32, pad: &Pad) {
        warn!("no source mapping for ssrc={ssrc} yet, parking pad until registered");

        match pad.add_probe(PadProbeType::BLOCK_DOWNSTREAM, |_pad, _info| {
            PadProbeReturn::Ok
        }) {
            Some(probe_id) => {
                self.pending_pads
                    .lock()
                    .unwrap()
                    .insert(ssrc, (pad.clone(), probe_id));
            }
            None => warn!("couldn't get probe id for ssrc={ssrc}"),
        }
    }

    /// Sets up the Colibri data channel to the JVB.
    ///
    /// On open it sends receiver constraints asking for every participant
    /// (`lastN: -1`) at up to 720p — without them the bridge forwards only a
    /// few streams and the recording would be missing participants. Incoming
    /// messages supply dominant-speaker changes for the timeline.
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

    /// Connects `webrtcbin`'s candidate signal to this Jingle session, so
    /// locally gathered candidates are sent as `transport-info`.
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

    /// Sends one locally gathered ICE candidate as a Jingle `transport-info`.
    ///
    /// The content name is derived from the m-line index, matching the order
    /// the offer was built in (audio, video, data). End-of-candidates (an
    /// empty candidate string) is not forwarded.
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

    /// Completes negotiation once `webrtcbin` has produced an answer.
    ///
    /// Opens the Colibri data channel, applies the answer as the local
    /// description, and sends it back as `session-accept`. The data channel is
    /// created here, before the local description is set, so it is part of the
    /// same negotiation rather than triggering a renegotiation.
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

        match self.parse_sdp_answer(answer.sdp(), &to, &from, sid, initiator) {
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

    /// Converts the SDP answer to Jingle and queues the `session-accept` IQ.
    ///
    /// The room's ICE credentials are latched from the first answer that
    /// carries them, since later renegotiations reuse them.
    fn parse_sdp_answer(
        &self,
        sdp_message: &SDPMessageRef,
        to: &str,
        from: &str,
        sid: &str,
        initiator: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let sdp_answer = sdp_message.as_text()?;
        let sdp_session = parse_sdp(&sdp_answer, true)?;
        let sdp = Sdp::new(&sdp_session);
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
        let jingle = sdp.parse_sdp_to_jingle(initiator, sid, to)?;
        let iq = make_stanza!("iq", {
            "id" => nanoid!(),
            "to" => from,
            "from" => to,
            "type" => "set"
        }, [jingle])?;

        self.tx.send(iq)?;

        Ok(())
    }

    /// Applies the offer built from `session-initiate` and asks `webrtcbin`
    /// for an answer.
    ///
    /// Both steps run through promises on GStreamer's threads, so this returns
    /// immediately; the answer continues in
    /// `on_answer_created`.
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

    /// Marks the start of the meeting on the timeline.
    fn on_meeting_started(&self) {
        self.timeline_handler.meeting_started();
    }

    /// Whether this endpoint is already tracked, i.e. whether incoming
    /// presence is a join or a mute update.
    pub fn endpoint_available(&self, endpoint: &str) -> bool {
        self.participants.lock().unwrap().contains_key(endpoint)
    }

    /// `recordings/<room>/<endpoint>`, created on demand since every artifact
    /// for a participant lands there.
    fn participant_dir(&self, endpoint_id: &str) -> String {
        let dir_path = format!("recordings/{}/{}", self.name, endpoint_id);
        dir_builder(&dir_path);
        dir_path
    }

    /// Renders the stand-in shown while the camera is muted, once.
    fn ensure_avatar(&self, endpoint_id: &str, nickname: &str, participant: &mut Participant) {
        if !participant.needs_avatar() {
            return;
        }

        let path = format!("{}/avatar.png", self.participant_dir(endpoint_id));
        match generate_avatar(nickname, &path) {
            Ok(_) => {
                info!("avatar generated for {endpoint_id} at {path}");
                participant.media.set_avatar_generated(true);
            }
            Err(err) => error!("failed to generate avatar for {endpoint_id}: {err}"),
        }
    }

    /// Registers a participant who has just joined and records the join, plus
    /// their initial camera/audio state, on the timeline.
    pub fn on_participant_joined(
        &self,
        endpoint_id: &str,
        nickname: &str,
        video_muted: bool,
        audio_muted: bool,
        screenshare_muted: bool,
    ) {
        let mut participant = Participant::new(nickname);
        participant.update_media(audio_muted, video_muted, screenshare_muted);
        self.ensure_avatar(endpoint_id, nickname, &mut participant);

        let mut participants = self.participants.lock().unwrap();

        participants.insert(endpoint_id.to_string(), participant);

        self.timeline_handler.participant_joined(
            Some(endpoint_id.to_string()),
            nickname.to_string(),
            video_muted,
            audio_muted,
            participants.is_empty(),
        );
    }

    /// Applies a mute update for a participant already in the room, emitting a
    /// timeline event for each flag that actually changed.
    pub fn source_info_updated(
        &self,
        endpoint_id: &str,
        nickname: &str,
        video_muted: bool,
        audio_muted: bool,
        screenshare_muted: bool,
    ) {
        let mut participants = self.participants.lock().unwrap();
        let Some(participant) = participants.get_mut(endpoint_id) else {
            warn!(
                "source info for unknown endpoint {endpoint_id} in room {}",
                self.name
            );
            return;
        };

        let changes = participant.update_media(audio_muted, video_muted, screenshare_muted);
        self.ensure_avatar(endpoint_id, nickname, participant);

        let endpoint = || Some(endpoint_id.to_string());
        match changes.video_muted {
            Some(true) => self.timeline_handler.camera_off(endpoint()),
            Some(false) => self.timeline_handler.camera_on(endpoint()),
            None => {}
        }
        match changes.audio_muted {
            Some(true) => self.timeline_handler.audio_off(endpoint()),
            Some(false) => self.timeline_handler.audio_on(endpoint()),
            None => {}
        }
        match changes.screenshare_muted {
            Some(true) => self.timeline_handler.screenshare_off(endpoint()),
            Some(false) => self.timeline_handler.screenshare_on(endpoint()),
            None => {}
        }
    }

    /// Records the departure and closes the files that participant was still
    /// being recorded to.
    pub fn on_participant_left(&self, endpoint_id: &str) {
        self.timeline_handler
            .participant_left(Some(endpoint_id.to_string()));

        // The participant stays in the map until its branches have drained;
        // the bus watcher removes each one as its EOS arrives.
        if let Some(participant) = self.participants.lock().unwrap().get_mut(endpoint_id) {
            participant.finalize_branches();
        }
    }

    /// Starts draining the room and returns immediately; the bus watcher
    /// finishes the teardown once every recording has finalized.
    /// Detaches every branch, then returns immediately; the bus watcher
    /// finishes teardown once each recording has finalized.
    ///
    /// Guarded so a second call is a no-op — the room can be told the meeting
    /// ended more than once.
    pub fn on_meeting_terminated(&self) {
        if self.draining.swap(true, Ordering::SeqCst) {
            return;
        }

        self.pipeline
            .debug_to_dot_file_with_ts(gstreamer::DebugGraphDetails::all(), "shutdown_start");

        self.timeline_handler.meeting_ended();

        // Parked pads never produced a file; their block probes stay
        // installed so nothing flows into an unlinked pad while draining.
        self.pending_pads.lock().unwrap().clear();

        for participant in self.participants.lock().unwrap().values_mut() {
            participant.finalize_branches();
        }

        match self.pipeline.bus() {
            Some(bus) => {
                let _ = bus.post(Application::new(Structure::new_empty(DRAIN_MESSAGE)));
            }
            None => error!("no bus to signal drain for room {}", self.name),
        }
    }

    /// Records what an SSRC carries and, if a pad for it was parked, starts
    /// recording that pad now.
    ///
    /// The parked pad's block probe is removed only after the branch is
    /// attached, so no buffer escapes into an unlinked pad.
    pub fn handle_register_ssrc(&self, parsed_source: ParsedSource) {
        let ssrc = parsed_source.ssrc;
        let endpoint_id = parsed_source.endpoint_id.clone();
        self.participant_dir(&endpoint_id);

        match self.participants.lock().unwrap().get_mut(&endpoint_id) {
            Some(participant) => participant.register_ssrc(parsed_source),
            None => {
                warn!(
                    "ssrc={ssrc} announced for unknown endpoint {endpoint_id} in room {}",
                    self.name
                );
                return;
            }
        }

        let parked = self.pending_pads.lock().unwrap().remove(&ssrc);
        if let Some((pad, probe_id)) = parked {
            info!("registered ssrc={}, processing parked pad", ssrc);
            let result = self.on_incoming_stream(&pad);
            pad.remove_probe(probe_id);
            if let Err(e) = result {
                error!(
                    "parked pad on_incoming_stream failed for ssrc={}: {e:?}",
                    ssrc
                );
            }
        }
    }

    /// Pulls the first branch matching `pred` out of whichever participant
    /// owns it, so a bus message can be traced back to its recording.
    fn take_branch(&self, pred: impl Fn(&Branch) -> bool) -> Option<Branch> {
        self.participants
            .lock()
            .unwrap()
            .values_mut()
            .find_map(|participant| participant.take_branch(&pred))
    }

    fn recording_in_progress(&self) -> bool {
        self.participants
            .lock()
            .unwrap()
            .values()
            .any(Participant::has_branches)
    }

    /// Starts the bus-watcher thread.
    ///
    /// The thread holds a strong [`Room`], which is what keeps a room alive
    /// after the manager drops it at meeting end, until draining finishes.
    fn spawn_bus_watcher(&self) {
        let Some(bus) = self.pipeline.bus() else {
            error!(
                "pipeline has no bus for room {}; recordings will not finalize",
                self.name
            );
            return;
        };

        let room = self.clone();
        std::thread::spawn(move || room.run_bus_watcher(bus));
    }

    /// Bus loop: retires branches as their EOS arrives, and ends the room once
    /// drain completes.
    ///
    /// Before drain starts it blocks on the bus indefinitely. The drain
    /// message arms a [`FINALIZE_TIMEOUT`] deadline, after which the loop
    /// exits even if some muxer never reported EOS — a stuck branch must not
    /// hold the process open.
    fn run_bus_watcher(&self, bus: Bus) {
        let mut drain_deadline: Option<Instant> = None;

        loop {
            let timeout = drain_deadline.map(|deadline| {
                ClockTime::try_from(deadline.saturating_duration_since(Instant::now()))
                    .unwrap_or(ClockTime::ZERO)
            });

            match bus.timed_pop(timeout) {
                Some(msg) => match msg.view() {
                    MessageView::Element(_) => {
                        if let Some(inner) = Self::forwarded_message(&msg) {
                            if matches!(inner.view(), MessageView::Eos(_)) {
                                if let Some(src) = inner.src() {
                                    self.on_branch_eos(src);
                                }
                            }
                        }
                    }
                    MessageView::Application(app) => {
                        if app.structure().is_some_and(|s| s.name() == DRAIN_MESSAGE) {
                            drain_deadline = Some(Instant::now() + FINALIZE_TIMEOUT);
                        }
                    }
                    MessageView::Error(err) => self.on_bus_error(err),
                    _ => {}
                },
                None => {
                    error!(
                        "timed out draining room {}; remaining recordings may be truncated",
                        self.name
                    );
                    break;
                }
            }

            if drain_deadline.is_some() && !self.recording_in_progress() {
                break;
            }
        }

        self.complete_drain();
    }

    /// Unwraps a `GstBinForwarded` element message.
    ///
    /// Branch bins are built with `message-forward`, so their EOS reaches the
    /// pipeline bus wrapped inside an element message rather than as a
    /// pipeline-level EOS.
    fn forwarded_message(msg: &gstreamer::Message) -> Option<gstreamer::Message> {
        let structure = msg.structure()?;
        if structure.name() != "GstBinForwarded" {
            return None;
        }
        structure.get::<gstreamer::Message>("message").ok()
    }

    /// A branch's filesink has seen EOS, so its file is complete: remove the
    /// branch from the pipeline.
    fn on_branch_eos(&self, src: &gstreamer::Object) {
        let Some(branch) = self.take_branch(|branch| branch.owns_filesink(src)) else {
            return;
        };

        branch.teardown(&self.pipeline);
    }

    /// Tears down the branch an error came from, leaving the rest of the room
    /// recording. Errors from outside any branch are only logged.
    fn on_bus_error(&self, err: &gstreamer::message::Error) {
        error!(
            "pipeline error in room {} from {:?}: {} ({:?})",
            self.name,
            err.src().map(|s| s.name()),
            err.error(),
            err.debug()
        );

        let Some(src) = err.src() else {
            return;
        };

        if let Some(branch) = self.take_branch(|branch| branch.contains(&src)) {
            warn!(
                "recording {} in room {} failed; the file may be incomplete",
                branch.path, self.name
            );
            branch.teardown(&self.pipeline);
        }
    }

    /// Final teardown: abandon any branch that never finalized, stop the
    /// pipeline, and wait for the timeline files.
    ///
    /// Once this returns, everything under `recordings/<room>/` is ready to be
    /// picked up by the render pass.
    fn complete_drain(&self) {
        for participant in self.participants.lock().unwrap().values_mut() {
            participant.abandon_branches(&self.pipeline);
        }

        if let Err(err) = self.pipeline.set_state(State::Null) {
            error!("failed to stop pipeline for room {}: {err:?}", self.name);
        }

        if self.timeline_handler.wait_for_files(TIMELINE_WRITE_TIMEOUT) {
            info!(
                "room {} drained: recordings, timeline and metadata are ready for pickup",
                self.name
            );
            // TODO: notify the render server from here.
        } else {
            error!(
                "timeline files for room {} were not written in time",
                self.name
            );
        }
    }
}
