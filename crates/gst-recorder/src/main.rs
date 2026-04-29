use config::{Config, ConfigError};
use env_logger::Builder;
use gst_meet::{make_stanza, xmpp::App};
use libstrophe::Stanza;
use log::{debug, error, info};
use serde::Deserialize;
use std::{
    env,
    fmt::Display,
    process::exit,
    sync::{
        Arc,
        mpsc::{SendError, Sender, channel},
    },
    thread,
};
use thiserror::Error;
use tiny_http::{Request, Server};

fn main() {
    let config = Arc::new(Settings::new().unwrap());
    config.logger_init();

    let ip = &config.server.ip;
    let port = &config.server.port;
    let server = Server::http(&format!("{ip}:{port}"));
    match server {
        Ok(server) => {
            info!("started listening on: {:?}", server.server_addr());
            let (tx, rx) = channel::<Stanza>();

            let app = App::xmpp_connect(
                &config.xmpp_client.domain_url,
                config.xmpp_client.domain_port,
                &config.xmpp_client.bot_jid,
                &config.xmpp_client.bot_password,
                tx.clone(),
                rx,
            );

            match app {
                Ok(mut app) => {
                    let xmpp_handle = thread::spawn(move || {
                        app.xmpp_run();
                    });

                    for request in server.incoming_requests() {
                        let config = Arc::clone(&config);
                        let tx = tx.clone();
                        thread::spawn(move || match handle_request(request, config, tx) {
                            Ok(room) => {
                                info!("sent presence for: {room} room");
                            }
                            Err(err) => {
                                error!("failed to send presence for: {err:?} room");
                            }
                        });
                    }

                    if let Err(err) = xmpp_handle.join() {
                        error!("{err:?}");
                        exit(1);
                    }
                }
                Err(err) => {
                    error!("failed connecting to xmpp: {:?}", err);
                }
            }
        }
        Err(err) => {
            error!("error starting server: {:?}", err);
        }
    }
}

#[derive(Error, Debug)]
enum RequestError {
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
}

fn handle_request(
    request: Request,
    config: Arc<Settings>,
    tx: Sender<Stanza>,
) -> Result<String, RequestError> {
    let room = request
        .url()
        .trim_start_matches(config.server.start_pattern_trim.as_str());

    debug!("room: {room}");

    let x = make_stanza!("x", {
    "xmlns" => "http://jabber.org/protocol/muc"
}, [])
    .map_err(|e| RequestError::ParseError {
        room: room.to_string(),
        source: e,
    })?;

    let presence = make_stanza!("presence", {
    "to" => format!("{}@muc.meet.jitsi/xxxx", room)
}, [x])
    .map_err(|e| RequestError::ParseError {
        room: room.to_string(),
        source: e,
    })?;

    tx.send(presence).map_err(|e| RequestError::SendError {
        room: room.to_string(),
        source: e,
    })?;

    Ok(room.to_string())
}

#[derive(Debug, Clone, Deserialize)]
#[allow(unused)]
pub(crate) struct Settings {
    pub debug: bool,
    pub name: String,
    pub server: ServerConfig,
    pub xmpp_client: XmppClient,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(unused)]
pub(crate) struct ServerConfig {
    pub ip: String,
    pub port: String,
    pub start_pattern_trim: String,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(unused)]
pub(crate) struct XmppClient {
    bot_jid: String,
    bot_password: String,
    domain_url: String,
    domain_port: u16,
}

#[derive(Debug)]
enum LogLevel {
    Info,
    Debug,
}

impl Settings {
    fn new() -> Result<Self, ConfigError> {
        let run_mode = env::var("RUN_MODE").unwrap_or_else(|_| "development".into());

        let config = Config::builder()
            .add_source(config::File::with_name(
                "crates/gst-recorder/config/default",
            ))
            .add_source(
                config::File::with_name(&format!("crates/gst-recorder/config/{run_mode}"))
                    .required(false),
            )
            .add_source(config::Environment::with_prefix("APP"))
            .build()?;

        config.try_deserialize()
    }

    fn logger_init(&self) {
        let mut builder = Builder::new();
        if self.debug {
            builder.parse_filters(&LogLevel::Debug.to_string());
        }
        builder.parse_filters(&LogLevel::Info.to_string());
        builder.init();
    }
}

impl Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Info => write!(f, "{}", "info"),
            Self::Debug => write!(f, "{}", "debug"),
        }
    }
}
