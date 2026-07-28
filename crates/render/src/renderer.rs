use std::{fs, ops::Deref, path::Path};

use gstreamer::{
    Caps, ClockTime, Fraction, MessageView, State, StateChangeError, glib::BoolError,
    prelude::ElementExt,
};
use gstreamer_editing_services::{
    self as ges, TrackType,
    gst_pbutils::{EncodingAudioProfile, EncodingContainerProfile, EncodingVideoProfile},
    prelude::{
        ClipExt, GESPipelineExt, GESTrackExt, LayerExt, TimelineElementExt,
        TimelineElementExtManual, TimelineExt,
    },
};
use thiserror::Error;

use crate::{
    clip::{Clip, ClipKind},
    layout::{OUT_HEIGHT, OUT_WIDTH},
};

#[derive(Debug, Error)]
pub enum RendererError {
    #[error("gst bool error: {0}")]
    BoolError(#[from] BoolError),

    #[error("io error: {0}")]
    IOError(#[from] std::io::Error),

    #[error("state change error: {0}")]
    StateChangeError(#[from] StateChangeError),

    #[error("renderer error: {0}")]
    RendererError(String),
}

type Result<T> = std::result::Result<T, RendererError>;

pub(crate) struct Renderer(ges::Timeline);

impl Deref for Renderer {
    type Target = ges::Timeline;
    fn deref(&self) -> &ges::Timeline {
        &self.0
    }
}

impl Renderer {
    pub fn render(&self, out_uri: &str) -> Result<()> {
        let video = EncodingVideoProfile::builder(&Caps::builder("video/x-h264").build()).build();

        let audio = EncodingAudioProfile::builder(
            &Caps::builder("audio/mpeg").field("mpegversion", 4).build(),
        )
        .build();

        let container = EncodingContainerProfile::builder(
            &Caps::builder("video/quicktime")
                .field("variant", "iso")
                .build(),
        )
        .add_profile(video)
        .add_profile(audio)
        .build();

        fs::File::create(format!("{out_uri}/output.mp4"))?;

        let path = Path::new(&format!("{out_uri}/output.mp4")).canonicalize()?;
        let uri = format!("file://{}", path.display());

        let pipeline = ges::Pipeline::new();
        pipeline.set_timeline(self.deref())?;
        pipeline.set_render_settings(&uri, &container)?;
        pipeline.set_mode(ges::PipelineFlags::RENDER)?; // offline: render as fast as possible

        pipeline.set_state(State::Playing)?;
        let bus = pipeline.bus().unwrap();
        for msg in bus.iter_timed(ClockTime::NONE) {
            use MessageView::*;
            match msg.view() {
                Eos(..) => break,
                Error(e) => {
                    pipeline.set_state(State::Null)?;
                    return Err(RendererError::RendererError(format!(
                        "{} [{:?}]",
                        e.error(),
                        e.debug(),
                    )));
                }
                _ => {}
            }
        }
        pipeline.set_state(State::Null)?;
        Ok(())
    }

    pub fn build(clips: &[Clip]) -> Result<Self> {
        let timeline = ges::Timeline::new_audio_video();

        for track in timeline.tracks() {
            if track.track_type() == TrackType::VIDEO {
                let caps = Caps::builder("video/x-raw")
                    .field("width", OUT_WIDTH)
                    .field("height", OUT_HEIGHT)
                    .field("framerate", Fraction::new(30, 1))
                    .build();

                track.set_restriction_caps(&caps);
            }
        }

        let layers_count = clips.iter().map(|clip| clip.layer + 1).max().unwrap_or(1);

        let layers = Self::layers(&timeline, layers_count);

        for clip in clips {
            Self::add_clip(&layers, &clip)?;
        }

        Ok(Renderer(timeline))
    }

    fn layers(timeline: &ges::Timeline, count: u32) -> Vec<ges::Layer> {
        (0..count).map(|_| timeline.append_layer()).collect()
    }

    fn secs(t: f64) -> ClockTime {
        ClockTime::from_seconds_f64(t)
    }

    fn add_clip(layers: &[ges::Layer], c: &Clip) -> Result<()> {
        let layer = &layers[c.layer as usize];
        let secs = Self::secs;

        match c.kind {
            ClipKind::Camera | ClipKind::ScreenShare => {
                let uri = &c.uri;
                let clip = ges::UriClip::new(uri)?;
                clip.set_supported_formats(ges::TrackType::VIDEO);
                clip.set_start(secs(c.timeline_start));
                clip.set_duration(secs(c.timeline_end - c.timeline_start));
                clip.set_inpoint(secs(c.inpoint.unwrap_or(0.0)));
                layer.add_clip(&clip)?;
                if let Some(r) = c.rect {
                    clip.set_child_property("posx", &r.x)?;
                    clip.set_child_property("posy", &r.y)?;
                    clip.set_child_property("width", &r.w)?;
                    clip.set_child_property("height", &r.h)?;
                }
            }

            ClipKind::Avatar => {
                let uri = &c.uri;
                let clip = ges::UriClip::new(uri)?;
                clip.set_start(secs(c.timeline_start));
                clip.set_duration(secs(c.timeline_end - c.timeline_start));
                clip.set_inpoint(secs(0.0));
                layer.add_clip(&clip)?;
                if let Some(r) = c.rect {
                    clip.set_child_property("posx", &r.x)?;
                    clip.set_child_property("posy", &r.y)?;
                    clip.set_child_property("width", &r.w)?;
                    clip.set_child_property("height", &r.h)?;
                }
            }

            ClipKind::Audio => {
                let uri = &c.uri;
                let clip = ges::UriClip::new(uri)?;
                clip.set_supported_formats(ges::TrackType::AUDIO);
                clip.set_start(secs(c.timeline_start));
                clip.set_duration(secs(c.timeline_end - c.timeline_start));
                clip.set_inpoint(secs(c.inpoint.unwrap_or(0.0)));
                layer.add_clip(&clip)?;
            }
        }
        Ok(())
    }
}
