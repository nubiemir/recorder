//! Reading and writing the JSON artifacts that accompany a recording.

use serde::Serialize;
use serde_json::json;

use crate::timeline::Timeline;
use std::{
    fs::{self, File},
    io,
};

/// Reads a `timeline.json` back into memory.
///
/// Unused today (hence the leading underscore) and panics on any failure;
/// intended for tests and for a render pass that reads a recorded meeting.
pub fn _read_file(path: &str) -> Timeline {
    let file = File::open(path).unwrap();
    let timeline: Timeline = serde_json::from_reader(file).unwrap();
    timeline
}

/// Serializes `content` to `path` as JSON, replacing any existing file.
pub fn write_file<T: Serialize>(path: &str, content: T) -> io::Result<()> {
    let contents = json!(content);
    fs::write(path, contents.to_string())?;
    Ok(())
}
