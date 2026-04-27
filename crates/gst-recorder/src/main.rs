use config::{Config, ConfigError};
use env_logger::Builder;
use gst_meet::xmpp::App;
use log::{error, info};
use serde::Deserialize;
use std::{env, fmt::Display, sync::Arc, thread};
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
            let app = App::xmpp_connect(
                &config.xmpp_client.domain_url,
                config.xmpp_client.domain_port,
                &config.xmpp_client.bot_jid,
                &config.xmpp_client.bot_password,
            );

            match app {
                Ok(mut app) => {
                    app.xmpp_context.run();
                    for request in server.incoming_requests() {
                        let config = Arc::clone(&config);
                        thread::spawn(move || {
                            handle_request(request, config);
                        });
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

fn handle_request(request: Request, config: Arc<Settings>) {
    let _room = request
        .url()
        .trim_start_matches(config.server.start_pattern_trim.as_str());
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
