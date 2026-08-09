//! The XMPP client: connection setup and stanza dispatch.
//!
//! One connection serves every meeting the recorder is in. Incoming presence
//! and IQ stanzas are dispatched to the [`RoomManager`], and outgoing stanzas
//! are queued on an mpsc channel that a timed handler flushes on the
//! connection thread — libstrophe's connection is not `Sync`, so the room
//! threads can never touch it directly.

use libstrophe::{
    ConnectClientError, Connection, ConnectionEvent, ConnectionFlags, Context, HandlerResult,
    Stanza,
};
use log::{debug, error, info};
use nanoid::nanoid;
use std::{
    sync::{
        Arc, Mutex,
        mpsc::{Receiver, SendError, Sender},
    },
    time::Duration,
};
use thiserror::Error;

use crate::{
    config::{ConfigSettings, Webrtc},
    iq::Iq,
    make_stanza,
    presence::{ParticipantPresence, PresenceLifecycle},
    room_manager::{RoomManager, Rooms},
};

/// Failures from connecting to XMPP or building and queueing stanzas.
#[derive(Error, Debug)]
pub enum AppError {
    #[error("failed to initialize xmpp: {0}")]
    InitializationError(libstrophe::Error),

    #[error("failed to connect: {0:?}")]
    ConnectClientError(ConnectClientError<'static, 'static>),

    #[error("failed to parse stanza for room '{room}': {source}")]
    ParseError {
        room: String,
        #[source]
        source: libstrophe::Error,
    },
    #[error("failed to send stanza for room '{room}': {source:?}")]
    SendError {
        room: String,
        #[source]
        source: SendError<Stanza>,
    },

    #[error("failed to connect: something went wrong")]
    Unkown,
}

/// A connected XMPP client. Holding it keeps the libstrophe context alive;
/// [`App::xmpp_run`] then drives the event loop.
#[allow(unused)]
pub struct App {
    xmpp_context: Context<'static, 'static>,
    tx: Sender<Stanza>,
}

impl App {
    fn new(context: Context<'static, 'static>, tx: Sender<Stanza>) -> Self {
        App {
            xmpp_context: context,
            tx,
        }
    }

    /// Builds an unconnected client for `jid`.
    ///
    /// TLS is disabled because the recorder is expected to run beside the
    /// XMPP server on a trusted network; over an untrusted one this sends
    /// credentials in the clear.
    fn init_xmpp_connection(
        jid: &str,
        password: &str,
    ) -> Result<Connection<'static, 'static>, AppError> {
        let ctx = libstrophe::Context::new_with_default_logger();
        let mut conn = libstrophe::Connection::new(ctx);
        conn.set_jid(jid);
        conn.set_pass(password);
        let disable_tls = conn.set_flags(ConnectionFlags::DISABLE_TLS);
        if let Err(err) = disable_tls {
            return Err(AppError::InitializationError(err));
        }

        return Ok(conn);
    }

    /// Builds the connection-event callback.
    ///
    /// On connect it installs three handlers: a zero-delay timed handler that
    /// flushes the outgoing stanza queue, and the presence and IQ handlers.
    /// Disconnect stops the context, which ends [`App::xmpp_run`].
    fn xmpp_connection_handler(
        webrtc: Webrtc,
        tx: Sender<Stanza>,
        rx: Receiver<Stanza>,
        room_manager: RoomManager,
    ) -> impl FnMut(&libstrophe::Context, &mut Connection, ConnectionEvent) + Send {
        let rx_shared = Arc::new(Mutex::new(rx));
        let webrtc = Arc::new(webrtc);
        let room_manager = Arc::new(Mutex::new(room_manager));
        move |ctx, conn, evt| match evt {
            ConnectionEvent::Connect => {
                info!("XMPP connected");

                let rx_clone = rx_shared.clone();
                conn.timed_handler_add(
                    move |_ctx, conn| {
                        while let Ok(stanza) = rx_clone.lock().unwrap().try_recv() {
                            debug!("Sending Stanza: {}", stanza.to_string());
                            conn.send(&stanza);
                        }
                        HandlerResult::KeepHandler
                    },
                    Duration::from_millis(0),
                );

                conn.handler_add(
                    Self::handle_presence(room_manager.clone(), tx.clone(), webrtc.clone()),
                    None,
                    Some("presence"),
                    None,
                );
                conn.handler_add(
                    Self::handle_iq(room_manager.clone(), tx.clone()),
                    None,
                    Some("iq"),
                    None,
                );
            }

            ConnectionEvent::Disconnect(conn_error) => {
                if let Some(err) = conn_error {
                    error!("XMPP disconnected with error: {:?}", err);
                } else {
                    error!("XMPP disconnected");
                }

                ctx.stop();
            }

            _ => {}
        }
    }

    /// IQ handler: routes `<jingle>` to session handling and `<query>` to
    /// service discovery, ignoring anything else.
    fn handle_iq(
        room_manager: Rooms,
        tx: Sender<Stanza>,
    ) -> impl FnMut(&Context, &mut Connection, &Stanza) -> HandlerResult {
        move |_ctx, _conn, stanza| {
            debug!("iq stanza received: {}", stanza.to_string());
            let mut iq = Iq::new(stanza);

            if let Some(child) = stanza.get_first_child() {
                match child.name() {
                    Some("jingle") => {
                        iq.handle_jingle(&child, room_manager.clone(), tx.clone());
                    }
                    Some("query") => {
                        iq.handle_query(&child, tx.clone()).ok();
                    }
                    _ => {}
                }
            }

            HandlerResult::KeepHandler
        }
    }

    /// Presence handler: classifies the stanza and forwards it to the room
    /// manager. The room name is the local part of the sender's JID.
    fn handle_presence(
        room_manager: Rooms,
        tx: Sender<Stanza>,
        webrtc: Arc<Webrtc>,
    ) -> impl FnMut(&Context, &mut Connection, &Stanza) -> HandlerResult {
        move |_ctx, _conn, stanza| {
            debug!("presence stanza received: {}", stanza.to_string());
            if let Some(p_life_cycle) = ParticipantPresence::from_presence(stanza) {
                match p_life_cycle {
                    PresenceLifecycle::ParticipantJoined(participant) => {
                        let room_name = participant
                            .from
                            .split('@')
                            .next()
                            .unwrap_or_default()
                            .to_string();

                        let mut rm = room_manager.lock().unwrap();

                        match rm.on_participant_joined(&room_name, tx.clone(), &webrtc, participant)
                        {
                            Ok(_) => {
                                info!("processed participant joined for: {room_name} room");
                            }
                            Err(err) => {
                                error!(
                                    "failed to process participant joined for: {room_name} | err: {err:?}"
                                );
                            }
                        }
                    }

                    PresenceLifecycle::ParticipantLeft(participant) => {
                        let room_name = participant.from.split('@').next().unwrap_or_default();
                        let mut rm = room_manager.lock().unwrap();
                        rm.on_participant_left(room_name, &participant.endpoint_id);
                        info!("processed participant left for: {room_name} room");
                    }

                    PresenceLifecycle::MeetingTerminated(participant) => {
                        let room_name = participant.from.split('@').next().unwrap_or_default();
                        let mut rm = room_manager.lock().unwrap();
                        rm.on_meeting_terminated(room_name);
                        info!("processed meeting terminated for: {room_name} room");
                    }
                }
            }
            HandlerResult::KeepHandler
        }
    }

    /// Connects to the configured server. `rx` is the queue this client drains
    /// to send stanzas the rooms produce; `tx` is handed to those rooms.
    pub fn connect(
        config: &ConfigSettings,
        room_manager: RoomManager,
        tx: Sender<Stanza>,
        rx: Receiver<Stanza>,
    ) -> Result<Self, AppError> {
        let xmpp_client = &config.xmpp_client;
        let webrtc = config.webrtc.clone();
        let conn = Self::init_xmpp_connection(&xmpp_client.bot_jid, &xmpp_client.bot_password)?;
        let ctx = conn.connect_client(
            Some(&xmpp_client.domain_url),
            Some(xmpp_client.domain_port),
            Self::xmpp_connection_handler(webrtc, tx.clone(), rx, room_manager),
        );

        match ctx {
            Ok(ctx) => Ok(Self::new(ctx, tx)),
            Err(err) => Err(AppError::ConnectClientError(err)),
        }
    }

    /// Runs the XMPP event loop on the calling thread until disconnect.
    pub fn xmpp_run(&mut self) {
        self.xmpp_context.run();
    }

    /// Queues MUC presence to join `room`, using a random nickname as the
    /// recorder's endpoint id.
    ///
    /// Returns as soon as the stanza is queued — the room itself is created
    /// later, when the server's presence reply comes back.
    pub fn handle_join_room(tx: &Sender<Stanza>, room: &str) -> Result<String, AppError> {
        debug!("room: {room}");

        let x = make_stanza!("x", {
            "xmlns" => "http://jabber.org/protocol/muc"
        }, [])
        .map_err(|e| AppError::ParseError {
            room: room.to_string(),
            source: e,
        })?;

        let presence = make_stanza!("presence", {
            "to" => format!("{}@muc.meet.jitsi/{}", room, nanoid!(10))
        }, [x])
        .map_err(|e| AppError::ParseError {
            room: room.to_string(),
            source: e,
        })?;

        tx.send(presence).map_err(|e| AppError::SendError {
            room: room.to_string(),
            source: e,
        })?;

        Ok(room.to_string())
    }
}
