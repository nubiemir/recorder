use crate::{
    e2ee::olm_adapter::OlmAdapter, make_stanza, util::username_generator::generate_username,
};
use libstrophe::Stanza;
use nanoid::nanoid;
use thiserror::Error;

pub mod participant_presence;

#[derive(Error, Debug)]
pub enum PresenceError {
    #[error("failed to parse stanza err: {0}")]
    ParseError(#[from] libstrophe::Error),
}

#[derive(Debug)]
#[allow(unused)]
pub struct Presence {
    stats_id: String,
    curve25519: String,
    ed25519: String,
    codec_list: [&'static str; 4],
    e2ee_enabled: bool,
}

impl Presence {
    fn new() -> Self {
        let olm_adapter = OlmAdapter::new();
        let id_keys = olm_adapter.get_id_keys();
        Self {
            stats_id: generate_username(),
            curve25519: id_keys.curve25519.to_string(),
            ed25519: id_keys.ed25519.to_string(),
            codec_list: ["h264", "vp8", "vp9", "av1"],
            e2ee_enabled: false,
        }
    }

    pub fn handle_join_recorder(room: &str) -> Result<(Stanza, Presence), PresenceError> {
        let recorder_presence = Self::new();

        let stats = make_stanza!("stats-id", {}, text: &recorder_presence.stats_id)?;

        let codecs = recorder_presence.codec_list.join(",");
        let codec_list = make_stanza!("jitsi_participant_codecList", {}, text: codecs)?;

        let curve25519 = make_stanza!(
            "jitsi_participant_e2ee.idKey.curve25519",
            {},
            text: &recorder_presence.curve25519
        )?;

        let ed25519 = make_stanza!(
            "jitsi_participant_e2ee.idKey.ed25519",
            {},
            text: &recorder_presence.ed25519
        )?;

        let x = make_stanza!("x", {
            "xmlns" => "http://jabber.org/protocol/muc"
        })?;

        let presence = make_stanza!("presence", {
            "to" => format!("{}@muc.meet.jitsi/{}", room, nanoid!(10))
        }, [stats, codec_list, curve25519, ed25519, x])?;

        Ok((presence, recorder_presence))
    }
}
