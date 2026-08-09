//! Turning layout segments into concrete clips over the recorded files.
//!
//! A [`Placement`] says "show this participant's camera here, from 12s to
//! 40s"; a [`Clip`] adds which file that is and how far into it to seek, since
//! each recording starts when that stream did, not when the meeting did.

use std::path::Path;

use crate::{
    layout::{Placement, Rect, Segment, Source},
    timeline::Timeline,
};

/// What a clip's source file holds.
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

/// One source file placed on the output timeline.
#[derive(Debug, Clone)]
#[allow(unused)]
pub(crate) struct Clip {
    pub kind: ClipKind,
    pub participant: Participant,
    /// `file://` URI of the recording.
    pub uri: String,
    /// When the clip starts and ends on the output timeline, in seconds.
    pub timeline_start: f64,
    pub timeline_end: f64,
    /// Where to place it in the frame; `None` for audio.
    pub rect: Option<Rect>,
    pub layer: u32,
    /// How far into the source file to start, in seconds — the offset between
    /// the meeting clock and this file's own clock. `None` for stills.
    pub inpoint: Option<f64>,
}

impl Clip {
    /// Resolves one placement into a clip: the file that backs it, and the
    /// seek offset that lines that file up with meeting time.
    ///
    /// Panics if the expected recording is missing, since a placement is only
    /// produced for a stream the timeline says was live.
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
    /// Builds every clip for a render: the visual clips from the segments,
    /// plus one audio clip per participant per span they were unmuted.
    ///
    /// Audio goes on its own layer above every visual one, and the result is
    /// sorted by start time then layer, which is the order the renderer adds
    /// them to the GES timeline.
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
