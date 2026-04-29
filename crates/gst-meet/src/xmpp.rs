use std::{
    sync::{
        Arc, Mutex,
        mpsc::{Receiver, Sender},
    },
    time::Duration,
};

use libstrophe::{
    ConnectClientError, Connection, ConnectionEvent, ConnectionFlags, Context, HandlerResult,
    Stanza,
};
use log::{debug, error, info};
use thiserror::Error;

use crate::iq::Iq;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("failed to initialize xmpp: {0}")]
    InitializationError(libstrophe::Error),

    #[error("failed to connect: {0:?}")]
    ConnectClientError(ConnectClientError<'static, 'static>),

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
        rx: Receiver<Stanza>,
    ) -> impl FnMut(&libstrophe::Context<'_, '_>, &mut Connection<'_, '_>, ConnectionEvent<'_, '_>)
    + Send
    + 'static {
        let rx_shared = Arc::new(Mutex::new(rx));
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
                conn.handler_add(Self::handle_iq(), None, Some("iq"), None);
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

    fn handle_iq() -> impl FnMut(&Context, &mut Connection, &Stanza) -> HandlerResult {
        move |_ctx: &Context, _conn: &mut Connection, stanza: &Stanza| {
            debug!("iq stanza received: {}", stanza.to_string());
            let mut iq = Iq::new(stanza);

            if let Some(child) = stanza.get_first_child() {
                match child.name() {
                    Some("jingle") => {
                        iq.handle_jingle(&child);
                    }
                    Some("query") => {
                        iq.handle_query(&child);
                    }
                    _ => {}
                }
            }

            HandlerResult::KeepHandler
        }
    }

    fn handle_message() -> impl FnMut(&Context, &mut Connection, &Stanza) -> HandlerResult {
        move |_ctx: &Context, _conn: &mut Connection, stanza: &Stanza| {
            info!("message stanza received: {}", stanza.to_string());
            HandlerResult::KeepHandler
        }
    }

    pub fn xmpp_connect(
        host: &str,
        port: u16,
        jid: &str,
        password: &str,
        tx: Sender<Stanza>,
        rx: Receiver<Stanza>,
    ) -> Result<Self, AppError> {
        let conn = Self::init_xmpp_connection(jid, password)?;
        let ctx = conn.connect_client(Some(host), Some(port), Self::xmpp_connection_handler(rx));

        match ctx {
            Ok(ctx) => Ok(Self::new(ctx, tx)),
            Err(err) => Err(AppError::ConnectClientError(err)),
        }
    }

    pub fn xmpp_run(&mut self) {
        self.xmpp_context.run();
    }
}
