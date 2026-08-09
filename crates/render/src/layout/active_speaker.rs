//! Speaker-focused layout: one large tile, with a corner thumbnail while
//! someone is screensharing.

use crate::{
    layout::{
        Layout, OUT_HEIGHT, OUT_WIDTH, Placement, Rect, Source, dominant, present, source_for,
    },
    timeline::Timeline,
};

/// Size of the corner thumbnail shown over a screenshare.
const THUMB_WIDTH: i32 = 320;
const THUMB_HEIGHT: i32 = 180;

/// Shows the dominant speaker full-frame; if anyone is screensharing, the
/// share takes the frame and the speaker moves to a bottom-right thumbnail.
#[derive(Debug, Default)]
pub(crate) struct ActiveSpeaker;

impl Layout for ActiveSpeaker {
    /// Returns no placements when nobody is present, or when no dominant
    /// speaker is known — a segment with no placements is skipped entirely.
    fn placements_at(&self, tl: &Timeline, time_sec: f64) -> Vec<Placement> {
        let present = present(tl, time_sec);

        if present.is_empty() {
            return vec![];
        }

        let sharer = present
            .iter()
            .find(|pid| tl.participants[**pid].sharing_at(time_sec))
            .copied();

        let dominant = match dominant(tl, time_sec) {
            Some(d) => d,
            None => return vec![],
        };

        let full = Rect {
            x: 0,
            y: 0,
            w: OUT_WIDTH,
            h: OUT_HEIGHT,
        };

        match sharer {
            Some(sharer) => {
                let thumb = Rect {
                    x: OUT_WIDTH - THUMB_WIDTH,
                    y: OUT_HEIGHT - THUMB_HEIGHT,
                    w: THUMB_WIDTH,
                    h: THUMB_HEIGHT,
                };

                vec![
                    Placement {
                        source: Source::Share(sharer.clone()),
                        rect: full,
                        layer: 1,
                    },
                    Placement {
                        source: source_for(tl, dominant, time_sec, false),
                        rect: thumb,
                        layer: 0,
                    },
                ]
            }
            None => {
                vec![Placement {
                    source: source_for(tl, dominant, time_sec, false),
                    rect: full,
                    layer: 1,
                }]
            }
        }
    }
}
