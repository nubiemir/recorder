use std::path::Path;

use crate::{
    layout::{Placement, Rect, Segment, Source},
    timeline::Timeline,
};

#[derive(Debug, Clone)]
pub(crate) enum ClipKind {
    Camera,
    ScreenShare,
    Avatar,
    Audio,
}

#[derive(Debug, Clone)]
#[allow(unused)]
pub(crate) struct Participant {
    pub endpoint: String,
    pub nickname: String,
}

#[derive(Debug, Clone)]
#[allow(unused)]
pub(crate) struct Clip {
    pub kind: ClipKind,
    pub participant: Participant,
    pub uri: String,
    pub timeline_start: f64,
    pub timeline_end: f64,
    pub rect: Option<Rect>,
    pub layer: u32,
    pub inpoint: Option<f64>,
}

impl Clip {
    fn placement_to_clip(
        tl: &Timeline,
        dir_entry: &str,
        placement: &Placement,
        start: f64,
        end: f64,
    ) -> Clip {
        match &placement.source {
            Source::Camera(pid) => {
                let joined = tl.participants[pid].joined;

                let path = Path::new(&format!("{dir_entry}/{pid}/camera_video.mkv"))
                    .canonicalize()
                    .unwrap();

                let uri = format!("file://{}", path.display());
                Clip {
                    kind: ClipKind::Camera,
                    participant: Participant {
                        endpoint: pid.clone(),
                        nickname: tl.dominant[0].nickname.clone(),
                    },
                    uri,
                    timeline_start: start,
                    timeline_end: end,
                    inpoint: Some((start - joined).max(0.0)),
                    rect: Some(placement.rect),
                    layer: placement.layer,
                }
            }

            Source::Share(pid) => {
                let share_start = tl.participants[pid]
                    .share
                    .first()
                    .map(|(start, _)| *start)
                    .unwrap_or(0.0);

                let path = Path::new(&format!("{dir_entry}/{pid}/screenshare_video.mkv"))
                    .canonicalize()
                    .unwrap();
                let uri = format!("file://{}", path.display());

                Clip {
                    kind: ClipKind::ScreenShare,
                    participant: Participant {
                        endpoint: pid.clone(),
                        nickname: tl.dominant[0].nickname.clone(),
                    },
                    uri,
                    timeline_start: start,
                    timeline_end: end,
                    inpoint: Some((start - share_start).max(0.0)),
                    rect: Some(placement.rect),
                    layer: placement.layer,
                }
            }

            Source::Avatar(pid) => {
                let path = Path::new(&format!("{dir_entry}/{pid}/avatar.png"))
                    .canonicalize()
                    .unwrap();
                let uri = format!("file://{}", path.display());

                Clip {
                    kind: ClipKind::Avatar,
                    participant: Participant {
                        endpoint: pid.clone(),
                        nickname: tl.dominant[0].nickname.clone(),
                    },
                    uri,
                    timeline_start: start,
                    timeline_end: end,
                    inpoint: None,
                    rect: Some(placement.rect),
                    layer: placement.layer,
                }
            }
        }
    }
    pub fn to_clips(tl: &Timeline, segments: &[Segment], dir_entry: &str) -> Vec<Clip> {
        let mut clips = Vec::new();

        for segment in segments {
            for placement in &segment.placements {
                clips.push(Self::placement_to_clip(
                    tl,
                    dir_entry,
                    &placement,
                    segment.start,
                    segment.end,
                ));
            }
        }

        let audio_layer = segments
            .iter()
            .flat_map(|segment| segment.placements.iter().map(|placement| placement.layer))
            .max()
            .unwrap_or(1);

        for (pid, participant) in &tl.participants {
            for &(start, end) in &participant.audio {
                let path = Path::new(&format!("{dir_entry}/{pid}/audio.mkv"))
                    .canonicalize()
                    .unwrap();
                let uri = format!("file://{}", path.display());

                clips.push(Clip {
                    kind: ClipKind::Audio,
                    participant: Participant {
                        endpoint: pid.clone(),
                        nickname: tl.dominant[0].nickname.clone(),
                    },
                    uri,
                    timeline_start: start,
                    timeline_end: end,
                    inpoint: Some((start - participant.joined).max(0.0)),
                    rect: None,
                    layer: audio_layer,
                });
            }
        }

        clips.sort_by(|a, b| {
            a.timeline_start
                .partial_cmp(&b.timeline_start)
                .unwrap()
                .then(a.layer.cmp(&b.layer))
        });

        clips
    }
}
