//! The room's end of the timeline channel: one method per recordable event.

use serde::Serialize;
use std::{
    fmt::Display,
    sync::{
        Mutex,
        mpsc::{Receiver, Sender},
    },
    time::{Duration, Instant},
};

/// Something worth recording about the meeting.
///
/// `timestamp_ms` is always relative to the room's start, and `endpoint` is
/// `None` for meeting-wide events. [`Display`] yields the string written into
/// `timeline.json`.
#[derive(Debug, Serialize)]
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

/// Non-blocking sink for timeline events, held by the room.
///
/// Every method stamps the current offset and posts to the collector thread;
/// send failures are ignored, since a dead collector must not take the
/// recording down with it. The `Mutex` around the receiver exists only to make
/// the handler `Sync` — the room is shared across the XMPP, GStreamer and bus
/// threads.
#[derive(Debug)]
pub(crate) struct TimelineHandler {
    tx: Sender<TimelineEvent>,
    start_instant: Instant,
    files_written_rx: Mutex<Receiver<()>>,
}

impl TimelineHandler {
    pub fn new(
        tx: Sender<TimelineEvent>,
        instant: Instant,
        files_written_rx: Receiver<()>,
    ) -> Self {
        Self {
            tx,
            start_instant: instant,
            files_written_rx: Mutex::new(files_written_rx),
        }
    }

    /// Blocks until the timeline thread has written timeline.json and
    /// metadata.json after meeting_ended(), or the timeout passes.
    pub fn wait_for_files(&self, timeout: Duration) -> bool {
        self.files_written_rx
            .lock()
            .unwrap()
            .recv_timeout(timeout)
            .is_ok()
    }

    /// Milliseconds since the room started.
    fn get_relative_ms(&self) -> u128 {
        self.start_instant.elapsed().as_millis()
    }

    pub fn meeting_started(&self) {
        let _ = self.tx.send(TimelineEvent::MeetingStart {
            timestamp_ms: self.get_relative_ms(),
            endpoint: None,
        });
    }

    /// Records a join, plus a camera/audio "on" event at the same instant for
    /// whichever devices are already live — so the render pass sees an
    /// explicit start for every interval rather than inferring one.
    pub fn participant_joined(
        &self,
        endpoint: Option<String>,
        nickname: String,
        video_muted: bool,
        audio_muted: bool,
        is_empty: bool,
    ) {
        let timestamp = if is_empty { 0 } else { self.get_relative_ms() };
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

    /// Records a dominant-speaker change, as reported by the JVB over the
    /// Colibri data channel.
    pub fn dominant_change(&self, endpoint: Option<String>) {
        let _ = self.tx.send(TimelineEvent::Dominant {
            timestamp_ms: self.get_relative_ms(),
            endpoint,
        });
    }

    /// Final event: tells the collector thread to write its files and exit.
    pub fn meeting_ended(&self) {
        let _ = self.tx.send(TimelineEvent::MeetingEnd {
            timestamp_ms: self.get_relative_ms(),
            endpoint: None,
        });
    }
}
