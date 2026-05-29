use gstreamer::{Pad, glib::object::ObjectExt};

pub const SCREEN_W: i32 = 1920;
pub const SCREEN_H: i32 = 1080;

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
            });

            // all cameras (including sharer's camera) stacked on right 25%
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
                });
            }

            return rects;
        }

        // ── Dominant speaker layout ───────────────────────────────────────────
        let count = tiles.len();
        if count == 0 {
            return rects;
        }

        if count == 1 {
            // only one participant, full screen
            rects.push(TileRect {
                endpoint: tiles[0].0.to_string(),
                xpos: 0,
                ypos: 0,
                width: SCREEN_W,
                height: SCREEN_H,
                is_screenshare: false,
                is_highlighted: false,
                is_dominant_large: false,
            });
            return rects;
        }

        if let Some(dom_id) = dominant_id {
            let small_count = tiles.len();

            // 2 columns if more than 10 participants
            let strip_cols = if small_count > 10 { 2 } else { 1 };
            let small_w = 140i32 * strip_cols; // total strip width stays ~280px
            let tile_w = small_w / strip_cols;
            let large_w = SCREEN_W - small_w;

            let rows = (small_count as f64 / strip_cols as f64).ceil() as i32;
            let tile_h = SCREEN_H / rows.max(1);

            // dominant large tile
            rects.push(TileRect {
                endpoint: dom_id.to_string(),
                xpos: 0,
                ypos: 0,
                width: large_w,
                height: SCREEN_H,
                is_screenshare: false,
                is_dominant_large: true,
                is_highlighted: false,
            });

            // all participants in small strip including dominant
            for (i, (ep, _, _)) in tiles.iter().enumerate() {
                let col = i as i32 % strip_cols;
                let row = i as i32 / strip_cols;

                rects.push(TileRect {
                    endpoint: ep.to_string(),
                    xpos: large_w + col * tile_w,
                    ypos: row * tile_h,
                    width: tile_w,
                    height: tile_h,
                    is_screenshare: false,
                    is_dominant_large: false,
                    is_highlighted: ep.as_str() == dom_id,
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
            });
        }

        rects
    }

    pub fn apply(rects: &[TileRect], get_pad: impl Fn(&str, bool) -> Option<Pad>) {
        for rect in rects {
            if let Some(pad) = get_pad(&rect.endpoint, rect.is_screenshare) {
                pad.set_property("xpos", rect.xpos);
                pad.set_property("ypos", rect.ypos);
                pad.set_property("width", rect.width);
                pad.set_property("height", rect.height);
            }
        }
    }
}
