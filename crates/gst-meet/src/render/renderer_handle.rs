use std::sync::mpsc::{self, Sender};

use gstreamer::Pad;

use crate::render::renderer_command::RendererCommand;

#[derive(Clone, Debug)]
pub struct RendererHandle {
    pub tx: Sender<RendererCommand>,
}

impl RendererHandle {
    pub fn participant_joined(
        &self,
        endpoint_id: &str,
        nickname: &str,
        video_muted: bool,
        audio_muted: bool,
    ) {
        let _ = self.tx.send(RendererCommand::ParticipantJoined {
            endpoint_id: endpoint_id.to_string(),
            nickname: nickname.to_string(),
            video_muted,
            audio_muted,
        });
    }

    pub fn participant_left(&self, endpoint_id: &str) {
        let _ = self.tx.send(RendererCommand::ParticipantLeft {
            endpoint_id: endpoint_id.to_string(),
        });
    }

    pub fn source_info_updated(
        &self,
        endpoint_id: &str,
        video_muted: bool,
        audio_muted: bool,
        has_screenshare: bool,
    ) {
        let _ = self.tx.send(RendererCommand::SourceInfoUpdated {
            endpoint_id: endpoint_id.to_string(),
            video_muted,
            audio_muted,
            has_screenshare,
        });
    }

    pub fn register_ssrc(&self, ssrc: u32, endpoint_id: &str, source_name: &str) {
        let _ = self.tx.send(RendererCommand::RegisterSsrc {
            ssrc,
            endpoint_id: endpoint_id.to_string(),
            source_name: source_name.to_string(),
        });
    }

    pub fn endpoint_for_ssrc(&self, ssrc: u32) -> Option<String> {
        let (reply_tx, reply_rx) = mpsc::channel();
        let _ = self.tx.send(RendererCommand::EndpointForSsrc {
            ssrc,
            reply: reply_tx,
        });
        reply_rx.recv().ok().flatten()
    }

    pub fn is_screenshare_ssrc(&self, ssrc: u32) -> bool {
        let (reply_tx, reply_rx) = mpsc::channel();
        let _ = self.tx.send(RendererCommand::IsScreenshareSsrc {
            ssrc,
            reply: reply_tx,
        });
        reply_rx.recv().unwrap_or(false)
    }

    pub fn video_stream_arrived(&self, endpoint_id: &str, is_screenshare: bool) -> Option<Pad> {
        let (reply_tx, reply_rx) = mpsc::channel();
        let _ = self.tx.send(RendererCommand::VideoStreamArrived {
            endpoint_id: endpoint_id.to_string(),
            is_screenshare,
            reply: reply_tx,
        });
        reply_rx.recv().ok().flatten()
    }

    pub fn video_stream_removed(&self, endpoint_id: &str, is_screenshare: bool) {
        let _ = self.tx.send(RendererCommand::VideoStreamRemoved {
            endpoint_id: endpoint_id.to_string(),
            is_screenshare,
        });
    }

    pub fn dominant_speaker(&self, endpoint_id: &str) {
        let _ = self.tx.send(RendererCommand::DominantSpeaker {
            endpoint_id: endpoint_id.to_string(),
        });
    }
}
