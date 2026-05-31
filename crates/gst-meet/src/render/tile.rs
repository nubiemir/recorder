use gstreamer::{Element, Pad};

#[derive(Debug, PartialEq, PartialOrd)]
#[allow(unused)]
pub enum TileContent {
    BlackTile,
    Camera,
    Screenshare,
}

#[derive(Debug)]
#[allow(unused)]
pub struct Tile {
    pub endpoint: String,
    pub nickname: String,
    pub content: TileContent,
    pub black_src: Option<Element>,
    pub text_overlay: Option<Element>,
    pub convert: Option<Element>,
    pub caps_filter: Option<Element>,
    pub compositor_pad: Option<Pad>,
    pub large_pad: Option<Pad>,
    pub border_pad: Option<Pad>,
    pub screenshare_compositor_pad: Option<Pad>,
    pub has_screenshare: bool,
    pub video_muted: bool,
    pub audio_muted: bool,
    pub tee: Option<Element>,
    pub thumb_queue: Option<Element>,
    pub large_queue: Option<Element>,
    pub large_tee_pad: Option<Pad>,
}

impl Tile {
    pub fn new(endpoint: String, nickname: String, video_muted: bool, audio_muted: bool) -> Self {
        Self {
            endpoint,
            nickname,
            content: TileContent::BlackTile,
            black_src: None,
            text_overlay: None,
            convert: None,
            compositor_pad: None,
            caps_filter: None,
            large_pad: None,
            border_pad: None,
            screenshare_compositor_pad: None,
            has_screenshare: false,
            video_muted,
            audio_muted,
            thumb_queue: None,
            large_queue: None,
            large_tee_pad: None,
            tee: None,
        }
    }
}
