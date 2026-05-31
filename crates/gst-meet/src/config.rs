use std::fmt::Display;

use config::{Config, ConfigError};
use env_logger::Builder;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[allow(unused)]
pub struct ConfigSettings {
    pub debug: bool,
    pub name: String,
    pub server: ServerConfig,
    pub xmpp_client: XmppClient,
    pub webrtc: Webrtc,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(unused)]
pub struct ServerConfig {
    pub ip: String,
    pub port: String,
    pub start_pattern_trim: String,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(unused)]
pub struct Webrtc {
    pub stun_server: String,
    pub bundle_policy: String,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(unused)]
pub struct XmppClient {
    pub bot_jid: String,
    pub bot_password: String,
    pub domain_url: String,
    pub domain_port: u16,
}

#[derive(Debug)]
enum LogLevel {
    Info,
    Debug,
}

impl ConfigSettings {
    pub fn new(config: Config) -> Result<Self, ConfigError> {
        config.try_deserialize()
    }

    pub fn logger_init(&self) {
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
