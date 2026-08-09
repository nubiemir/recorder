//! A participant's mute state, and the diff between two reports of it.

/// Mute flags that flipped in a single SourceInfo update. `None` means the
/// flag did not change, so the room emits no timeline event for it.
#[derive(Debug, Default, Clone, Copy)]
pub struct MediaChanges {
    pub audio_muted: Option<bool>,
    pub video_muted: Option<bool>,
    pub screenshare_muted: Option<bool>,
}

/// Last known mute state for one participant.
///
/// Everything starts muted, which matches how [`parse_source_info`] treats a
/// missing or unreadable `SourceInfo`: absence of evidence is not evidence of
/// a live stream.
///
/// [`parse_source_info`]: crate::presence::ParticipantPresence
#[derive(Debug, Default)]
pub struct Media {
    pub audio_muted: bool,
    pub video_muted: bool,
    pub screenshare_muted: bool,
    /// Whether the camera-off placeholder has already been rendered, so it is
    /// drawn once rather than on every mute toggle.
    pub avatar_generated: bool,
}

impl Media {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_audio_muted(&mut self, audio_muted: bool) -> &mut Self {
        self.audio_muted = audio_muted;
        self
    }

    pub fn set_video_muted(&mut self, video_muted: bool) -> &mut Self {
        self.video_muted = video_muted;
        self
    }

    pub fn set_screenshare_muted(&mut self, screenshare_muted: bool) -> &mut Self {
        self.screenshare_muted = screenshare_muted;
        self
    }

    pub fn set_avatar_generated(&mut self, avatar_generated: bool) -> &mut Self {
        self.avatar_generated = avatar_generated;
        self
    }

    /// Stores a freshly announced mute state and reports what changed.
    pub fn apply(
        &mut self,
        audio_muted: bool,
        video_muted: bool,
        screenshare_muted: bool,
    ) -> MediaChanges {
        let changes = MediaChanges {
            audio_muted: (self.audio_muted != audio_muted).then_some(audio_muted),
            video_muted: (self.video_muted != video_muted).then_some(video_muted),
            screenshare_muted: (self.screenshare_muted != screenshare_muted)
                .then_some(screenshare_muted),
        };

        self.set_audio_muted(audio_muted)
            .set_video_muted(video_muted)
            .set_screenshare_muted(screenshare_muted);

        changes
    }
}
