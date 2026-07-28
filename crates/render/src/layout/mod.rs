use crate::timeline::{Dominant, Timeline};
pub(crate) mod active_speaker;

pub const OUT_WIDTH: i32 = 1280;
pub const OUT_HEIGHT: i32 = 720;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Source {
    Camera(String),
    Share(String),
    Avatar(String),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    pub source: Source,
    pub rect: Rect,
    pub layer: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
    pub placements: Vec<Placement>,
}

pub(crate) trait Layout {
    fn placements_at(&self, tl: &Timeline, time_sec: f64) -> Vec<Placement>;
}

pub fn present(tl: &Timeline, time_sec: f64) -> Vec<&String> {
    tl.participants
        .iter()
        .filter(|(_, p)| p.present_at(time_sec))
        .map(|(pid, _)| pid)
        .collect()
}

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

pub fn source_for(tl: &Timeline, pid: &str, time_sec: f64, sharing_ok: bool) -> Source {
    if sharing_ok && tl.participants[pid].sharing_at(time_sec) {
        Source::Share(pid.to_string())
    } else if tl.participants[pid].camera_on_at(time_sec) {
        Source::Camera(pid.to_string())
    } else {
        Source::Avatar(pid.to_string())
    }
}

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
