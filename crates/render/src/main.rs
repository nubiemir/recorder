use std::{fs, path::Path};

use gstreamer::{Caps, ClockTime, MessageView, State};
use gstreamer_editing_services::{self as ges, gst_pbutils, prelude::*};

fn ges_test() -> Result<(), Box<dyn std::error::Error>> {
    // Timeline with one audio + one video track
    let timeline = ges::Timeline::new_audio_video();
    let layer = timeline.append_layer();

    let path = Path::new("recordings/test1/18c29da1/camera_video.mkv").canonicalize()?;
    let uri = format!("file://{}", path.display());

    // Add a clip from a file URI
    let clip = ges::UriClip::new(&uri)?;
    layer.add_clip(&clip)?;

    // Trim: start at 0 on the timeline, use 5s..15s of the source
    clip.set_start(ClockTime::ZERO);
    // clip.set_inpoint(ClockTime::from_seconds(5));
    clip.set_duration(ClockTime::from_seconds(100));

    let path = Path::new("recordings/test1/18c29da1/audio.mkv").canonicalize()?;
    let uri = format!("file://{}", path.display());

    // Add a clip from a file URI
    let clip = ges::UriClip::new(&uri)?;
    layer.add_clip(&clip)?;

    // Trim: start at 0 on the timeline, use 5s..15s of the source
    clip.set_start(ClockTime::ZERO);
    // clip.set_inpoint(ClockTime::from_seconds(5));
    clip.set_duration(ClockTime::from_seconds(100));

    // A pipeline that can preview or render the timeline
    let pipeline = ges::Pipeline::new();
    pipeline.set_timeline(&timeline)?;
    let container = Caps::builder("video/quicktime")
        .field("variant", "iso")
        .build();
    let video_caps = Caps::new_empty_simple("video/x-h264");
    let audio_caps = Caps::new_empty_simple("audio/mpeg");

    let video_profile = gst_pbutils::EncodingVideoProfile::builder(&video_caps)
        .preset_name("vtenc_h264_hw")
        .build();

    let profile = gst_pbutils::EncodingContainerProfile::builder(&container)
        .add_profile(video_profile)
        .add_profile(gst_pbutils::EncodingAudioProfile::builder(&audio_caps).build())
        .build();

    fs::File::create("recordings/test1/output.mp4")?;

    let path = Path::new("recordings/test1/output.mp4").canonicalize()?;
    let uri = format!("file://{}", path.display());

    pipeline.set_render_settings(&uri, &profile)?;
    pipeline.set_mode(ges::PipelineFlags::RENDER)?;
    pipeline.set_state(State::Playing)?;

    // Run until EOS or error
    let bus = pipeline.bus().unwrap();
    for msg in bus.iter_timed(ClockTime::NONE) {
        match msg.view() {
            MessageView::Eos(..) => break,
            MessageView::Error(err) => {
                eprintln!("Error: {}", err.error());
                break;
            }
            _ => (),
        }
    }

    pipeline.set_state(State::Null)?;
    Ok(())
}

fn main() {
    gstreamer::init().expect("failed to initialize gstreamer");
    ges::init().expect("failed to initialize GES");
    ges_test().expect("hello something went wrong");
}
