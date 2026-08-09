//! Reads the recorder's `timeline.json` and turns its event log into intervals.
//!
//! The recorder writes discrete events ("camera_on at 12.4s"); layout needs
//! spans ("camera on from 12.4s to 40.1s"). This module pairs on/off events
//! into intervals, closing anything still open at the end of the meeting.

use serde::Deserialize;
use std::{collections::HashMap, fs::File};
use thiserror::Error;

/// Failures while reading or interpreting a `timeline.json`.
#[derive(Debug, Error)]
pub enum TimelineError {
    #[error("io error: {0}")]
    IOError(#[from] std::io::Error),

    #[error("json parsing error: {0}")]
    SerdeJsonError(#[from] serde_json::Error),

    #[error("load timeline error: {0}")]
    LoadTimelineError(&'static str),
}

type Result<T> = std::result::Result<T, TimelineError>;
type Participants = HashMap<String, Participant>;

/// One participant's presence and device intervals, in seconds from the start
/// of the meeting. Each `(start, end)` pair is a span the device was live.
#[derive(Debug, Default)]
pub struct Participant {
    pub joined: f64,
    pub left: f64,
    pub camera: Vec<(f64, f64)>,
    pub share: Vec<(f64, f64)>,
    pub audio: Vec<(f64, f64)>,
}

/// A stretch of time during which one participant was the dominant speaker.
#[derive(Debug)]
pub struct Dominant {
    pub start: f64,
    pub end: f64,
    pub endpoint: String,
    pub nickname: String,
}

/// The whole meeting in interval form: who was present when, and who was
/// speaking.
#[derive(Debug)]
pub struct Timeline {
    pub participants: Participants,
    pub dominant: Vec<Dominant>,
    /// Meeting length in seconds; every interval is clamped to this.
    pub end: f64,
}

/// One entry as it appears in `timeline.json`.
#[derive(Debug, Deserialize)]
pub struct RawEvent {
    pub event: String,
    #[serde(default)]
    pub participant_id: Option<String>,
    pub timestamp: u64,
}

impl Timeline {
    /// Loads and interprets a `timeline.json`.
    pub fn load_timeline(path: &str) -> Result<Timeline> {
        let events = Self::read_file(path)?;
        let end = Self::calculate_end(&events)?;

        let timeline = Self::process_events(events, end);

        Ok(timeline)
    }

    fn read_file(path: &str) -> Result<Vec<RawEvent>> {
        let file = File::open(path)?;
        let events: Vec<RawEvent> = serde_json::from_reader(file)?;
        Ok(events)
    }

    /// Meeting length, taken from the `meeting_ended` event.
    ///
    /// Its absence is an error rather than a fallback to the last event: a
    /// timeline without it came from a recording that never drained, and
    /// rendering it would silently truncate.
    fn calculate_end(events: &Vec<RawEvent>) -> Result<f64> {
        const MEETING_ENDED: &str = "meeting_ended";
        let end = events
            .iter()
            .find(|evt| evt.event == MEETING_ENDED)
            .map(|evt| evt.timestamp as f64 / 1000.0)
            .ok_or_else(|| TimelineError::LoadTimelineError("no meeting_ended event found"))?;

        Ok(end)
    }

    /// Folds the event log into intervals.
    ///
    /// `open_*` maps hold the start of each device span still waiting for its
    /// "off" event; leaving, or the end of the meeting, closes whatever is
    /// still open. Dominant speaker works the same way: each `dominant` event
    /// for a different participant closes the previous speaker's span.
    ///
    /// Events with no participant id accumulate under the empty key, which is
    /// discarded at the end.
    fn process_events(events: Vec<RawEvent>, end: f64) -> Timeline {
        let mut participants: Participants = HashMap::new();

        let mut open_camera: HashMap<String, f64> = HashMap::new();
        let mut open_share: HashMap<String, f64> = HashMap::new();
        let mut open_audio: HashMap<String, f64> = HashMap::new();
        let mut dominant: Vec<Dominant> = Vec::new();
        let mut current_dominant: Option<(f64, String)> = None; // Some(start, who)

        for event in events {
            let time_in_sec = event.timestamp as f64 / 1000.0;
            let pid = event.participant_id.unwrap_or_default();
            let participant = participants.entry(pid.clone()).or_default();

            match event.event.as_str() {
                "joined" => {
                    participant.joined = time_in_sec;
                    if current_dominant.is_none() {
                        current_dominant = Some((time_in_sec, pid));
                    }
                }
                "left" => {
                    participant.left = time_in_sec;

                    if let Some(sec) = open_camera.remove(&pid) {
                        participant.camera.push((sec, time_in_sec));
                    }
                    if let Some(sec) = open_share.remove(&pid) {
                        participant.share.push((sec, time_in_sec));
                    }
                    if let Some(sec) = open_audio.remove(&pid) {
                        participant.audio.push((sec, time_in_sec));
                    }
                }
                "camera_on" => {
                    open_camera.insert(pid, time_in_sec);
                }
                "camera_off" => {
                    if let Some(sec) = open_camera.remove(&pid) {
                        participant.camera.push((sec, time_in_sec));
                    }
                }
                "screenshare_on" => {
                    open_share.insert(pid, time_in_sec);
                }
                "screenshare_off" => {
                    if let Some(sec) = open_share.remove(&pid) {
                        participant.share.push((sec, time_in_sec));
                    }
                }
                "audio_on" => {
                    open_audio.insert(pid, time_in_sec);
                }
                "audio_off" => {
                    if let Some(sec) = open_audio.remove(&pid) {
                        participant.audio.push((sec, time_in_sec));
                    }
                }

                "dominant" => match current_dominant.take() {
                    Some((start, who)) if who != pid => {
                        dominant.push(Dominant {
                            start,
                            end: time_in_sec,
                            endpoint: who,
                            nickname: "xxx###xxx".to_string(),
                        });
                        current_dominant = Some((time_in_sec, pid));
                    }
                    Some(saved) => {
                        current_dominant = Some(saved);
                    }
                    None => {
                        current_dominant = Some((time_in_sec, pid));
                    }
                },

                _ => {}
            }
        }

        for (pid, participant) in participants.iter_mut() {
            if participant.left == 0.0 && participant.joined >= 0.0 {
                participant.left = end;
            }
            if let Some(sec) = open_camera.remove(pid) {
                participant.camera.push((sec, end));
            }
            if let Some(sec) = open_share.remove(pid) {
                participant.share.push((sec, end));
            }
            if let Some(sec) = open_audio.remove(pid) {
                participant.audio.push((sec, end));
            }
        }

        participants.remove("");
        if let Some((start, who)) = current_dominant.take() {
            dominant.push(Dominant {
                start,
                end,
                endpoint: who,
                nickname: "xxx###xxx".to_string(),
            });
        }

        Timeline {
            participants,
            dominant,
            end,
        }
    }
}

impl Participant {
    /// Whether they were in the meeting at `t` seconds. Intervals are
    /// half-open, so the instant they leave already counts as absent.
    pub fn present_at(&self, t: f64) -> bool {
        self.joined <= t && t < self.left
    }
    /// Whether their camera was live at `t` seconds.
    pub fn camera_on_at(&self, t: f64) -> bool {
        self.camera.iter().any(|&(s, e)| s <= t && t < e)
    }
    /// Whether they were screensharing at `t` seconds.
    pub fn sharing_at(&self, t: f64) -> bool {
        self.share.iter().any(|&(s, e)| s <= t && t < e)
    }
}
