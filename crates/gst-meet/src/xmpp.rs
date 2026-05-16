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
    room::Room,
    room_manager::RoomManager,
};

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

    fn xmpp_connection_handler(
        webrtc: Webrtc,
        tx: Sender<Stanza>,
        rx: Receiver<Stanza>,
        room_manager: Arc<Mutex<RoomManager>>,
    ) -> impl FnMut(&libstrophe::Context<'_, '_>, &mut Connection<'_, '_>, ConnectionEvent<'_, '_>)
    + Send
    + 'static {
        let rx_shared = Arc::new(Mutex::new(rx));
        let webrtc = Arc::new(webrtc);
        move |ctx, conn, evt| match evt {
            ConnectionEvent::Connect => {
                info!("XMPP connected");

                let rx_clone = Arc::clone(&rx_shared);
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

                conn.handler_add(Self::handle_message(), None, Some("presence"), None);
                conn.handler_add(
                    Self::handle_iq(room_manager.clone(), tx.clone(), webrtc.clone()),
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

    fn handle_iq(
        room_manager: Arc<Mutex<RoomManager>>,
        tx: Sender<Stanza>,
        webrtc: Arc<Webrtc>,
    ) -> impl FnMut(&Context, &mut Connection, &Stanza) -> HandlerResult {
        move |_ctx: &Context, _conn: &mut Connection, stanza: &Stanza| {
            debug!("iq stanza received: {}", stanza.to_string());
            let mut iq = Iq::new(stanza);

            if let Some(child) = stanza.get_first_child() {
                match child.name() {
                    Some("jingle") => {
                        let room_name = iq.from.split('@').next().unwrap_or_default();

                        let room = Room::new(
                            room_name.to_string(),
                            tx.clone(),
                            &webrtc,
                            iq.clone(),
                            &child,
                        );
                        if let Ok(room) = room {
                            match room_manager.lock() {
                                Ok(mut room_manager) => {
                                    room_manager.insert(room);
                                }
                                Err(err) => {
                                    error!("failed to get mutext guard lock for jingle: {err:?}");
                                }
                            }
                            iq.handle_jingle(&child, room_manager.clone());
                        }
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

    fn handle_message() -> impl FnMut(&Context, &mut Connection, &Stanza) -> HandlerResult {
        move |_ctx: &Context, _conn: &mut Connection, stanza: &Stanza| {
            debug!("message stanza received: {}", stanza.to_string());
            HandlerResult::KeepHandler
        }
    }

    pub fn xmpp_connect(
        config: &ConfigSettings,
        room_manager: Arc<Mutex<RoomManager>>,
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

    pub fn xmpp_run(&mut self) {
        self.xmpp_context.run();
    }

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
