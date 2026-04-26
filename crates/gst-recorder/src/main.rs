use config::{Config, ConfigError};
use env_logger::Builder;
use gst_meet::app::App;
use libstrophe::Connection;
use log::{error, info};
use serde::Deserialize;
use std::{env, fmt::Display};
use tiny_http::Server;

fn main() {
    let config = Settings::new().unwrap();
    config.logger_init();

    let xmpp_connection = init_xmpp_connection(&config);

    let ip = &config.server.ip;
    let port = &config.server.port;
    let server = Server::http(&format!("{ip}:{port}"));
    match server {
        Ok(server) => {
            info!("started listening on: {:?}", server.server_addr());
            let app = App::new(xmpp_connection);
            for request in server.incoming_requests() {
                let room = request
                    .url()
                    .trim_start_matches(config.server.start_pattern_trim.as_str());
            }
        }
        Err(err) => {
            error!("Error starting server: {:?}", err);
        }
    }
}

fn init_xmpp_connection(settings: &Settings) -> Connection<'static, 'static> {
    let ctx = libstrophe::Context::new_with_default_logger();
    let mut conn = libstrophe::Connection::new(ctx);
    conn.set_jid(&settings.xmpp_clinet.bot_jid);
    conn.set_pass(&settings.xmpp_clinet.bot_password);
    return conn;
}

#[derive(Debug, Clone, Deserialize)]
#[allow(unused)]
pub(crate) struct Settings {
    pub debug: bool,
    pub name: String,
    pub server: ServerConfig,
    pub xmpp_clinet: XmppClient,
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
    domain_port: String,
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
