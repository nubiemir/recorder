//! The record of *what happened* in a meeting, next to the recordings of it.
//!
//! Media files say nothing about when someone joined, muted, or was the
//! dominant speaker, which is exactly what a later render pass needs to lay
//! out a composite video. The room reports events through
//! `timeline_handler::TimelineHandler`; a background engine
//! (`timeline_engine::TimelineEngine`) collects them and, at meeting end,
//! writes `timeline.json` and `metadata.json` into the room's output
//! directory.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

pub(crate) mod timeline_engine;
pub(crate) mod timeline_handler;
pub(crate) mod timeline_process;

/// Recorded file names for one participant's streams, as written into
/// `metadata.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(unused)]
pub struct Media {
    #[serde(skip_serializing_if = "Option::is_none")]
    audio: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    camera: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    screenshare: Option<String>,
}

/// A participant as described in `metadata.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(unused)]
pub struct Participant {
    name: String,
    media: Media,
}

/// One entry in `timeline.json`: what happened, to whom, and how many
/// milliseconds into the meeting.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(unused)]
pub struct Timeline {
    timestamp: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    participant_id: Option<String>,
    event: String,
}

/// Meeting-level summary written to `metadata.json` once recording ends.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(unused)]
pub struct Metadata {
    room_id: String,
    room_name: String,
    start_timestamp: u128,
    end_timestamp: u128,
    output_dir: String,
    total_duration_ms: u128,
    participants: HashMap<String, Participant>,
}
