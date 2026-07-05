use std::{
    collections::HashMap,
    sync::mpsc::{self, Receiver},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use log::{error, info};
use serde::Serialize;

use crate::timeline::{
    Media, Metadata, Participant, Timeline,
    timeline_handler::{TimelineEvent, TimelineHandler},
    timeline_process::write_file,
};

pub struct TimelineEngine {
    pub output_path: String,
    pub start_instant: Instant,
    pub start_timestamp: u128,
}

fn now_unix() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
}

impl TimelineEngine {
    pub fn new(output_path: String) -> Self {
        Self {
            output_path,
            start_instant: Instant::now(),
            start_timestamp: now_unix(),
        }
    }

    pub fn spawn(
        output_path: String,
        room: String,
        start_instant: Instant,
        start_timestamp: u128,
    ) -> TimelineHandler {
        let (tx, rx) = mpsc::channel();
        let (files_written_tx, files_written_rx) = mpsc::channel();

        std::thread::spawn(move || {
            Self::run(
                rx,
                files_written_tx,
                output_path,
                room,
                start_instant,
                start_timestamp,
            );
        });

        TimelineHandler::new(tx, start_instant, files_written_rx)
    }
    fn run(
        rx: Receiver<TimelineEvent>,
        files_written_tx: mpsc::Sender<()>,
        output_path: String,
        room: String,
        start_instant: Instant,
        start_timestamp: u128,
    ) {
        let mut events: Vec<Timeline> = Vec::new();
        let mut participants = HashMap::new();

        while let Ok(evt) = rx.recv() {
            let meeting_ended = matches!(evt, TimelineEvent::MeetingEnd { .. });

            Self::parse_event(evt, &mut events, &mut participants);

            if meeting_ended {
                break;
            }
        }

        let end_timestamp = now_unix();
        let total_duration_ms = start_instant.elapsed().as_millis();

        let metadata = Metadata {
            room_id: "xxxxxx".to_string(),
            room_name: room,
            output_dir: output_path.to_string(),
            total_duration_ms,
            start_timestamp,
            end_timestamp,
            participants,
        };

        let metadata_path = format!("{}/metadata.json", output_path);
        let timeline_path = format!("{}/timeline.json", output_path);

        Self::generate_output_files(&metadata_path, &metadata);
        Self::generate_output_files(&timeline_path, &events);

        let _ = files_written_tx.send(());
    }

    fn generate_output_files(path: &str, content: impl Serialize) {
        match write_file(path, content) {
            Ok(()) => {
                info!("file saved to {}", path);
            }
            Err(err) => {
                error!("failed to create file at {} | err {}", path, err);
            }
        }
    }

    fn parse_event(
        event: TimelineEvent,
        events: &mut Vec<Timeline>,
        participants: &mut HashMap<String, Participant>,
    ) {
        if let TimelineEvent::Joined {
            endpoint, nickname, ..
        } = &event
        {
            if let Some(endpoint) = endpoint {
                participants.insert(
                    endpoint.clone(),
                    Participant {
                        name: nickname.to_string(),
                        media: Media {
                            camera: None,
                            audio: None,
                            screenshare: None,
                        },
                    },
                );
            }
        }

        let event_code = event.to_string();

        match event {
            TimelineEvent::MeetingStart {
                timestamp_ms,
                endpoint,
            }
            | TimelineEvent::MeetingEnd {
                timestamp_ms,
                endpoint,
            }
            | TimelineEvent::Joined {
                timestamp_ms,
                endpoint,
                ..
            }
            | TimelineEvent::Left {
                timestamp_ms,
                endpoint,
            }
            | TimelineEvent::CameraOn {
                timestamp_ms,
                endpoint,
            }
            | TimelineEvent::CameraOff {
                timestamp_ms,
                endpoint,
            }
            | TimelineEvent::ScreenshareOn {
                timestamp_ms,
                endpoint,
            }
            | TimelineEvent::ScreenshareOff {
                timestamp_ms,
                endpoint,
            }
            | TimelineEvent::AudioOn {
                timestamp_ms,
                endpoint,
            }
            | TimelineEvent::AudioOff {
                timestamp_ms,
                endpoint,
            }
            | TimelineEvent::Dominant {
                timestamp_ms,
                endpoint,
            } => {
                events.push(Timeline {
                    timestamp: timestamp_ms,
                    participant_id: endpoint,
                    event: event_code,
                });
            }
        }
    }
}
