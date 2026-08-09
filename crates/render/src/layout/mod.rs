//! Deciding what is on screen, and when it changes.
//!
//! A [`Layout`] answers "what should be visible at this instant"; the layout
//! itself is stateless. [`segment_with`] turns that into spans by evaluating
//! the layout between every pair of interesting timestamps and merging
//! neighbours that produce identical placements.

use crate::timeline::{Dominant, Timeline};
pub(crate) mod active_speaker;

/// Output frame size for the composited video.
pub const OUT_WIDTH: i32 = 1280;
pub const OUT_HEIGHT: i32 = 720;

/// Which recording fills a slot: a live camera, a screenshare, or the
/// camera-off placeholder.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Source {
    Camera(String),
    Share(String),
    Avatar(String),
}

/// Position and size in output pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// One source placed on screen. Higher `layer` values are further back — the
/// renderer treats layer 0 as the topmost.
#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    pub source: Source,
    pub rect: Rect,
    pub layer: u32,
}

/// A span over which the layout does not change, in seconds.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
    pub placements: Vec<Placement>,
}

/// A composition strategy: given the meeting state, what is on screen now.
pub(crate) trait Layout {
    fn placements_at(&self, tl: &Timeline, time_sec: f64) -> Vec<Placement>;
}

/// Everyone in the meeting at `time_sec`.
pub fn present(tl: &Timeline, time_sec: f64) -> Vec<&String> {
    tl.participants
        .iter()
        .filter(|(_, p)| p.present_at(time_sec))
        .map(|(pid, _)| pid)
        .collect()
}

/// The dominant speaker at `time_sec`, ignoring one who has already left —
/// the bridge's last speaker report can outlive their departure.
pub fn dominant<'a>(tl: &'a Timeline, time_sec: f64) -> Option<&'a str> {
    tl.dominant
        .iter()
        .find(|d| d.start <= time_sec && time_sec < d.end)
        .map(|d| d.endpoint.as_str())
        .filter(|pid| {
            tl.participants
                .get(*pid)
                .map(|p| p.present_at(time_sec))
                .unwrap_or(false)
        })
}

/// Picks what to show for one participant: their screenshare if
/// `sharing_ok`, otherwise their camera, otherwise their avatar.
pub fn source_for(tl: &Timeline, pid: &str, time_sec: f64, sharing_ok: bool) -> Source {
    if sharing_ok && tl.participants[pid].sharing_at(time_sec) {
        Source::Share(pid.to_string())
    } else if tl.participants[pid].camera_on_at(time_sec) {
        Source::Camera(pid.to_string())
    } else {
        Source::Avatar(pid.to_string())
    }
}

/// Cuts the meeting into segments of constant layout.
///
/// Candidate boundaries are every instant something could change — joins,
/// leaves, camera and share edges, dominant-speaker switches. The layout is
/// then sampled at the *midpoint* of each window, which avoids the ambiguity
/// of evaluating exactly on a boundary, and adjacent windows with identical
/// placements are merged.
pub fn segment_with<L: Layout>(layout: L, tl: &Timeline) -> Vec<Segment> {
    let mut bounds = vec![0.0, tl.end];

    for participant in tl.participants.values() {
        bounds.push(participant.joined);
        bounds.push(participant.left);

        for &(start, end) in participant.camera.iter().chain(&participant.share) {
            bounds.push(start);
            bounds.push(end);
        }
    }

    for &Dominant { start, .. } in &tl.dominant {
        bounds.push(start);
    }

    bounds.retain(|&time_sec| (0.0..=tl.end).contains(&time_sec));
    bounds.sort_by(|a, b| a.partial_cmp(b).unwrap());
    bounds.dedup_by(|a, b| (*a - *b).abs() < 1e-9);

    let mut segments: Vec<Segment> = vec![];

    for window in bounds.windows(2) {
        let (bound0, bound1) = (window[0], window[1]);

        if bound1 <= bound0 {
            continue;
        }

        let time_sec = (bound0 + bound1) / 2.0;

        let placements = layout.placements_at(tl, time_sec);

        if placements.is_empty() {
            continue;
        }

        match segments.last_mut() {
            Some(last) if last.placements == placements && (last.end - bound0).abs() < 1e-9 => {
                last.end = bound1;
            }
            _ => segments.push(Segment {
                start: bound0,
                end: bound1,
                placements,
            }),
        }
    }

    segments
}
