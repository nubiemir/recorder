//! Everything scoped to a single endpoint in a meeting.
//!
//! A [`Participant`] holds three related things: their current mute state
//! ([`media::Media`]), the sources they announced over Jingle ([`SourceEntry`],
//! keyed by SSRC), and the recordings currently running for those sources
//! ([`branch::Branch`], also keyed by SSRC). The room owns the pipeline and the
//! bus; per-endpoint bookkeeping lives here.

use crate::{
    e2ee::E2EE,
    iq::jingle_action::ParsedSource,
    participant::{
        branch::Branch,
        media::{Media, MediaChanges},
    },
};
use gstreamer::Pipeline;
use log::{info, warn};
use std::{collections::HashMap, fmt::Display};

pub mod branch;
pub mod media;

/// Which of a participant's streams an SSRC carries.
///
/// Also the file stem the recording is written under, via [`Display`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceKind {
    CameraVideo,
    Audio,
    ScreenshareVideo,
}

/// A stream announced over Jingle, resolved from its SSRC.
#[derive(Debug, Clone)]
pub struct SourceEntry {
    pub endpoint: String,
    pub kind: SourceKind,
}

/// One endpoint in the meeting, with its media state, sources and recordings.
#[derive(Debug)]
pub struct Participant {
    pub e2ee_enabled: bool,
    pub nickname: String,
    pub media: Media,
    pub e2ee: E2EE,
    pub branches: HashMap<u32, Branch>,   // ssrc, Branch
    pub ssrcs: HashMap<u32, SourceEntry>, // ssrc, SourceEntry
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

impl Participant {
    pub fn new(nickname: &str) -> Self {
        Self {
            e2ee_enabled: false,
            e2ee: E2EE::new(),
            nickname: nickname.to_string(),
            media: Media::new(),
            branches: HashMap::new(),
            ssrcs: HashMap::new(),
        }
    }

    /// Applies a SourceInfo update, returning only the flags that flipped so
    /// the caller can emit the matching timeline events.
    pub fn update_media(&mut self, audio: bool, video: bool, screenshare: bool) -> MediaChanges {
        self.media.apply(audio, video, screenshare)
    }

    /// An avatar stands in for the camera feed while video is muted, and is
    /// rendered at most once per participant.
    pub fn needs_avatar(&self) -> bool {
        self.media.video_muted && !self.media.avatar_generated
    }

    pub fn add_branch(&mut self, ssrc: u32, branch: Branch) {
        self.branches.insert(ssrc, branch);
    }

    pub fn has_branches(&self) -> bool {
        !self.branches.is_empty()
    }

    /// Sends EOS to every branch still recording, e.g. when the participant
    /// leaves or the meeting ends.
    pub fn finalize_branches(&mut self) {
        for branch in self.branches.values_mut() {
            branch.finalize();
        }
    }

    /// Removes the first branch matching `pred`, used to resolve an EOS or an
    /// error message on the bus back to the recording it came from.
    pub fn take_branch(&mut self, pred: &dyn Fn(&Branch) -> bool) -> Option<Branch> {
        let ssrc = *self
            .branches
            .iter()
            .find(|(_, branch)| pred(branch))
            .map(|(ssrc, _)| ssrc)?;
        self.branches.remove(&ssrc)
    }

    /// Drops any branch that never reached EOS; its file may be truncated.
    pub fn abandon_branches(&mut self, pipeline: &Pipeline) {
        for (_, branch) in self.branches.drain() {
            warn!(
                "recording {} did not finalize; the file may be truncated",
                branch.path
            );
            branch.teardown(pipeline);
        }
    }

    /// "c8ef68f5-v1" -> ('v', 1), "eee01355-a0" -> ('a', 0)
    fn parse_source_name(&self, name: &str) -> Option<(char, u32)> {
        let suffix = name.rsplit('-').next()?; // "v1"
        let mut chars = suffix.chars();
        let letter = chars.next()?; // 'v' or 'a'
        let idx: u32 = chars.as_str().parse().ok()?; // 0, 1, ...
        Some((letter, idx))
    }

    /// Records what an SSRC carries, from a Jingle `source-add` or the initial
    /// `session-initiate`.
    ///
    /// Until this runs, a pad arriving for that SSRC can't be attributed to
    /// anyone and the room parks it. Unparseable source names are skipped with
    /// a warning rather than guessed at.
    pub fn register_ssrc(&mut self, parsed_source: ParsedSource) {
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

        self.ssrcs.insert(
            parsed_source.ssrc,
            SourceEntry {
                endpoint: parsed_source.endpoint_id,
                kind,
            },
        );
    }

    /// What this participant announced for `ssrc`, if anything.
    pub fn source_for_ssrc(&self, ssrc: u32) -> Option<&SourceEntry> {
        self.ssrcs.get(&ssrc)
    }
}
