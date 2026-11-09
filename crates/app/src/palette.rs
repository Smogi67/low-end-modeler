//! Every color Mixer draws with, which Settings > Theme lets people change. The
//! interface reads them through `theme::color`, the 3D view through `get`. Saved in
//! Documents › Mixer › theme.txt, one "name #rrggbbaa" line each; a missing or odd line
//! keeps the default.

use std::path::PathBuf;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::export::write_whole;
use crate::places::mixer_folder;

/// One color the theme sets.
pub struct Entry {
    pub key: &'static str,
    pub label: &'static str,
    pub group: &'static str,
    pub default: [f32; 4],
}

const fn e(key: &'static str, label: &'static str, group: &'static str, c: [f32; 4]) -> Entry {
    Entry {
        key,
        label,
        group,
        default: c,
    }
}

pub const INTERFACE: &str = "Interface";
pub const VIEW: &str = "3D view";
pub const SHAPES: &str = "Shapes";

pub const WINDOW: usize = 0;
pub const BAR: usize = 1;
pub const BORDER: usize = 2;
pub const FRAME: usize = 3;
pub const FRAME_HOVER: usize = 4;
pub const FRAME_ACTIVE: usize = 5;
pub const TEXT: usize = 6;
pub const MUTED: usize = 7;
pub const FAINT: usize = 8;
pub const ACCENT: usize = 9;
pub const ON_ACCENT: usize = 10;
pub const WARNING: usize = 11;
pub const SUCCESS: usize = 12;
pub const DANGER: usize = 13;
pub const OVERLAY: usize = 14;
pub const POPUP: usize = 15;
pub const BACKGROUND_TOP: usize = 16;
pub const BACKGROUND_BOTTOM: usize = 17;
pub const GRID_MINOR: usize = 18;
pub const GRID_MAJOR: usize = 19;
pub const X_AXIS: usize = 20;
pub const Y_AXIS: usize = 21;
pub const Z_AXIS: usize = 22;
pub const FACE: usize = 23;
pub const FACE_BACK: usize = 24;
pub const FACE_SELECTED: usize = 25;
pub const EDGE: usize = 26;
pub const EDGE_SELECTED: usize = 27;
pub const EDGE_SHARP: usize = 28;
pub const EDGE_PLAIN: usize = 29;
pub const COUNT: usize = 30;

/// The default theme: Mixer's dark look.
pub const ENTRIES: [Entry; COUNT] = [
    e("window", "Panels", INTERFACE, [0.118, 0.125, 0.137, 0.97]),
    e(
        "bar",
        "Top and bottom bars",
        INTERFACE,
        [0.090, 0.094, 0.106, 1.0],
    ),
    e("border", "Borders", INTERFACE, [0.20, 0.21, 0.23, 1.0]),
    e(
        "frame",
        "Buttons and fields",
        INTERFACE,
        [0.165, 0.173, 0.192, 1.0],
    ),
    e(
        "framehover",
        "Buttons, pointed at",
        INTERFACE,
        [0.212, 0.224, 0.247, 1.0],
    ),
    e(
        "frameactive",
        "Buttons, pressed",
        INTERFACE,
        [0.255, 0.267, 0.294, 1.0],
    ),
    e("text", "Text", INTERFACE, [0.91, 0.92, 0.93, 1.0]),
    e("muted", "Labels", INTERFACE, [0.58, 0.61, 0.65, 1.0]),
    e("faint", "Hints", INTERFACE, [0.40, 0.42, 0.46, 1.0]),
    e("accent", "Highlight", INTERFACE, [0.957, 0.553, 0.157, 1.0]),
    e(
        "onaccent",
        "Text on highlight",
        INTERFACE,
        [0.10, 0.07, 0.04, 1.0],
    ),
    e("warning", "Warnings", INTERFACE, [0.98, 0.76, 0.30, 1.0]),
    e("success", "Good news", INTERFACE, [0.50, 0.78, 0.29, 1.0]),
    e("danger", "Problems", INTERFACE, [0.95, 0.40, 0.40, 1.0]),
    e(
        "overlay",
        "Hints over the view",
        INTERFACE,
        [0.08, 0.085, 0.095, 0.88],
    ),
    e("popup", "Menus", INTERFACE, [0.13, 0.137, 0.15, 0.99]),
    e(
        "backtop",
        "Background, top",
        VIEW,
        [0.235, 0.245, 0.265, 1.0],
    ),
    e(
        "backbottom",
        "Background, bottom",
        VIEW,
        [0.135, 0.14, 0.15, 1.0],
    ),
    e("gridminor", "Grid lines", VIEW, [0.31, 0.32, 0.345, 1.0]),
    e(
        "gridmajor",
        "Grid, every tenth line",
        VIEW,
        [0.40, 0.41, 0.44, 1.0],
    ),
    e("xaxis", "X axis", VIEW, [0.93, 0.33, 0.38, 1.0]),
    e("yaxis", "Y axis", VIEW, [0.55, 0.80, 0.25, 1.0]),
    e("zaxis", "Z axis", VIEW, [0.30, 0.56, 0.96, 1.0]),
    e("face", "Faces", SHAPES, [0.74, 0.75, 0.77, 1.0]),
    e(
        "faceback",
        "Faces seen from behind",
        SHAPES,
        [0.62, 0.20, 0.20, 1.0],
    ),
    e(
        "faceselected",
        "Selected faces (strength in opacity)",
        SHAPES,
        [1.0, 0.64, 0.30, 0.40],
    ),
    e("edge", "Edges", SHAPES, [0.11, 0.115, 0.13, 1.0]),
    e(
        "edgeselected",
        "Selected edges and points",
        SHAPES,
        [1.0, 0.62, 0.18, 1.0],
    ),
    e("edgesharp", "Sharp edges", SHAPES, [0.15, 0.72, 0.95, 1.0]),
    e(
        "edgeplain",
        "Edge lines in View mode",
        SHAPES,
        [0.30, 0.31, 0.34, 1.0],
    ),
];

pub type Colors = [[f32; 4]; COUNT];

pub fn defaults() -> Colors {
    std::array::from_fn(|i| ENTRIES[i].default)
}

static CURRENT: RwLock<Option<Colors>> = RwLock::new(None);
static VERSION: AtomicU64 = AtomicU64::new(0);

/// Goes up each time the theme changes, so the 3D view knows to remake its grid.
pub fn version() -> u64 {
    VERSION.load(Ordering::Relaxed)
}

/// A color of the theme in use.
pub fn get(i: usize) -> [f32; 4] {
    match CURRENT.read() {
        Ok(guard) => guard.as_ref().map_or(ENTRIES[i].default, |c| c[i]),
        Err(_) => ENTRIES[i].default,
    }
}

pub fn rgb(i: usize) -> [f32; 3] {
    let [r, g, b, _] = get(i);
    [r, g, b]
}

/// Puts a theme in use.
pub fn set(colors: &Colors) {
    if let Ok(mut guard) = CURRENT.write() {
        *guard = Some(*colors);
    }
    VERSION.fetch_add(1, Ordering::Relaxed);
}

/// The theme in use.
pub fn current() -> Colors {
    std::array::from_fn(get)
}

fn path() -> PathBuf {
    mixer_folder().join("theme.txt")
}

fn hex(c: [f32; 4]) -> String {
    let b = c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
    format!("#{:02x}{:02x}{:02x}{:02x}", b[0], b[1], b[2], b[3])
}

fn from_hex(text: &str) -> Option<[f32; 4]> {
    let digits = text.strip_prefix('#')?;
    if !(digits.len() == 6 || digits.len() == 8) || !digits.is_ascii() {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&digits[i..i + 2], 16).ok();
    let alpha = if digits.len() == 8 { byte(6)? } else { 255 };
    Some([byte(0)?, byte(2)?, byte(4)?, alpha].map(|b| b as f32 / 255.0))
}

pub fn text(colors: &Colors) -> String {
    let mut out =
        String::from("# Mixer theme: one color per line. Delete this file for the default.\n");
    for (entry, color) in ENTRIES.iter().zip(colors) {
        out.push_str(&format!("{} {}\n", entry.key, hex(*color)));
    }
    out
}

pub fn parse(text: &str) -> Colors {
    let mut colors = defaults();
    for line in text.lines() {
        let mut words = line.split_whitespace();
        let (Some(key), Some(value)) = (words.next(), words.next()) else {
            continue;
        };
        if let (Some(i), Some(color)) = (ENTRIES.iter().position(|e| e.key == key), from_hex(value))
        {
            colors[i] = color;
        }
    }
    colors
}

pub fn load() -> Colors {
    std::fs::read_to_string(path()).map_or_else(|_| defaults(), |text| parse(&text))
}

/// Saves the theme; the default theme removes the file instead.
pub fn save(colors: &Colors) -> Result<(), String> {
    let same = colors
        .iter()
        .zip(&defaults())
        .all(|(a, b)| hex(*a) == hex(*b));
    if same {
        let _ = std::fs::remove_file(path());
        return Ok(());
    }
    write_whole(&path(), text(colors).as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn themes_read_back_and_odd_lines_keep_the_default() {
        let mut colors = defaults();
        colors[ACCENT] = [0.2, 0.4, 0.8, 1.0];
        colors[FACE_SELECTED] = [1.0, 0.0, 0.0, 0.5];
        let back = parse(&text(&colors));
        for (a, b) in back.iter().zip(&colors) {
            for k in 0..4 {
                assert!((a[k] - b[k]).abs() < 1.0 / 255.0);
            }
        }
        let odd = parse("accent #12\nface nothing\nnope #ffffff\ntext #ff000080\n");
        assert_eq!(odd[ACCENT], ENTRIES[ACCENT].default);
        assert_eq!(odd[TEXT], [1.0, 0.0, 0.0, 128.0 / 255.0]);
        assert_eq!(parse(""), defaults());
        // Keys are unique, so every line finds its color.
        let mut keys: Vec<_> = ENTRIES.iter().map(|e| e.key).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), COUNT);
    }
}
