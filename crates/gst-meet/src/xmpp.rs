use libstrophe::{ConnectClientError, Connection, ConnectionEvent, ConnectionFlags, Context};
use log::{error, info};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("failed to initialize xmpp: {0}")]
    InitializationError(libstrophe::Error),

    #[error("failed to connect: {0:?}")]
    ConnectClientError(ConnectClientError<'static, 'static>),

    #[error("failed to connect: something went wrong")]
    Unkown,
}

pub struct App {
    pub xmpp_context: Context<'static, 'static>,
}

impl App {
    fn new(context: Context<'static, 'static>) -> Self {
        App {
            xmpp_context: context,
        }
    }

    pub fn xmpp_connect(
        host: &str,
        port: u16,
        jid: &str,
        password: &str,
    ) -> Result<Self, AppError> {
        let conn = Self::init_xmpp_connection(jid, password)?;
        let ctx = conn.connect_client(Some(host), Some(port), Self::xmpp_connection_handler());

        match ctx {
            Ok(ctx) => Ok(Self::new(ctx)),
            Err(err) => Err(AppError::ConnectClientError(err)),
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

    fn xmpp_connection_handler()
    -> impl FnMut(&libstrophe::Context<'_, '_>, &mut Connection<'_, '_>, ConnectionEvent<'_, '_>)
    + Send
    + 'static {
        move |ctx, _conn, evt| match evt {
            ConnectionEvent::Connect => {
                info!("XMPP connected");
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
}
