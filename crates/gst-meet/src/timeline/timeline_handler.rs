use log::{info, warn};
use serde::Serialize;
use std::{
    collections::HashMap,
    fmt::Display,
    sync::{Mutex, mpsc::Sender},
    time::Instant,
};

use crate::iq::jingle_action::ParsedSource;

#[derive(Debug, Serialize)]
#[serde(tag = "eventType")]
pub enum TimelineEvent {
    MeetingStart {
        timestamp_ms: u128,
        endpoint: Option<String>,
    },
    Joined {
        timestamp_ms: u128,
        endpoint: Option<String>,
        nickname: String,
    },
    Left {
        timestamp_ms: u128,
        endpoint: Option<String>,
    },
    CameraOn {
        timestamp_ms: u128,
        endpoint: Option<String>,
    },
    CameraOff {
        timestamp_ms: u128,
        endpoint: Option<String>,
    },
    ScreenshareOn {
        timestamp_ms: u128,
        endpoint: Option<String>,
    },
    ScreenshareOff {
        timestamp_ms: u128,
        endpoint: Option<String>,
    },
    AudioOn {
        timestamp_ms: u128,
        endpoint: Option<String>,
    },
    AudioOff {
        timestamp_ms: u128,
        endpoint: Option<String>,
    },
    Dominant {
        timestamp_ms: u128,
        endpoint: Option<String>,
    },
    MeetingEnd {
        timestamp_ms: u128,
        endpoint: Option<String>,
    },
}

impl Display for TimelineEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MeetingStart { .. } => write!(f, "meeting_started"),
            Self::Joined { .. } => write!(f, "joined"),
            Self::Left { .. } => write!(f, "left"),
            Self::CameraOn { .. } => write!(f, "camera_on"),
            Self::CameraOff { .. } => write!(f, "camera_off"),
            Self::ScreenshareOn { .. } => write!(f, "screenshare_on"),
            Self::ScreenshareOff { .. } => write!(f, "screenshare_off"),
            Self::AudioOn { .. } => write!(f, "audio_on"),
            Self::AudioOff { .. } => write!(f, "audio_off"),
            Self::Dominant { .. } => write!(f, "dominant"),
            Self::MeetingEnd { .. } => write!(f, "meeting_ended"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceKind {
    CameraVideo,
    Audio,
    ScreenshareVideo,
}

impl Display for SourceKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CameraVideo => write!(f, "camera_video"),
            Self::Audio => write!(f, "audio"),
            Self::ScreenshareVideo => write!(f, "screenshare_video"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SourceEntry {
    pub endpoint: String,
    pub kind: SourceKind,
}

#[derive(Debug)]
pub(crate) struct TimelineHandler {
    tx: Sender<TimelineEvent>,
    start_instant: Instant,
    pub ssrc_map: Mutex<HashMap<u32, SourceEntry>>,
}

impl TimelineHandler {
    pub fn new(tx: Sender<TimelineEvent>, instant: Instant) -> Self {
        Self {
            tx,
            start_instant: instant,
            ssrc_map: Mutex::new(HashMap::new()),
        }
    }

    fn get_relative_ms(&self) -> u128 {
        self.start_instant.elapsed().as_millis()
    }

    pub fn meeting_started(&self) {
        let _ = self.tx.send(TimelineEvent::MeetingStart {
            timestamp_ms: self.get_relative_ms(),
            endpoint: None,
        });
    }

    pub fn participant_joined(
        &self,
        endpoint: Option<String>,
        nickname: String,
        video_muted: bool,
        audio_muted: bool,
    ) {
        let timestamp = self.get_relative_ms();
        let _ = self.tx.send(TimelineEvent::Joined {
            timestamp_ms: timestamp,
            endpoint: endpoint.clone(),
            nickname,
        });

        if !video_muted {
            let _ = self.tx.send(TimelineEvent::CameraOn {
                timestamp_ms: timestamp,
                endpoint: endpoint.clone(),
            });
        }

        if !audio_muted {
            let _ = self.tx.send(TimelineEvent::AudioOn {
                timestamp_ms: timestamp,
                endpoint: endpoint.clone(),
            });
        }
    }

    pub fn participant_left(&self, endpoint: Option<String>) {
        let _ = self.tx.send(TimelineEvent::Left {
            timestamp_ms: self.get_relative_ms(),
            endpoint,
        });
    }

    pub fn camera_on(&self, endpoint: Option<String>) {
        let _ = self.tx.send(TimelineEvent::CameraOn {
            timestamp_ms: self.get_relative_ms(),
            endpoint,
        });
    }

    pub fn camera_off(&self, endpoint: Option<String>) {
        let _ = self.tx.send(TimelineEvent::CameraOff {
            timestamp_ms: self.get_relative_ms(),
            endpoint,
        });
    }

    pub fn screenshare_on(&self, endpoint: Option<String>) {
        let _ = self.tx.send(TimelineEvent::ScreenshareOn {
            timestamp_ms: self.get_relative_ms(),
            endpoint,
        });
    }

    pub fn screenshare_off(&self, endpoint: Option<String>) {
        let _ = self.tx.send(TimelineEvent::ScreenshareOff {
            timestamp_ms: self.get_relative_ms(),
            endpoint,
        });
    }

    pub fn audio_on(&self, endpoint: Option<String>) {
        let _ = self.tx.send(TimelineEvent::AudioOn {
            timestamp_ms: self.get_relative_ms(),
            endpoint,
        });
    }

    pub fn audio_off(&self, endpoint: Option<String>) {
        let _ = self.tx.send(TimelineEvent::AudioOff {
            timestamp_ms: self.get_relative_ms(),
            endpoint,
        });
    }

    pub fn dominant_change(&self, endpoint: Option<String>) {
        let _ = self.tx.send(TimelineEvent::Dominant {
            timestamp_ms: self.get_relative_ms(),
            endpoint,
        });
    }

    pub fn meeting_ended(&self) {
        let _ = self.tx.send(TimelineEvent::MeetingEnd {
            timestamp_ms: self.get_relative_ms(),
            endpoint: None,
        });
    }

    /// "c8ef68f5-v1" -> ('v', 1), "eee01355-a0" -> ('a', 0)
    fn parse_source_name(&self, name: &str) -> Option<(char, u32)> {
        let suffix = name.rsplit('-').next()?; // "v1"
        let mut chars = suffix.chars();
        let letter = chars.next()?; // 'v' or 'a'
        let idx: u32 = chars.as_str().parse().ok()?; // 0, 1, ...
        Some((letter, idx))
    }

    pub fn register_ssrc(&self, parsed_source: ParsedSource) {
        let (letter, index) = match self.parse_source_name(&parsed_source.source_name) {
            Some(v) => v,
            None => {
                warn!(
                    "register_ssrc: unparseable source name {}, skipping",
                    parsed_source.source_name
                );
                return;
            }
        };

        let kind = match letter {
            'v' => {
                if parsed_source.video_type == Some("d".to_string()) {
                    SourceKind::ScreenshareVideo
                } else {
                    SourceKind::CameraVideo
                }
            }
            'a' => SourceKind::Audio,
            _ => {
                warn!("unexpected source letter in {}", parsed_source.source_name);
                return;
            }
        };

        info!(
            "register_ssrc: ssrc={} endpoint={} name={} index={} kind={:?}",
            parsed_source.ssrc, parsed_source.endpoint_id, parsed_source.source_name, index, kind
        );

        self.ssrc_map.lock().unwrap().insert(
            parsed_source.ssrc,
            SourceEntry {
                endpoint: parsed_source.endpoint_id,
                kind,
            },
        );
    }

    pub fn endpoint_for_ssrc(&self, ssrc: u32) -> Option<SourceEntry> {
        self.ssrc_map.lock().unwrap().get(&ssrc).cloned()
    }

    // pub fn is_audio_ssrc(&self, ssrc: u32) -> bool {
    //     self.ssrc_map
    //         .lock()
    //         .unwrap()
    //         .get(&ssrc)
    //         .map(|e| e.kind)
    //         .unwrap_or(false)
    // }
    // pub fn is_screenshare_ssrc(&self, ssrc: u32) -> bool {
    //     self.ssrc_map
    //         .lock()
    //         .unwrap()
    //         .get(&ssrc)
    //         .map(|e| e.is_screenshare)
    //         .unwrap_or(false)
    // }
}
