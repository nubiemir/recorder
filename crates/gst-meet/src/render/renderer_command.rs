use gstreamer::Pad;
use std::sync::mpsc::Sender;

#[derive(Debug)]
pub enum RendererCommand {
    ParticipantJoined {
        endpoint_id: String,
        nickname: String,
        video_muted: bool,
        audio_muted: bool,
        screenshare_muted: bool,
    },
    ParticipantLeft {
        endpoint_id: String,
    },
    SourceInfoUpdated {
        endpoint_id: String,
        video_muted: bool,
        audio_muted: bool,
        screenshare_muted: bool,
    },
    RegisterSsrc {
        ssrc: u32,
        endpoint_id: String,
        source_name: String,
    },
    VideoStreamRemoved {
        endpoint_id: String,
        is_screenshare: bool,
    },
    DominantSpeaker {
        endpoint_id: String,
    },

    VideoStreamArrived {
        endpoint_id: String,
        is_screenshare: bool,
        reply: Sender<Option<Pad>>,
    },
    EndpointForSsrc {
        ssrc: u32,
        reply: Sender<Option<String>>,
    },
    IsScreenshareSsrc {
        ssrc: u32,
        reply: Sender<bool>,
    },
}
