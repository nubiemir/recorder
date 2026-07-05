use std::collections::HashMap;

use serde::{Deserialize, Serialize};

pub(crate) mod timeline_engine;
pub(crate) mod timeline_handler;
pub(crate) mod timeline_process;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(unused)]
pub struct Participant {
    name: String,
    media: Media,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(unused)]
pub struct Timeline {
    timestamp: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    participant_id: Option<String>,
    event: String,
}

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
