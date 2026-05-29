use std::collections::HashMap;

use libstrophe::Stanza;

use crate::{get_attribute, util::find_first};

#[derive(Debug, Clone)]
pub enum PresenceLifecycle {
    ParticipantJoined(ParticipantPresence),
    ParticipantLeft(ParticipantPresence),
    MeetingTerminated,
}

#[derive(Debug, Clone)]
pub struct ParticipantPresence {
    pub endpoint_id: String,
    pub display_name: Option<String>,
    pub real_jid: Option<String>,
    pub video_muted: bool,
    pub audio_muted: bool,
    pub is_screen_share: bool,
    pub from: String,
}

impl ParticipantPresence {
    pub fn from_presence(stanza: &Stanza) -> Option<PresenceLifecycle> {
        let presence_stanza = get_attribute!(stanza, {
            name => "name",
            from => "from",
            kind => "kind"
        });

        let endpoint_id = presence_stanza.from.rsplit('/').next()?.to_string();
        let display_name = find_first(Some(&stanza), "nick")?.text();

        let item_stanza = find_first(Some(&stanza), "x>item")?;

        let real_jid = get_attribute!(item_stanza, [jid]).jid;

        let real_jid = if real_jid.is_empty() {
            None
        } else {
            Some(real_jid)
        };

        let (video_muted, audio_muted, is_screen_share) =
            Self::parse_source_info(stanza, &endpoint_id);

        let participant = Self {
            endpoint_id,
            display_name,
            real_jid,
            from: presence_stanza.from,
            video_muted,
            audio_muted,
            is_screen_share,
        };

        if presence_stanza.kind.is_empty() {
            return Some(PresenceLifecycle::ParticipantJoined(participant));
        } else {
            let destroy_stanza = find_first(Some(&stanza), "x>destroy");

            match destroy_stanza {
                Some(_) => return Some(PresenceLifecycle::MeetingTerminated),
                None => return Some(PresenceLifecycle::ParticipantLeft(participant)),
            }
        }
    }

    fn parse_source_info(stanza: &Stanza, endpoint_id: &str) -> (bool, bool, bool) {
        let source_info_text = match find_first(Some(stanza), "SourceInfo").and_then(|n| n.text()) {
            Some(t) => t,
            None => return (false, false, false),
        };

        let video_key = format!("{}-v0", endpoint_id);
        let screen_key = format!("{}-v1", endpoint_id);

        let map: HashMap<String, serde_json::Value> = match serde_json::from_str(&source_info_text)
        {
            Ok(m) => m,
            Err(_) => return (false, false, false),
        };

        let video_muted = map
            .get(&video_key)
            .and_then(|v| v.get("muted"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let is_screen_share = map.contains_key(&screen_key);

        let audio_muted = find_first(Some(stanza), "audiomuted")
            .and_then(|n| n.text())
            .map(|t| t == "true")
            .unwrap_or(false);

        (video_muted, audio_muted, is_screen_share)
    }
}
