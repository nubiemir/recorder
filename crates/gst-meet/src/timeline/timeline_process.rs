use serde::Serialize;
use serde_json::json;

use crate::timeline::Timeline;
use std::{
    fs::{self, File},
    io,
};

pub fn read_file(path: &str) -> Timeline {
    let file = File::open(path).unwrap();
    let timeline: Timeline = serde_json::from_reader(file).unwrap();
    timeline
}

pub fn write_file<T: Serialize>(path: &str, content: T) -> io::Result<()> {
    let contents = json!(content);
    fs::write(path, contents.to_string())?;
    Ok(())
}
