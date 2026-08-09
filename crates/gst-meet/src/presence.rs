//! Parsing of MUC presence stanzas into room lifecycle events.
//!
//! Jitsi carries mute state in a `<SourceInfo>` child of presence, so the same
//! stanza type reports both "who is here" and "what is muted"; a participant
//! already in the room simply sends presence again when they toggle a device.

use std::collections::HashMap;

use libstrophe::Stanza;

use crate::{get_attribute, util::find_first};

/// What a presence stanza means for the room.
#[derive(Debug, Clone)]
pub enum PresenceLifecycle {
    /// Available presence: either a new participant, or an update from one
    /// already in the room (the caller distinguishes the two).
    ParticipantJoined(ParticipantPresence),
    /// Unavailable presence: the participant left.
    ParticipantLeft(ParticipantPresence),
    /// Unavailable presence carrying `<destroy>`: the room itself is gone, so
    /// the recording must finish.
    MeetingTerminated(ParticipantPresence),
}

/// One participant's state as of a single presence stanza.
#[derive(Debug, Clone)]
pub struct ParticipantPresence {
    /// MUC resource part, the id Jitsi uses everywhere else for this person.
    pub endpoint_id: String,
    /// `<nick>` text; only present on available presence.
    pub display_name: Option<String>,
    /// Real JID behind the MUC nickname, when the room is non-anonymous.
    pub real_jid: Option<String>,
    pub video_muted: bool,
    pub audio_muted: bool,
    pub screenshare_muted: bool,
    /// Full `room@conference.domain/endpoint` JID the stanza came from.
    pub from: String,
}

impl ParticipantPresence {
    /// Classifies a presence stanza, or returns `None` if it isn't MUC presence
    /// we can act on (no `<x><item>`, or available presence with no `<nick>`).
    pub fn from_presence(stanza: &Stanza) -> Option<PresenceLifecycle> {
        let presence_stanza = get_attribute!(stanza, {
            name => "name",
            from => "from",
            kind => "type"
        });

        let endpoint_id = presence_stanza.from.rsplit('/').next()?.to_string();

        let item_stanza = find_first(Some(&stanza), "x>item")?;

        let real_jid = get_attribute!(item_stanza, [jid]).jid;

        let real_jid = if real_jid.is_empty() {
            None
        } else {
            Some(real_jid)
        };

        let (video_muted, audio_muted, screenshare_muted) =
            Self::parse_source_info(stanza, &endpoint_id);

        let mut participant = Self {
            endpoint_id,
            display_name: None,
            real_jid,
            from: presence_stanza.from,
            video_muted,
            audio_muted,
            screenshare_muted,
        };

        if presence_stanza.kind.is_empty() {
            let display_name = find_first(Some(&stanza), "nick")?.text();
            participant.display_name = display_name;
            return Some(PresenceLifecycle::ParticipantJoined(participant));
        } else {
            let destroy_stanza = find_first(Some(&stanza), "x>destroy");

            match destroy_stanza {
                Some(_) => return Some(PresenceLifecycle::MeetingTerminated(participant)),
                None => return Some(PresenceLifecycle::ParticipantLeft(participant)),
            }
        }
    }

    /// Reads mute flags out of the `<SourceInfo>` JSON blob, returning
    /// `(video_muted, audio_muted, screenshare_muted)`.
    ///
    /// Source names follow Jitsi's `<endpoint>-<kind><index>` convention:
    /// `-v0` camera, `-a0` microphone, `-v1` screenshare. Anything missing or
    /// unparseable is treated as muted, so a stream we can't reason about is
    /// never assumed to be live.
    fn parse_source_info(stanza: &Stanza, endpoint_id: &str) -> (bool, bool, bool) {
        let source_info_text = match find_first(Some(stanza), "SourceInfo").and_then(|n| n.text()) {
            Some(t) => t,
            None => return (true, true, false),
        };

        let video_key = format!("{}-v0", endpoint_id);
        let audio_key = format!("{}-a0", endpoint_id);
        let screen_key = format!("{}-v1", endpoint_id);

        let map: HashMap<String, serde_json::Value> = match serde_json::from_str(&source_info_text)
        {
            Ok(m) => m,
            Err(_) => return (true, true, false),
        };

        let video_muted = map
            .get(&video_key)
            .and_then(|v| v.get("muted"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let screenshare_muted = map
            .get(&screen_key)
            .and_then(|v| v.get("muted"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let audio_muted = map
            .get(&audio_key)
            .and_then(|v| v.get("muted"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        (video_muted, audio_muted, screenshare_muted)
    }
}
