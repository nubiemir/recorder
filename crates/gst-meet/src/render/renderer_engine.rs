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

    /// Called from presence handler when a real participant is seen
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

        // always add black tile first so tile exists before promote_to_dominant
        if let Err(e) = self.add_black_tile_for(endpoint_id, nickname, video_muted, audio_muted) {
            error!("failed to add black tile for {}: {:?}", endpoint_id, e);
            return;
        }

        info!(
            "endpoint_id: {} | nickname: {} | video_muted: {} | audio_muted: {}",
            endpoint_id, nickname, video_muted, audio_muted
        );

        // NOW promote — tile exists so large_pad can be stored on it
        let no_dominant = self.dominant_speaker.lock().unwrap().is_none();
        if no_dominant {
            info!("setting initial dominant speaker → {}", endpoint_id);
            self.promote_to_dominant(endpoint_id);
        }

        self.recalculate_layout();
    }

    /// Called from presence type="unavailable"
    pub fn on_participant_left(&self, endpoint_id: &str) {
        info!("participant left endpoint={}", endpoint_id);
        self.remove_tile(endpoint_id);
        self.recalculate_layout();
    }

    // ── Source info / mute state ──────────────────────────────────────────

    /// Called every time presence SourceInfo changes
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

        // camera just turned off while video was active → swap to black tile
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

    // ── SSRC map (from source-add jingle) ────────────────────────────────

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

    // ── RTP stream arrived (from on_incoming_stream) ──────────────────────

    /// Call this when webrtcbin gives us a new src pad for a participant.
    /// Returns the compositor sink pad to link into.
    pub fn on_video_stream_arrived(&self, endpoint_id: &str, is_screenshare: bool) -> Option<Pad> {
        info!(
            "video stream arrived endpoint={} screenshare={}",
            endpoint_id, is_screenshare
        );

        if is_screenshare {
            return self.add_screenshare_pad(endpoint_id);
        }

        // remove black tile, return new compositor pad for real video
        self.swap_to_video(endpoint_id)
    }

    /// Call this when a video stream disappears (pad removed)
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
        // remove large pad from previous dominant
        let prev = {
            let mut dominant = self.dominant_speaker.lock().unwrap();
            let prev = dominant.clone();
            *dominant = Some(endpoint_id.to_string());
            prev
        };

        if let Some(prev_id) = prev {
            if prev_id != endpoint_id {
                let mut tiles = self.tiles.lock().unwrap();
                if let Some(tile) = tiles.get_mut(&prev_id) {
                    if let Some(pad) = tile.large_pad.take() {
                        self.compositor.release_request_pad(&pad);
                    }
                }
            }
        }

        // add large pad for new dominant
        let large_pad = self.compositor.request_pad_simple("sink_%u");
        let mut tiles = self.tiles.lock().unwrap();
        if let Some(tile) = tiles.get_mut(endpoint_id) {
            tile.large_pad = large_pad;
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

        self.pipeline
            .add_many([&src, &capsfilter, &textoverlay, &convert])?;

        Element::link_many([&src, &capsfilter, &textoverlay, &convert])?;

        let compositor_pad = self
            .compositor
            .request_pad_simple("sink_%u")
            .ok_or_else(|| bool_error!("no compositor pad"))?;

        convert
            .static_pad("src")
            .ok_or_else(|| bool_error!("failed to get static pad"))?
            .link(&compositor_pad)
            .map_err(|e| bool_error!("failed to link convert to compositor: {e}"))?;

        src.sync_state_with_parent()?;
        capsfilter.sync_state_with_parent()?;
        textoverlay.sync_state_with_parent()?;
        convert.sync_state_with_parent()?;

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

        Ok(())
    }

    fn swap_to_black_tile(&self, endpoint_id: &str) {
        let mut tiles = match self.tiles.lock() {
            Ok(tiles) => tiles,
            Err(err) => {
                error!("failed to acquire tiles lock: {err}");
                return;
            }
        };

        if let Some(tile) = tiles.get_mut(endpoint_id) {
            if let Some(pad) = tile.compositor_pad.take() {
                self.compositor.release_request_pad(&pad);
            }
            tile.content = TileContent::BlackTile;
        }

        tiles.get(endpoint_id).and_then(|tile| {
            if let Err(e) = self.add_black_tile_for(
                endpoint_id,
                &tile.nickname,
                tile.video_muted,
                tile.audio_muted,
            ) {
                error!(
                    "failed to add black tile on swap for {}: {:?}",
                    endpoint_id, e
                );
            }

            Some(tile)
        });
    }

    fn swap_to_video(&self, endpoint_id: &str) -> Option<Pad> {
        let mut tiles = match self.tiles.lock() {
            Ok(tiles) => tiles,
            Err(err) => {
                error!("failed to acquire tiles lock: {err}");
                return None;
            }
        };

        if let Some(tile) = tiles.get_mut(endpoint_id) {
            // stop and remove black tile elements
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

            if let Some(pad) = tile.compositor_pad.take() {
                self.compositor.release_request_pad(&pad);
            }

            tile.content = TileContent::Camera;
        }

        let pad = self.compositor.request_pad_simple("sink_%u")?;

        if let Some(tile) = tiles.get_mut(endpoint_id) {
            tile.compositor_pad = Some(pad.clone());
        }

        self.recalculate_layout();
        Some(pad)
    }

    fn add_screenshare_pad(&self, endpoint_id: &str) -> Option<Pad> {
        let pad = self.compositor.request_pad_simple("sink_%u")?;

        let mut tiles = match self.tiles.lock() {
            Ok(tiles) => tiles,
            Err(err) => {
                error!("failed to acquire tiles lock: {err}");
                return None;
            }
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
            Err(err) => {
                error!("failed to acquire tiles lock: {err}");
                return;
            }
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
            Err(err) => {
                error!("failed to acquire tiles lock: {err}");
                return;
            }
        };

        if let Some(mut tile) = tiles.remove(endpoint_id) {
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

            if let Some(pad) = tile.compositor_pad.take() {
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
            |ep| {
                // large pad getter for dominant
                tiles.get(ep).and_then(|t| t.large_pad.clone())
            },
        );
    }
}
