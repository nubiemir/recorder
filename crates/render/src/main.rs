//! Render service: turns a recorded meeting into a single composited video.
//!
//! `GET /?room=<name>` reads `recordings/<room>/timeline.json`, cuts the
//! meeting into segments where the layout is constant, resolves each segment
//! into clips over the per-participant recordings, and renders the result with
//! GStreamer Editing Services.
//!
//! The pipeline is: [`timeline`] (what happened) → [`layout`] (what should be
//! on screen when) → [`clip`] (which file and which part of it) →
//! [`renderer`] (the GES timeline and the encode).

use std::{fs, path::Path, thread};

use gstreamer::{Caps, ClockTime, MessageView, State};
use gstreamer_editing_services::{self as ges, gst_pbutils, prelude::*};
use log::{error, info, warn};
use tiny_http::{Request, Response, Server};

use crate::{
    clip::Clip,
    layout::{active_speaker::ActiveSpeaker, segment_with},
    renderer::Renderer,
    timeline::{Timeline, TimelineError},
};

mod clip;
mod layout;
mod renderer;
mod timeline;

/// Hardcoded GES smoke test kept for debugging the encoder settings; not
/// wired into the service.
fn _ges_test() -> std::result::Result<(), Box<dyn std::error::Error>> {
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

type Result<T> = std::result::Result<T, RenderError>;

/// Failures while locating or loading a recorded meeting.
#[derive(Debug, thiserror::Error)]
enum RenderError {
    #[error("io error: {0}")]
    IOError(#[from] std::io::Error),

    #[error("timeline error: {0}")]
    TimelineError(#[from] TimelineError),
}

/// Pulls the room name out of the `room=` query parameter.
fn parse_room(request: &Request) -> String {
    let url = request.url();

    if let Some(pos) = url.find("room=") {
        let query_value = &url[pos + 5..];
        let room_name = query_value.split('&').next().unwrap_or(query_value);
        return room_name.to_string();
    }

    "unknown_room".to_string()
}

/// Loads the recorded timeline for a room.
fn process_request(room_name: &str) -> Result<Timeline> {
    warn!("found this room: {}", room_name);
    let format_path = format!("recordings/{}/timeline.json", room_name);
    let path = Path::new(&format_path).canonicalize()?;
    let timeline = Timeline::load_timeline(path.to_str().unwrap_or_default())?;
    Ok(timeline)
}

/// Serves render requests, one thread per request.
///
/// The response is sent only after the render finishes, so a request to this
/// service blocks for roughly the length of the encode.
fn main() {
    env_logger::init();

    gstreamer::init().expect("failed to initialize gstreamer");
    ges::init().expect("failed to initialize GES");

    let server = Server::http(&format!("127.0.0.1:3333"));

    match server {
        Ok(server) => {
            info!("started listening on: {:?}", server.server_addr());

            for request in server.incoming_requests() {
                thread::spawn(move || {
                    let room = parse_room(&request);

                    let timeline = process_request(&room).unwrap();
                    let segments = segment_with(ActiveSpeaker::default(), &timeline);
                    let clips = Clip::to_clips(&timeline, &segments, &format!("recordings/{room}"));
                    let renderer = Renderer::build(&clips);
                    match renderer {
                        Ok(renderer) => match renderer.render(&format!("recordings/{room}")) {
                            Ok(_) => {
                                info!("successfully renderered mp4 file");
                            }
                            Err(err) => {
                                error!("failed to produce final file error: {:#?}", err);
                            }
                        },
                        Err(err) => {
                            error!("renderer error: {:#?}", err);
                        }
                    }

                    let message = format!("successfully joined room: {}", room);
                    let response = Response::from_string(message).with_status_code(200); // Forces standard HTTP compliance

                    let _ = request.respond(response);
                });
            }
        }
        Err(err) => {
            error!("error starting server: {:?}", err);
        }
    }
}
