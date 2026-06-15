use log::error;
use serde::Serialize;
use std::{
    collections::HashMap,
    fmt::Display,
    sync::{Mutex, mpsc::Sender},
    time::Instant,
};

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

#[derive(Debug)]
pub(crate) struct TimelineHandler {
    tx: Sender<TimelineEvent>,
    start_instant: Instant,
    pub ssrc_map: Mutex<HashMap<u32, (String, bool, bool)>>,
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

    pub fn register_ssrc(&self, ssrc: u32, endpoint_id: &str, source_name: &str) {
        let is_screenshare = source_name.ends_with("-v1");
        let is_audio = source_name.ends_with("-a0");
        self.ssrc_map
            .lock()
            .unwrap()
            .insert(ssrc, (endpoint_id.to_string(), is_screenshare, is_audio));
    }

<<<<<<< HEAD
=======
    pub fn printout(&self) {
        let ssrc_map = self.ssrc_map.lock().unwrap();
        error!("ssrc_map: {:?}", ssrc_map);
    }

>>>>>>> 22da2cd502ab09519f587700931944b42d3ef299
    pub fn endpoint_for_ssrc(&self, ssrc: u32) -> Option<String> {
        self.ssrc_map
            .lock()
            .unwrap()
            .get(&ssrc)
            .map(|(ep, _, _)| ep.clone())
    }

    pub fn is_screenshare_ssrc(&self, ssrc: u32) -> bool {
        self.ssrc_map
            .lock()
            .unwrap()
            .get(&ssrc)
            .map(|(_, is_share, _)| *is_share)
            .unwrap_or(false)
    }

    pub fn is_audio_ssrc(&self, ssrc: u32) -> bool {
        self.ssrc_map
            .lock()
            .unwrap()
            .get(&ssrc)
            .map(|(_, _, is_audio)| *is_audio)
            .unwrap_or(false)
    }
}
