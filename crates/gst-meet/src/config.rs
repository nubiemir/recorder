//! Deserialized form of the recorder's TOML configuration.

use std::fmt::Display;

use config::{Config, ConfigError};
use env_logger::Builder;
use serde::Deserialize;

/// Top-level configuration, mirroring the layout of `config/default.toml`.
#[derive(Debug, Clone, Deserialize)]
#[allow(unused)]
pub struct ConfigSettings {
    /// Raises the log filter to `debug`.
    pub debug: bool,
    /// Name the recorder reports for itself.
    pub name: String,
    pub server: ServerConfig,
    pub xmpp_client: XmppClient,
    pub webrtc: Webrtc,
}

/// Where the recorder listens for commands telling it to join a meeting.
#[derive(Debug, Clone, Deserialize)]
#[allow(unused)]
pub struct ServerConfig {
    pub ip: String,
    pub port: String,
    /// Prefix stripped off an incoming start command before the remainder is
    /// treated as a room name.
    pub start_pattern_trim: String,
}

/// Settings applied to `webrtcbin` when a room's pipeline is built.
#[derive(Debug, Clone, Deserialize)]
#[allow(unused)]
pub struct Webrtc {
    pub stun_server: String,
    /// `bundle-policy` value, e.g. `max-bundle`; the JVB expects a single
    /// bundled transport.
    pub bundle_policy: String,
}

/// Credentials and address of the XMPP server the recorder signs in to.
#[derive(Debug, Clone, Deserialize)]
#[allow(unused)]
pub struct XmppClient {
    pub bot_jid: String,
    pub bot_password: String,
    pub domain_url: String,
    pub domain_port: u16,
}

/// Log filter passed to `env_logger`.
#[derive(Debug)]
enum LogLevel {
    Info,
    Debug,
}

impl ConfigSettings {
    /// Deserializes a loaded [`Config`] into these settings.
    pub fn new(config: Config) -> Result<Self, ConfigError> {
        config.try_deserialize()
    }

    /// Installs the global logger at `debug` or `info` depending on the
    /// `debug` setting. Call once at startup; `env_logger` panics if a logger
    /// is already set.
    pub fn logger_init(&self) {
        let mut builder = Builder::new();
        if self.debug {
            builder.parse_filters(&LogLevel::Debug.to_string());
        } else {
            builder.parse_filters(&LogLevel::Info.to_string());
        }
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
