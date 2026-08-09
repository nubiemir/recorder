use gstreamer::{
    Bin, Element, ElementFactory, GhostPad, Pad, PadProbeReturn, PadProbeType, Pipeline, State,
    event::Eos,
    glib::{
        self, BoolError,
        object::{Cast, ObjectExt},
    },
    prelude::{
        ElementExt, ElementExtManual, GstBinExt, GstBinExtManual, GstObjectExt, PadExt,
        PadExtManual,
    },
};
use log::{error, info, warn};

use crate::participant::SourceEntry;

/// One recording chain: `queue -> depay -> [parse] -> matroskamux -> filesink`,
/// wrapped in a bin so its EOS is forwarded to the room's bus.
#[derive(Debug)]
pub struct Branch {
    pub bin: Bin,
    pub src_pad: Pad,
    pub entry_pad: Pad,
    pub filesink: Element,
    pub path: String,
    pub finalizing: bool,
}

impl Branch {
    /// Builds the chain for one SSRC. The bin is returned unlinked and in the
    /// Null state; the caller adds it to the pipeline and links `src_pad` to
    /// `entry_pad`.
    pub fn build(
        ssrc: u32,
        src_pad: Pad,
        encoding: &str,
        source: &SourceEntry,
        room_name: &str,
    ) -> Result<Self, BoolError> {
        let bin = Bin::builder()
            .name(format!("branch_{}", ssrc))
            .property("message-forward", true)
            .build();

        let queue = ElementFactory::make("queue")
            .name(format!("queue_{}", ssrc))
            .build()?;

        let depay = match encoding {
            "AV1" => ElementFactory::make("rtpav1depay").build()?,
            "VP8" => ElementFactory::make("rtpvp8depay").build()?,
            "VP9" => ElementFactory::make("rtpvp9depay").build()?,
            "H264" => ElementFactory::make("rtph264depay").build()?,
            "OPUS" => ElementFactory::make("rtpopusdepay").build()?,
            _ => return Err(glib::bool_error!("unsupported encoding {encoding}")),
        };

        let parse = match encoding {
            "AV1" => Some(ElementFactory::make("av1parse").build()?),
            "H264" => Some(ElementFactory::make("h264parse").build()?),
            "OPUS" => Some(ElementFactory::make("opusparse").build()?),
            _ => None,
        };

        let (muxer, ext) = {
            let muxer = ElementFactory::make("matroskamux").build()?;
            muxer.set_property("min-index-interval", 1_000_000_000i64);
            muxer.set_property("offset-to-zero", true);
            (muxer, "mkv")
        };

        let filesink = ElementFactory::make("filesink").build()?;
        let path = format!(
            "recordings/{}/{}/{}.{}",
            room_name, source.endpoint, source.kind, ext
        );
        filesink.set_property("location", &path);

        bin.add_many([&queue, &depay, &muxer, &filesink])?;
        if let Some(parse) = &parse {
            bin.add(parse)?;
            Element::link_many([&queue, &depay, parse, &muxer])?;
        } else {
            Element::link_many([&queue, &depay, &muxer])?;
        }
        Element::link(&muxer, &filesink)?;

        let queue_sink = queue.static_pad("sink").unwrap();
        let ghost = GhostPad::with_target(&queue_sink)?;
        ghost.set_active(true)?;
        bin.add_pad(&ghost)?;

        Ok(Self {
            bin,
            src_pad,
            entry_pad: ghost.upcast(),
            filesink,
            path,
            finalizing: false,
        })
    }

    /// True when `src` is this branch's filesink, i.e. the EOS on the bus
    /// belongs to this recording.
    pub fn owns_filesink(&self, src: &gstreamer::Object) -> bool {
        self.filesink.upcast_ref::<gstreamer::Object>() == src
    }

    /// True when `src` is any element inside this branch, used to attribute
    /// bus errors.
    pub fn contains(&self, src: &gstreamer::Object) -> bool {
        src.has_as_ancestor(&self.bin)
    }

    /// Cuts the branch off from the webrtcbin pad and pushes EOS so the muxer
    /// writes its index and closes the file. Idempotent.
    pub fn finalize(&mut self) {
        if self.finalizing {
            return;
        }
        self.finalizing = true;
        info!("finalizing recording {}", self.path);
        self.detach();
    }

    fn detach(&self) {
        self.src_pad
            .add_probe(PadProbeType::DATA_DOWNSTREAM, |_pad, _info| {
                PadProbeReturn::Drop
            });

        if let Err(err) = self.src_pad.unlink(&self.entry_pad) {
            warn!("failed to unlink branch {}: {err:?}", self.path);
        }

        if !self.entry_pad.send_event(Eos::new()) {
            error!(
                "failed to send EOS to branch {}; file may not finalize",
                self.path
            );
        }
    }

    /// Removes the branch from the pipeline once its file is closed.
    pub fn teardown(&self, pipeline: &Pipeline) {
        if let Err(err) = pipeline.remove(&self.bin) {
            warn!(
                "failed to remove branch {} from pipeline: {err:?}",
                self.path
            );
        }

        match self.bin.set_state(State::Null) {
            Ok(_) => info!("finalized recording {}", self.path),
            Err(err) => error!("failed to stop branch {}: {err:?}", self.path),
        }
    }
}
