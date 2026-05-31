use std::{collections::HashMap, sync::Mutex};

use gstreamer::{
    Caps, Element, ElementFactory, Pad, Pipeline, State,
    glib::{BoolError, bool_error},
    prelude::{ElementExt, ElementExtManual, GstBinExt, GstBinExtManual, PadExt},
};

use log::{error, info};

use crate::render::{
    layout::LayoutEngine,
    tile::{Tile, TileContent},
};

#[derive(Debug)]
#[allow(unused)]
pub struct RendererEngine {
    pipeline: Pipeline,
    compositor: Element,
    tiles: Mutex<HashMap<String, Tile>>, // endpoint_id → Tile
    ssrc_map: Mutex<HashMap<u32, (String, bool)>>,
    dominant_speaker: Mutex<Option<String>>,
}

impl RendererEngine {
    pub fn new(pipeline: Pipeline, compositor: Element) -> Self {
        Self {
            pipeline,
            compositor,
            tiles: Mutex::new(HashMap::new()),
            ssrc_map: Mutex::new(HashMap::new()),
            dominant_speaker: Mutex::new(None),
        }
    }

    // ── Participant lifecycle ─────────────────────────────────────────────

    pub fn on_participant_joined(
        &self,
        endpoint_id: &str,
        nickname: &str,
        video_muted: bool,
        audio_muted: bool,
    ) {
        {
            let tiles = match self.tiles.lock() {
                Ok(t) => t,
                Err(e) => {
                    error!("tiles lock: {e}");
                    return;
                }
            };
            if tiles.contains_key(endpoint_id) {
                return;
            }
        }

        info!(
            "participant joined endpoint={} nick={}",
            endpoint_id, nickname
        );

        if let Err(e) = self.add_black_tile_for(endpoint_id, nickname, video_muted, audio_muted) {
            error!("failed to add black tile for {}: {:?}", endpoint_id, e);
            return;
        }

        let no_dominant = self.dominant_speaker.lock().unwrap().is_none();
        if no_dominant {
            info!("setting initial dominant speaker → {}", endpoint_id);
            self.promote_to_dominant(endpoint_id);
        }

        self.recalculate_layout();
    }

    pub fn on_participant_left(&self, endpoint_id: &str) {
        info!("participant left endpoint={}", endpoint_id);
        self.remove_tile(endpoint_id);
        self.recalculate_layout();
    }

    // ── Source info / mute state ──────────────────────────────────────────

    pub fn on_source_info_updated(
        &self,
        endpoint_id: &str,
        video_muted: bool,
        audio_muted: bool,
        has_screenshare: bool,
    ) {
        let mut tiles = self.tiles.lock().unwrap();

        let tile = match tiles.get_mut(endpoint_id) {
            Some(t) => t,
            None => {
                error!("source_info for unknown endpoint {}", endpoint_id);
                return;
            }
        };

        let prev_video_muted = tile.video_muted;
        tile.video_muted = video_muted;
        tile.audio_muted = audio_muted;
        tile.has_screenshare = has_screenshare;

        if !prev_video_muted && video_muted && tile.content == TileContent::Camera {
            info!("camera off for {}, switching to black tile", endpoint_id);
            drop(tiles);
            self.swap_to_black_tile(endpoint_id);
            self.recalculate_layout();
            return;
        }

        drop(tiles);
        self.recalculate_layout();
    }

    // ── SSRC map ─────────────────────────────────────────────────────────

    pub fn register_ssrc(&self, ssrc: u32, endpoint_id: &str, source_name: &str) {
        let is_screenshare = source_name.ends_with("-v1");
        self.ssrc_map
            .lock()
            .unwrap()
            .insert(ssrc, (endpoint_id.to_string(), is_screenshare));
    }

    pub fn endpoint_for_ssrc(&self, ssrc: u32) -> Option<String> {
        self.ssrc_map
            .lock()
            .unwrap()
            .get(&ssrc)
            .map(|(ep, _)| ep.clone())
    }

    pub fn is_screenshare_ssrc(&self, ssrc: u32) -> bool {
        self.ssrc_map
            .lock()
            .unwrap()
            .get(&ssrc)
            .map(|(_, is_share)| *is_share)
            .unwrap_or(false)
    }

    // ── RTP stream arrived ────────────────────────────────────────────────

    pub fn on_video_stream_arrived(&self, endpoint_id: &str, is_screenshare: bool) -> Option<Pad> {
        info!(
            "video stream arrived endpoint={} screenshare={}",
            endpoint_id, is_screenshare
        );

        if is_screenshare {
            return self.add_screenshare_pad(endpoint_id);
        }

        self.swap_to_video(endpoint_id)
    }

    pub fn on_video_stream_removed(&self, endpoint_id: &str, is_screenshare: bool) {
        if is_screenshare {
            self.remove_screenshare_pad(endpoint_id);
        } else {
            self.swap_to_black_tile(endpoint_id);
        }
        self.recalculate_layout();
    }

    // ── Dominant speaker ─────────────────────────────────────────────────

    fn promote_to_dominant(&self, endpoint_id: &str) {
        let mut tiles = self.tiles.lock().unwrap();

        let prev = {
            let mut dominant = self.dominant_speaker.lock().unwrap();
            let prev = dominant.clone();
            *dominant = Some(endpoint_id.to_string());
            prev
        };

        // 1. Demote previous dominant
        if let Some(prev_id) = prev {
            if prev_id != endpoint_id {
                if let Some(tile) = tiles.get_mut(&prev_id) {
                    if let Some(pad) = tile.large_pad.take() {
                        self.compositor.release_request_pad(&pad);
                    }
                    if let (Some(tee), Some(tee_pad)) = (&tile.tee, tile.large_tee_pad.take()) {
                        tee.release_request_pad(&tee_pad);
                    }
                    if let Some(queue) = tile.large_queue.take() {
                        let _ = queue.set_state(State::Null);
                        let _ = self.pipeline.remove(&queue);
                    }
                }
            }
        }

        // 2. Promote new dominant
        if let Some(tile) = tiles.get_mut(endpoint_id) {
            if let Some(tee) = &tile.tee {
                let large_pad = self.compositor.request_pad_simple("sink_%u").unwrap();

                let large_queue = ElementFactory::make("queue").build().unwrap();
                self.pipeline.add(&large_queue).unwrap();

                let tee_src_pad = tee.request_pad_simple("src_%u").unwrap();
                tee_src_pad
                    .link(&large_queue.static_pad("sink").unwrap())
                    .unwrap();
                large_queue
                    .static_pad("src")
                    .unwrap()
                    .link(&large_pad)
                    .unwrap();

                large_queue.sync_state_with_parent().unwrap();

                tile.large_pad = Some(large_pad);
                tile.large_queue = Some(large_queue);
                tile.large_tee_pad = Some(tee_src_pad);
            }
        }
    }

    pub fn on_dominant_speaker(&self, endpoint_id: &str) {
        info!("dominant speaker → {}", endpoint_id);
        self.promote_to_dominant(endpoint_id);
        self.recalculate_layout();
    }

    // ── Internal ─────────────────────────────────────────────────────────

    fn add_black_tile_for(
        &self,
        endpoint_id: &str,
        nickname: &str,
        video_muted: bool,
        audio_muted: bool,
    ) -> Result<(), BoolError> {
        let src = ElementFactory::make("videotestsrc")
            .property_from_str("pattern", "black")
            .property("is-live", true)
            .build()?;

        let textoverlay = ElementFactory::make("textoverlay")
            .property("text", nickname)
            .property_from_str("valignment", "center")
            .property_from_str("halignment", "center")
            .property("font-desc", "Sans Bold 24")
            .build()?;

        let caps = Caps::builder("video/x-raw")
            .field("width", 1920i32)
            .field("height", 1080i32)
            .field("framerate", gstreamer::Fraction::new(30, 1))
            .field("format", "I420")
            .build();

        let capsfilter = ElementFactory::make("capsfilter")
            .property("caps", &caps)
            .build()?;

        let convert = ElementFactory::make("videoconvert").build()?;

        // Setup Tee & Thumbnail Queue
        let tee = ElementFactory::make("tee").build()?;
        let thumb_queue = ElementFactory::make("queue").build()?;

        self.pipeline.add_many([
            &src,
            &capsfilter,
            &textoverlay,
            &convert,
            &tee,
            &thumb_queue,
        ])?;

        // Link Black Gen -> Tee
        Element::link_many([&src, &capsfilter, &textoverlay, &convert, &tee])?;

        // Link Tee -> Thumbnail Queue
        Element::link_many([&tee, &thumb_queue])?;

        let compositor_pad = self
            .compositor
            .request_pad_simple("sink_%u")
            .ok_or_else(|| bool_error!("no compositor pad"))?;

        // Link Thumbnail Queue -> Compositor
        thumb_queue
            .static_pad("src")
            .unwrap()
            .link(&compositor_pad)
            .map_err(|e| bool_error!("failed to link convert to compositor: {e}"))?;

        for el in [
            &src,
            &capsfilter,
            &textoverlay,
            &convert,
            &tee,
            &thumb_queue,
        ] {
            el.sync_state_with_parent()?;
        }

        let mut tiles = self.tiles.lock().unwrap();
        let tile = tiles.entry(endpoint_id.to_string()).or_insert_with(|| {
            Tile::new(
                endpoint_id.to_string(),
                nickname.to_string(),
                video_muted,
                audio_muted,
            )
        });

        tile.content = TileContent::BlackTile;
        tile.compositor_pad = Some(compositor_pad);
        tile.caps_filter = Some(capsfilter);
        tile.black_src = Some(src);
        tile.text_overlay = Some(textoverlay);
        tile.convert = Some(convert);

        tile.tee = Some(tee);
        tile.thumb_queue = Some(thumb_queue);

        Ok(())
    }

    fn swap_to_video(&self, endpoint_id: &str) -> Option<Pad> {
        let mut tiles = match self.tiles.lock() {
            Ok(tiles) => tiles,
            Err(err) => {
                error!("failed to acquire tiles lock: {err}");
                return None;
            }
        };

        let tile = tiles.get_mut(endpoint_id)?;

        // Tear down ONLY the black generator elements upstream of the Tee
        for el in [
            tile.black_src.take(),
            tile.caps_filter.take(),
            tile.text_overlay.take(),
            tile.convert.take(),
        ]
        .into_iter()
        .flatten()
        {
            let _ = el.set_state(State::Null);
            let _ = self.pipeline.remove(&el);
        }

        tile.content = TileContent::Camera;

        // DO NOT request a new compositor pad here. The Tee is already routed to the
        // compositor via thumb_queue (and large_queue if dominant).
        // Just return the Tee's sink pad so the incoming WebRTC feed can plug right into it!
        tile.tee.as_ref().and_then(|tee| tee.static_pad("sink"))
    }

    fn swap_to_black_tile(&self, endpoint_id: &str) {
        let mut tiles = match self.tiles.lock() {
            Ok(tiles) => tiles,
            Err(err) => return,
        };

        let tile = match tiles.get_mut(endpoint_id) {
            Some(t) => t,
            None => return,
        };

        // We assume the caller unlinked the WebRTC source from the Tee before calling this.
        if let Some(tee) = &tile.tee {
            // Rebuild the black generator
            let src = ElementFactory::make("videotestsrc")
                .property_from_str("pattern", "black")
                .property("is-live", true)
                .build()
                .unwrap();
            let textoverlay = ElementFactory::make("textoverlay")
                .property("text", &tile.nickname)
                .property_from_str("valignment", "center")
                .property_from_str("halignment", "center")
                .property("font-desc", "Sans Bold 24")
                .build()
                .unwrap();
            let caps = Caps::builder("video/x-raw")
                .field("width", 1920i32)
                .field("height", 1080i32)
                .field("framerate", gstreamer::Fraction::new(30, 1))
                .field("format", "I420")
                .build();
            let capsfilter = ElementFactory::make("capsfilter")
                .property("caps", &caps)
                .build()
                .unwrap();
            let convert = ElementFactory::make("videoconvert").build().unwrap();

            self.pipeline
                .add_many([&src, &capsfilter, &textoverlay, &convert])
                .unwrap();
            Element::link_many([&src, &capsfilter, &textoverlay, &convert]).unwrap();

            // Link to existing Tee
            convert
                .static_pad("src")
                .unwrap()
                .link(&tee.static_pad("sink").unwrap())
                .unwrap();

            for el in [&src, &capsfilter, &textoverlay, &convert] {
                el.sync_state_with_parent().unwrap();
            }

            tile.black_src = Some(src);
            tile.caps_filter = Some(capsfilter);
            tile.text_overlay = Some(textoverlay);
            tile.convert = Some(convert);
            tile.content = TileContent::BlackTile;
        }
    }

    fn add_screenshare_pad(&self, endpoint_id: &str) -> Option<Pad> {
        let pad = self.compositor.request_pad_simple("sink_%u")?;
        let mut tiles = match self.tiles.lock() {
            Ok(tiles) => tiles,
            Err(_) => return None,
        };

        if let Some(tile) = tiles.get_mut(endpoint_id) {
            tile.screenshare_compositor_pad = Some(pad.clone());
            tile.has_screenshare = true;
        }

        self.recalculate_layout();
        Some(pad)
    }

    fn remove_screenshare_pad(&self, endpoint_id: &str) {
        let mut tiles = match self.tiles.lock() {
            Ok(tiles) => tiles,
            Err(_) => return,
        };

        if let Some(tile) = tiles.get_mut(endpoint_id) {
            if let Some(pad) = tile.screenshare_compositor_pad.take() {
                self.compositor.release_request_pad(&pad);
            }
            tile.has_screenshare = false;
        }
    }

    fn remove_tile(&self, endpoint_id: &str) {
        let mut tiles = match self.tiles.lock() {
            Ok(tiles) => tiles,
            Err(_) => return,
        };

        if let Some(mut tile) = tiles.remove(endpoint_id) {
            // Clean up ALL elements associated with this participant
            for el in [
                tile.black_src.take(),
                tile.caps_filter.take(),
                tile.text_overlay.take(),
                tile.convert.take(),
                tile.thumb_queue.take(),
                tile.large_queue.take(),
                tile.tee.take(),
            ]
            .into_iter()
            .flatten()
            {
                let _ = el.set_state(State::Null);
                let _ = self.pipeline.remove(&el);
            }

            // Release ALL requested compositor pads
            if let Some(pad) = tile.compositor_pad.take() {
                self.compositor.release_request_pad(&pad);
            }
            if let Some(pad) = tile.large_pad.take() {
                self.compositor.release_request_pad(&pad);
            }
            if let Some(pad) = tile.screenshare_compositor_pad.take() {
                self.compositor.release_request_pad(&pad);
            }
        }
    }

    fn recalculate_layout(&self) {
        let tiles = match self.tiles.lock() {
            Ok(tiles) => tiles,
            Err(err) => {
                error!("failed to acquire tiles lock: {err}");
                return;
            }
        };
        let dominant = match self.dominant_speaker.lock() {
            Ok(dominant) => dominant,
            Err(err) => {
                error!("failed to acquire dominant lock: {err}");
                return;
            }
        };

        let tile_info: Vec<(&String, bool, bool)> = tiles
            .iter()
            .map(|(ep, t)| {
                let is_dom = dominant.as_deref() == Some(ep.as_str());
                (ep, t.has_screenshare, is_dom)
            })
            .collect();

        let rects = LayoutEngine::calculate(&tile_info, dominant.as_deref());

        LayoutEngine::apply(
            &rects,
            |ep, is_screenshare| {
                tiles.get(ep).and_then(|t| {
                    if is_screenshare {
                        t.screenshare_compositor_pad.clone()
                    } else {
                        t.compositor_pad.clone()
                    }
                })
            },
            |ep| tiles.get(ep).and_then(|t| t.large_pad.clone()),
        );
    }
}
