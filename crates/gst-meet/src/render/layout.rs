use gstreamer::{Element, Pad, glib::object::ObjectExt};

pub const SCREEN_W: i32 = 1920;
pub const SCREEN_H: i32 = 1080;
pub const SMALL_TILE_W: i32 = 250;
pub const SMALL_TILE_H: i32 = 250;

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

#[derive(Debug)]
pub struct TileRect {
    pub endpoint: String,
    pub xpos: i32,
    pub ypos: i32,
    pub width: i32,
    pub height: i32,
    pub is_highlighted: bool,
    pub is_screenshare: bool,
    pub is_dominant_large: bool,
    pub zorder: u32,
}

#[derive(Debug)]
pub struct LayoutEngine;

impl LayoutEngine {
    pub fn calculate(
        tiles: &[(&String, bool, bool)], // (endpoint_id, has_screenshare, is_dominant)
        dominant_id: Option<&str>,
    ) -> Vec<TileRect> {
        let mut rects = vec![];

        // ── Screenshare layout ────────────────────────────────────────────────
        let screensharer = tiles.iter().find(|(_, has_share, _)| *has_share);

        if let Some((share_ep, _, _)) = screensharer {
            let share_w = (SCREEN_W as f64 * 0.75) as i32;

            // screenshare takes left 75%
            rects.push(TileRect {
                endpoint: share_ep.to_string(),
                xpos: 0,
                ypos: 0,
                width: share_w,
                height: SCREEN_H,
                is_screenshare: true,
                is_dominant_large: true,
                is_highlighted: false,
                zorder: 0,
            });

            // all cameras stacked on right 25%
            let cameras: Vec<_> = tiles.iter().collect();
            let count = cameras.len().max(1);
            let tile_w = SCREEN_W - share_w;
            let tile_h = SCREEN_H / count as i32;

            for (i, (ep, _, _)) in cameras.iter().enumerate() {
                rects.push(TileRect {
                    endpoint: ep.to_string(),
                    xpos: share_w,
                    ypos: i as i32 * tile_h,
                    width: tile_w,
                    height: tile_h,
                    is_screenshare: false,
                    is_dominant_large: false,
                    is_highlighted: ep.as_str() == share_ep.as_str(),
                    zorder: 1,
                });
            }

            return rects;
        }

        // ── Dominant speaker layout ───────────────────────────────────────────
        let count = tiles.len();
        if count == 0 {
            return rects;
        }

        // NOTE: The `if count == 1` block was removed from here.
        // Now, 1 user will be treated as a dominant speaker with 1 thumbnail.

        if let Some(dom_id) = dominant_id {
            let small_count = tiles.len();
            let strip_cols = if small_count > 10 { 2 } else { 1 };

            // dominant takes full screen, bottom layer
            rects.push(TileRect {
                endpoint: dom_id.to_string(),
                xpos: 0,
                ypos: 0,
                width: SCREEN_W,
                height: SCREEN_H,
                is_screenshare: false,
                is_dominant_large: true,
                is_highlighted: false,
                zorder: 0,
            });

            // small tiles top-right corner, on top layer
            let total_small_w = SCREEN_W - (SMALL_TILE_W * strip_cols);

            for (i, (ep, _, _)) in tiles.iter().enumerate() {
                let col = i as i32 % strip_cols;
                let row = i as i32 / strip_cols;

                rects.push(TileRect {
                    endpoint: ep.to_string(),
                    xpos: total_small_w + col * SMALL_TILE_W,
                    ypos: row * SMALL_TILE_H,
                    width: SMALL_TILE_W,
                    height: SMALL_TILE_H,
                    is_screenshare: false,
                    is_dominant_large: false,
                    is_highlighted: ep.as_str() == dom_id,
                    zorder: 1,
                });
            }

            return rects;
        }

        // ── Equal grid (no dominant, no screenshare) ──────────────────────────
        let cols = (count as f64).sqrt().ceil() as i32;
        let rows = ((count as f64) / cols as f64).ceil() as i32;
        let tile_w = SCREEN_W / cols;
        let tile_h = SCREEN_H / rows;

        for (i, (ep, _, _)) in tiles.iter().enumerate() {
            let col = i as i32 % cols;
            let row = i as i32 / cols;
            rects.push(TileRect {
                endpoint: ep.to_string(),
                xpos: col * tile_w,
                ypos: row * tile_h,
                width: tile_w,
                height: tile_h,
                is_screenshare: false,
                is_highlighted: false,
                is_dominant_large: false,
                zorder: 0,
            });
        }

        rects
    }

    pub fn apply(
        rects: &[TileRect],
        get_pad: impl Fn(&str, bool) -> Option<Pad>,
        get_large_pad: impl Fn(&str) -> Option<Pad>,
    ) {
        for rect in rects {
            let pad = if rect.is_dominant_large {
                get_large_pad(&rect.endpoint)
            } else {
                get_pad(&rect.endpoint, rect.is_screenshare)
            };

            if let Some(pad) = pad {
                pad.set_property("xpos", rect.xpos);
                pad.set_property("ypos", rect.ypos);
                pad.set_property("width", rect.width);
                pad.set_property("height", rect.height);
                pad.set_property("zorder", rect.zorder);
            }
        }
    }
}
