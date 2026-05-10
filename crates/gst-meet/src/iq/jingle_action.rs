use std::fmt::Display;

use chrono::Utc;
use libstrophe::Stanza;

use crate::{iq::jingle_media::JingleMedia, util::find_all};

#[derive(Debug)]
pub enum JingleAction<'a> {
    SessionInitiate(&'a Stanza),
    SourceAdd(&'a Stanza),
}

impl<'a> JingleAction<'a> {
    pub(crate) fn parse(s: &str, stanza: &'a Stanza) -> Option<Self> {
        match s {
            "session-initiate" => Some(Self::SessionInitiate(stanza)),
            "source-add" => Some(Self::SourceAdd(stanza)),
            _ => None,
        }
    }

    pub fn handle_session_initiate(&self, stanza: &Stanza, media: &mut JingleMedia) -> String {
        let mut sdp_session = self.parse_sdp_session(stanza);
        let sdp_media = media.parse_sdp_media(stanza);
        sdp_session.push_str(&sdp_media);

        sdp_session
    }

    pub fn handle_source_add(&self, _stanza: &Stanza) -> String {
        String::new()
    }

    fn parse_sdp_session(&self, stanza: &Stanza) -> String {
        let mut sdp = String::new();
        let session_id = Utc::now().timestamp_millis();
        sdp.push_str("v=0\r\n");
        sdp.push_str("o=- ");
        sdp.push_str(session_id.to_string().as_str());
        sdp.push_str(" 2 IN IP4 0.0.0.0\r\n");
        sdp.push_str("s=-\r\n");
        sdp.push_str("t=0 0\r\n");

        let fingerprints = find_all(Some(stanza), "content>transport>fingerprint");
        let has_cryptex = fingerprints
            .iter()
            .any(|x| x.get_attribute("cryptex") == Some("true"));

        if has_cryptex {
            sdp.push_str("a=cryptex\r\n");
        }
        sdp
    }
}

impl<'a> Display for JingleAction<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SessionInitiate(_) => write!(f, "session-initiate"),
            Self::SourceAdd(_) => write!(f, "source-add"),
        }
    }
}
