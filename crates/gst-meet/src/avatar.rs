//! Renders the placeholder frame shown for a participant whose camera is off.
//!
//! The image mimics the Jitsi web UI: initials in a colored circle over a dark
//! background, with the display name underneath. It is written once per
//! participant and reused by the render pass for every camera-off interval.

use std::{
    fs::File,
    io::{BufWriter, Write},
};

use cairo::{Context, Error, FontSlant, FontWeight, Format, ImageSurface};
use regex::Regex;
use thiserror::Error;
use unicode_segmentation::UnicodeSegmentation;

/// Anything that can go wrong while drawing or writing an avatar.
#[derive(Debug, Error)]
pub enum AvataError {
    #[error("cairo error: {0}")]
    CairoError(#[from] Error),

    #[error("regex error: {0}")]
    RegexError(#[from] regex::Error),

    #[error("io error: {0}")]
    IoError(#[from] cairo::IoError),

    #[error("file create error: {0}")]
    FileCreate(#[from] std::io::Error),
}

/// Circle colors, matching the palette Jitsi uses for its own avatars.
const AVATAR_COLORS: [&str; 9] = [
    "#6A50D3", "#FF9B42", "#DF486F", "#73348C", "#B23683", "#F96E57", "#4380E2", "#238561",
    "#00A8B3",
];

/// Picks a stable color for a set of initials, so the same person always gets
/// the same circle across meetings.
fn avatar_color(initials: &str) -> &'static str {
    let hash: u32 = initials.chars().map(|c| c as u32).sum();

    AVATAR_COLORS[(hash as usize) % AVATAR_COLORS.len()]
}

/// Strips trailing bracketed suffixes from a display name, repeatedly, so
/// `"Ada Lovelace (Guest) [mobile]"` becomes `"Ada Lovelace"`.
fn cleanup_name(nickname: &str) -> String {
    let re = Regex::new(r"\s*[\(\[\{][^()\[\]{}]*[\)\]\}]$").expect("valid regex");

    let mut cleaned = nickname.to_owned();

    while re.is_match(&cleaned) {
        cleaned = re.replace(&cleaned, "").into_owned();
    }

    cleaned
}

/// First grapheme cluster of a word, uppercased — a cluster rather than a
/// `char` so emoji and combining marks survive intact.
fn first_grapheme(word: &str) -> String {
    UnicodeSegmentation::graphemes(word, true)
        .next()
        .unwrap_or("")
        .to_uppercase()
}

/// Builds the one- or two-letter monogram: first letter of the first word, plus
/// first letter of the last word when the name has more than one.
fn initials(name: &str) -> String {
    let words = match Regex::new(r"\s*[(\[{][^)\]}]*[)\]}]$") {
        Ok(re) => re
            .split(name)
            .filter(|w| !w.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>(),

        Err(_) => vec![name.chars().next().unwrap_or_default().to_string()],
    };

    let first = words.first().map(|s| first_grapheme(s)).unwrap_or_default();

    let last = if words.len() > 1 {
        first_grapheme(words.last().unwrap())
    } else {
        String::new()
    };

    format!("{first}{last}")
}

/// Converts `#rrggbb` to the 0.0–1.0 components cairo expects.
///
/// Panics on malformed input; the only inputs are the [`AVATAR_COLORS`]
/// literals.
fn hex_to_rgb(hex: &str) -> (f64, f64, f64) {
    let hex = hex.trim_start_matches('#');

    let r = u8::from_str_radix(&hex[0..2], 16).unwrap();
    let g = u8::from_str_radix(&hex[2..4], 16).unwrap();
    let b = u8::from_str_radix(&hex[4..6], 16).unwrap();

    (r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0)
}

/// Draws `nickname`'s avatar and writes it to `path` as a 1920x1080 PNG —
/// full frame size, so the render pass can drop it straight into a tile.
///
/// The parent directory must already exist.
pub fn generate_avatar(nickname: &str, path: &str) -> Result<(), AvataError> {
    const WIDTH: i32 = 1920;
    const HEIGHT: i32 = 1080;

    let surface = ImageSurface::create(Format::ARgb32, WIDTH, HEIGHT)?;
    let cr = Context::new(&surface)?;

    // Background
    cr.set_source_rgb(0.12, 0.14, 0.18);
    cr.paint()?;

    // Avatar info
    let name = cleanup_name(nickname);
    let initials = initials(&name);

    let color = avatar_color(&initials);
    let (r, g, b) = hex_to_rgb(color);

    // Circle
    let radius = 180.0;
    let cx = WIDTH as f64 / 2.0;
    let cy = HEIGHT as f64 / 2.0 - 80.0;

    cr.arc(cx, cy, radius, 0.0, std::f64::consts::PI * 2.0);
    cr.set_source_rgb(r, g, b);
    cr.fill()?;

    // Initials (white)
    cr.select_font_face("Sans", FontSlant::Normal, FontWeight::Bold);
    cr.set_font_size(140.0);
    cr.set_source_rgb(1.0, 1.0, 1.0);

    let extents = cr.text_extents(&initials)?;

    cr.move_to(
        cx - (extents.width() / 2.0 + extents.x_bearing()),
        cy - (extents.height() / 2.0 + extents.y_bearing()),
    );

    cr.show_text(&initials)?;

    // Participant name
    cr.set_font_size(60.0);

    let extents = cr.text_extents(&name)?;

    cr.move_to(
        cx - (extents.width() / 2.0 + extents.x_bearing()),
        HEIGHT as f64 - 120.0,
    );

    cr.show_text(&name)?;

    surface.flush();

    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);

    surface.write_to_png(&mut writer)?;
    writer.flush()?;

    Ok(())
}
