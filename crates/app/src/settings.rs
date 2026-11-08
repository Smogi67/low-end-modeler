//! Mixer's settings, kept in Documents › Mixer › settings.txt as one "name value" line
//! each. The Mixer Launcher reads and writes some of the same lines (`renderer`,
//! `updates`), and each program keeps the other's lines as they are.

use std::path::PathBuf;

use crate::export::{ExportFormat, write_whole};
use crate::places::mixer_folder;

/// Choices offered for the interface size, in percent.
pub const INTERFACE_SIZES: [u32; 3] = [100, 125, 150];
/// Choices offered for auto-save, in minutes; 0 is off.
pub const AUTOSAVE_CHOICES: [u32; 4] = [0, 2, 5, 10];
pub const FEWEST_UNDO_STEPS: u32 = 16;
pub const MOST_UNDO_STEPS: u32 = 1000;
pub const FEWEST_RECENT: u32 = 3;
pub const MOST_RECENT: usize = 20;
pub const SLOWEST_TURN: u32 = 25;
pub const FASTEST_TURN: u32 = 300;

/// How Mixer may draw. Only OpenGL 2.1 is built so far (Stage 10 step 4 adds the rest).
pub const RENDERERS: [(&str, &str, &str); 4] = [
    (
        "opengl2.1",
        "OpenGL 2.1",
        "Works on almost every PC, including old ones.",
    ),
    (
        "vulkan",
        "Vulkan",
        "Modern and fast on newer graphics cards.",
    ),
    ("directx12", "DirectX 12", "Windows' own modern engine."),
    (
        "opengl3.3",
        "OpenGL 3.3",
        "Newer OpenGL, through the same system as Vulkan.",
    ),
];

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    // Display
    /// Smooth edges. Takes effect when Mixer next starts.
    pub antialiasing: bool,
    /// One of `RENDERERS`' keys. Takes effect when Mixer next starts.
    pub renderer: String,
    /// Interface size in percent: 100, 125 or 150.
    pub interface: u32,
    pub grid: bool,
    /// Shapes get darker on the sides turned away from the light.
    pub shadows: bool,
    // Mouse
    /// Two-finger scrolling turns the view; Shift slides it, Ctrl zooms.
    pub touchpad: bool,
    /// Zooming moves toward what is under the pointer.
    pub zoom_to_pointer: bool,
    pub invert_zoom: bool,
    /// How fast dragging turns the view, in percent.
    pub turn_speed: u32,
    // Saving
    /// Minutes between automatic saves; 0 is off.
    pub autosave: u32,
    pub undo_steps: u32,
    pub recent: u32,
    pub start_window: bool,
    // Export and updates
    /// For new projects.
    pub export_format: ExportFormat,
    pub export_scale: f32,
    /// Show the exported file in Explorer.
    pub open_export_folder: bool,
    /// The launcher checks GitHub when it opens.
    pub check_updates: bool,
    /// Lines Mixer does not use itself.
    others: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            antialiasing: true,
            renderer: "opengl2.1".into(),
            interface: 100,
            grid: true,
            shadows: true,
            touchpad: false,
            zoom_to_pointer: false,
            invert_zoom: false,
            turn_speed: 100,
            autosave: 5,
            undo_steps: mixer_mesh::DEFAULT_MAX_STEPS as u32,
            recent: 8,
            start_window: true,
            export_format: ExportFormat::Obj,
            export_scale: 1.0,
            open_export_folder: false,
            check_updates: true,
            others: Vec::new(),
        }
    }
}

fn on(value: &str) -> bool {
    value != "off"
}

fn word(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}

impl Settings {
    fn path() -> PathBuf {
        mixer_folder().join("settings.txt")
    }

    /// Reads the settings, or the defaults if there are none or they are unreadable.
    pub fn load() -> Self {
        let text = std::fs::read_to_string(Self::path()).unwrap_or_default();
        Self::parse(&text)
    }

    pub fn parse(text: &str) -> Self {
        let mut s = Self::default();
        for line in text.lines() {
            let mut words = line.split_whitespace();
            let (Some(key), Some(value)) = (words.next(), words.next()) else {
                continue;
            };
            let number = |low: u32, high: u32, fallback: u32| {
                value
                    .parse::<u32>()
                    .map_or(fallback, |n| n.clamp(low, high))
            };
            match key {
                "antialiasing" => s.antialiasing = on(value),
                "renderer" if RENDERERS.iter().any(|r| r.0 == value) => {
                    s.renderer = value.to_string();
                }
                "interface" => {
                    let wanted = number(100, 150, 100);
                    s.interface = INTERFACE_SIZES
                        .into_iter()
                        .min_by_key(|size| size.abs_diff(wanted))
                        .unwrap_or(100);
                }
                "grid" => s.grid = on(value),
                "shadows" => s.shadows = on(value),
                "touchpad" => s.touchpad = on(value),
                "zoomtopointer" => s.zoom_to_pointer = on(value),
                "invertzoom" => s.invert_zoom = on(value),
                "turnspeed" => s.turn_speed = number(SLOWEST_TURN, FASTEST_TURN, 100),
                "autosave" => s.autosave = number(0, 60, 5),
                "undosteps" => {
                    s.undo_steps = number(FEWEST_UNDO_STEPS, MOST_UNDO_STEPS, s.undo_steps);
                }
                "recent" => s.recent = number(FEWEST_RECENT, MOST_RECENT as u32, 8),
                "startwindow" => s.start_window = on(value),
                "exportformat" => {
                    if let Some(format) = ExportFormat::from_name(value) {
                        s.export_format = format;
                    }
                }
                "exportscale" => {
                    if let Ok(scale) = value.parse::<f32>()
                        && scale.is_finite()
                        && scale > 0.0
                    {
                        s.export_scale = scale.clamp(0.001, 1000.0);
                    }
                }
                "openexportfolder" => s.open_export_folder = on(value),
                "updates" => s.check_updates = on(value),
                _ if key.starts_with('#') => {}
                // A line Mixer does not know, such as one of the launcher's, is kept.
                _ => s.others.push(line.to_string()),
            }
        }
        s
    }

    pub fn text(&self) -> String {
        let mut text = String::from("# Mixer settings\n");
        let lines = [
            ("antialiasing", word(self.antialiasing).to_string()),
            ("renderer", self.renderer.clone()),
            ("interface", self.interface.to_string()),
            ("grid", word(self.grid).to_string()),
            ("shadows", word(self.shadows).to_string()),
            ("touchpad", word(self.touchpad).to_string()),
            ("zoomtopointer", word(self.zoom_to_pointer).to_string()),
            ("invertzoom", word(self.invert_zoom).to_string()),
            ("turnspeed", self.turn_speed.to_string()),
            ("autosave", self.autosave.to_string()),
            ("undosteps", self.undo_steps.to_string()),
            ("recent", self.recent.to_string()),
            ("startwindow", word(self.start_window).to_string()),
            ("exportformat", self.export_format.extension().to_string()),
            ("exportscale", format!("{}", self.export_scale)),
            (
                "openexportfolder",
                word(self.open_export_folder).to_string(),
            ),
            ("updates", word(self.check_updates).to_string()),
        ];
        for (key, value) in lines {
            text.push_str(&format!("{key} {value}\n"));
        }
        for line in &self.others {
            text.push_str(line);
            text.push('\n');
        }
        text
    }

    pub fn save(&self) -> Result<(), String> {
        write_whole(&Self::path(), self.text().as_bytes())
    }

    /// The interface size as a factor, 1.0 to 1.5.
    pub fn ui_scale(&self) -> f32 {
        self.interface as f32 / 100.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_read_back_exactly() {
        let changed = Settings {
            antialiasing: false,
            interface: 125,
            grid: false,
            shadows: false,
            touchpad: true,
            zoom_to_pointer: true,
            invert_zoom: true,
            turn_speed: 150,
            autosave: 0,
            undo_steps: 64,
            recent: 12,
            start_window: false,
            export_format: ExportFormat::Glb,
            export_scale: 0.25,
            open_export_folder: true,
            check_updates: false,
            ..Settings::default()
        };
        assert_eq!(Settings::parse(&changed.text()), changed);
        assert_eq!(Settings::parse(""), Settings::default());
        assert!(Settings::default().antialiasing);
    }

    #[test]
    fn odd_values_are_brought_into_range_and_other_lines_kept() {
        let s = Settings::parse(
            "interface 140\nturnspeed 9999\nrecent 1\nundosteps abc\nexportscale -3\n\
             renderer nonsense\nlauncherthing 7\n",
        );
        assert_eq!(s.interface, 150);
        assert_eq!(s.turn_speed, FASTEST_TURN);
        assert_eq!(s.recent, FEWEST_RECENT);
        assert_eq!(s.undo_steps, Settings::default().undo_steps);
        assert_eq!(s.export_scale, 1.0);
        assert_eq!(s.renderer, "opengl2.1");
        assert!(s.text().ends_with("launcherthing 7\n"));
        // The launcher's own lines are understood, not duplicated.
        let shared = Settings::parse("renderer vulkan\nupdates off\n");
        assert_eq!(shared.renderer, "vulkan");
        assert!(!shared.check_updates);
        assert_eq!(shared.text().matches("updates").count(), 1);
    }
}
