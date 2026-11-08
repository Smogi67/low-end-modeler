//! The settings window: four pages (Display, Mouse, Saving, Export and updates), with
//! every change saved straight away.

use imgui::{Condition, Drag, SliderFlags, StyleColor, StyleVar, Ui, WindowFlags};

use crate::engines::Report;
use crate::export::ExportFormat;
use crate::settings::{
    AUTOSAVE_CHOICES, FASTEST_TURN, FEWEST_RECENT, FEWEST_UNDO_STEPS, INTERFACE_SIZES, MOST_RECENT,
    MOST_UNDO_STEPS, RENDERERS, SLOWEST_TURN, Settings,
};
use crate::theme::{Fonts, color, icon};
use crate::wgpu_renderer::{Engine, Support};

/// The pages, in order.
pub const PAGES: [(char, &str); 4] = [
    (icon::MONITOR, "Display"),
    (icon::MOUSE, "Mouse"),
    (icon::DRIVE, "Saving"),
    (icon::EXPORT, "Export & updates"),
];

/// The window's own state.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsState {
    pub page: usize,
    pub values: Settings,
    /// What this run of Mixer started with, for "changes next time" notes.
    pub antialiasing_now: bool,
    pub antialiasing_refused: bool,
    /// The engine this run uses...
    pub renderer_now: String,
    /// ...and the one the settings asked for when Mixer started.
    pub renderer_wanted: String,
    /// Which engines this computer offers, once Mixer has checked.
    pub engines: Option<Report>,
}

/// What happened this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsAction {
    /// `values` changed; apply and save them.
    Changed,
    Close,
}

const WIDTH: f32 = 780.0;
const HEIGHT: f32 = 600.0;
const LABEL: f32 = 170.0;

fn heading(ui: &Ui, fonts: &Fonts, text: &str) {
    let font = ui.push_font(fonts.heading);
    ui.text_colored(color::FAINT, text);
    drop(font);
    ui.dummy([0.0, 2.0]);
}

fn note(ui: &Ui, tint: [f32; 4], text: &str) {
    let color = ui.push_style_color(StyleColor::Text, tint);
    ui.text_wrapped(text);
    drop(color);
}

/// A muted label, with the control after it at a fixed column.
fn label(ui: &Ui, text: &str) {
    ui.align_text_to_frame_padding();
    ui.text_colored(color::MUTED, text);
    ui.same_line_with_pos(LABEL);
}

/// A row of choices, one lit. Returns the one clicked.
fn choices(ui: &Ui, id: &str, names: &[String], chosen: usize) -> Option<usize> {
    let mut clicked = None;
    let spacing = ui.push_style_var(StyleVar::ItemSpacing([4.0, 4.0]));
    for (i, name) in names.iter().enumerate() {
        if i > 0 {
            ui.same_line();
        }
        let lit = i == chosen;
        let colors = if lit {
            vec![
                ui.push_style_color(StyleColor::Button, color::ACCENT_SOFT),
                ui.push_style_color(StyleColor::ButtonHovered, color::ACCENT_SOFT),
                ui.push_style_color(StyleColor::Text, color::ACCENT),
            ]
        } else {
            vec![ui.push_style_color(StyleColor::Text, color::MUTED)]
        };
        let width = ui.calc_text_size(name)[0] + 28.0;
        if ui.button_with_size(format!("{name}##{id}{i}"), [width.max(64.0), 30.0]) && !lit {
            clicked = Some(i);
        }
        drop(colors);
    }
    drop(spacing);
    clicked
}

/// Draws the settings window over everything.
pub fn build_settings(ui: &Ui, state: &mut SettingsState, fonts: &Fonts) -> Option<SettingsAction> {
    let display = ui.io().display_size;
    let size = [WIDTH.min(display[0] - 20.0), HEIGHT.min(display[1] - 20.0)];
    let pos = [
        ((display[0] - size[0]) / 2.0).max(10.0),
        ((display[1] - size[1]) / 2.0).max(10.0),
    ];
    ui.get_background_draw_list()
        .add_rect([0.0, 0.0], display, [0.0, 0.0, 0.0, 0.45])
        .filled(true)
        .build();
    let before = state.values.clone();
    let mut action = None;
    let padding = ui.push_style_var(StyleVar::WindowPadding([24.0, 20.0]));
    let rounding = ui.push_style_var(StyleVar::WindowRounding(12.0));
    ui.window("##settings")
        .position(pos, Condition::Always)
        .size(size, Condition::Always)
        .flags(
            WindowFlags::NO_TITLE_BAR
                | WindowFlags::NO_RESIZE
                | WindowFlags::NO_MOVE
                | WindowFlags::NO_COLLAPSE
                | WindowFlags::NO_SAVED_SETTINGS,
        )
        .build(|| {
            // Title, and Close on the right.
            let font = ui.push_font(fonts.title);
            ui.text_colored(color::ACCENT, icon::SETTINGS.to_string());
            ui.same_line_with_spacing(0.0, 12.0);
            ui.text("Settings");
            drop(font);
            let close = "Close##settings";
            ui.same_line_with_pos(ui.content_region_max()[0] - 90.0);
            if ui.button_with_size(close, [90.0, 32.0]) {
                action = Some(SettingsAction::Close);
            }
            ui.dummy([0.0, 6.0]);

            // The pages on the left.
            ui.child_window("##settings_pages")
                .size([190.0, -28.0])
                .build(|| {
                    for (i, (glyph, name)) in PAGES.iter().enumerate() {
                        let lit = state.page == i;
                        let colors = if lit {
                            vec![
                                ui.push_style_color(StyleColor::Button, color::FRAME_ACTIVE),
                                ui.push_style_color(StyleColor::Text, color::TEXT),
                            ]
                        } else {
                            vec![
                                ui.push_style_color(StyleColor::Button, [0.0; 4]),
                                ui.push_style_color(StyleColor::Text, color::MUTED),
                            ]
                        };
                        let align = ui.push_style_var(StyleVar::ButtonTextAlign([0.0, 0.5]));
                        if ui.button_with_size(format!("{glyph}   {name}##page{i}"), [-1.0, 38.0]) {
                            state.page = i;
                        }
                        drop(align);
                        drop(colors);
                    }
                });
            ui.same_line_with_spacing(0.0, 20.0);
            ui.child_window("##settings_page")
                .size([0.0, -28.0])
                .build(|| match state.page {
                    0 => display_page(ui, fonts, state),
                    1 => mouse_page(ui, fonts, &mut state.values),
                    2 => saving_page(ui, fonts, &mut state.values),
                    _ => export_page(ui, fonts, &mut state.values),
                });
            ui.text_colored(
                color::FAINT,
                "Changes are saved straight away. Esc closes this window.",
            );
        });
    drop(rounding);
    drop(padding);
    if action.is_none() && state.values != before {
        action = Some(SettingsAction::Changed);
    }
    action
}

fn display_page(ui: &Ui, fonts: &Fonts, state: &mut SettingsState) {
    let values = &mut state.values;
    heading(ui, fonts, "RENDERING ENGINE");
    let recommended = state
        .engines
        .as_ref()
        .map_or("opengl2.1", Report::recommended);
    for (key, name, description) in RENDERERS {
        if ui.radio_button_bool(format!("{name}##renderer"), values.renderer == key) {
            values.renderer = key.to_string();
        }
        if ui.is_item_hovered() {
            ui.tooltip_text(description);
        }
        let support =
            Engine::from_key(key).and_then(|engine| state.engines.as_ref()?.support(engine));
        let badge = |tint, text: &str| {
            ui.same_line_with_spacing(0.0, 10.0);
            ui.text_colored(tint, text);
        };
        if key == recommended {
            badge(color::SUCCESS, "Recommended");
        }
        if key == state.renderer_now {
            badge(color::ACCENT, "In use");
        }
        // Every engine may be picked, even one that seems missing (owner's choice).
        match support {
            Some(Support::No) => badge(color::WARNING, "May be unsupported"),
            Some(Support::Software) => badge(color::WARNING, "Software only, slow"),
            // The description, when there is room for it; it is in the tooltip too.
            _ => {
                let room =
                    ui.window_pos()[0] + ui.window_content_region_max()[0] - ui.item_rect_max()[0];
                if ui.calc_text_size(description)[0] + 12.0 < room {
                    badge(color::FAINT, description);
                }
            }
        }
    }
    if state.renderer_wanted != state.renderer_now && values.renderer == state.renderer_wanted {
        let name = RENDERERS
            .iter()
            .find(|r| r.0 == state.renderer_wanted)
            .map_or("That engine", |r| r.1);
        note(
            ui,
            color::WARNING,
            &format!(
                "{name} could not start this time, so Mixer uses OpenGL 2.1. \
                 mixer.log, beside Mixer, says why."
            ),
        );
    } else if values.renderer != state.renderer_now {
        note(ui, color::WARNING, "Changes the next time Mixer starts.");
    } else if state.engines.is_none() {
        note(
            ui,
            color::FAINT,
            "Mixer is checking which engines this computer offers; see again next time.",
        );
    }
    ui.dummy([0.0, 8.0]);
    heading(ui, fonts, "PICTURE");
    ui.checkbox("Smooth edges (anti-aliasing)", &mut values.antialiasing);
    if values.antialiasing && state.antialiasing_refused {
        note(
            ui,
            color::FAINT,
            "Your graphics driver did not allow it, so it is off.",
        );
    } else if values.antialiasing != state.antialiasing_now {
        note(ui, color::WARNING, "Changes the next time Mixer starts.");
    } else {
        note(ui, color::FAINT, "Turn off if Mixer feels slow.");
    }
    ui.checkbox("Show the grid", &mut values.grid);
    ui.checkbox("Shadows on shapes", &mut values.shadows);
    if !values.shadows {
        ui.indent_by(28.0);
        ui.checkbox("Edge lines in View mode", &mut values.flat_edges);
        note(
            ui,
            color::FAINT,
            if values.flat_edges {
                "Every side is lit the same; thin lines show the shape."
            } else {
                "Every side is lit the same, so in View mode a shape looks flat."
            },
        );
        ui.unindent_by(28.0);
    }
    ui.dummy([0.0, 4.0]);
    label(ui, "Interface size");
    let names: Vec<String> = INTERFACE_SIZES.iter().map(|s| format!("{s}%")).collect();
    let chosen = INTERFACE_SIZES
        .iter()
        .position(|&s| s == values.interface)
        .unwrap_or(0);
    if let Some(i) = choices(ui, "interface", &names, chosen) {
        values.interface = INTERFACE_SIZES[i];
    }
}

fn mouse_page(ui: &Ui, fonts: &Fonts, values: &mut Settings) {
    heading(ui, fonts, "TOUCHPAD");
    ui.checkbox("Touchpad mode", &mut values.touchpad);
    note(
        ui,
        color::FAINT,
        "Two-finger scrolling turns the view. Hold Shift while scrolling to slide it, or \
         Ctrl to zoom.",
    );
    ui.dummy([0.0, 8.0]);
    heading(ui, fonts, "ZOOM AND TURN");
    ui.checkbox("Zoom toward the mouse pointer", &mut values.zoom_to_pointer);
    ui.checkbox("Invert zoom direction", &mut values.invert_zoom);
    ui.dummy([0.0, 4.0]);
    label(ui, "Turn speed");
    ui.set_next_item_width(-1.0);
    let mut speed = values.turn_speed as i32;
    if ui
        .slider_config("##turn_speed", SLOWEST_TURN as i32, FASTEST_TURN as i32)
        .display_format("%d%%")
        .flags(SliderFlags::ALWAYS_CLAMP)
        .build(&mut speed)
    {
        values.turn_speed = speed.clamp(SLOWEST_TURN as i32, FASTEST_TURN as i32) as u32;
    }
    ui.dummy([0.0, 8.0]);
    note(
        ui,
        color::FAINT,
        "Always available: right-drag or Alt+drag turns the view, middle-drag or \
         Shift+Alt+drag slides it, and Ctrl+Alt+drag or the wheel zooms.",
    );
}

fn saving_page(ui: &Ui, fonts: &Fonts, values: &mut Settings) {
    heading(ui, fonts, "AUTO-SAVE");
    label(ui, "Save every");
    let names: Vec<String> = AUTOSAVE_CHOICES
        .iter()
        .map(|&m| {
            if m == 0 {
                "Off".to_string()
            } else {
                format!("{m} min")
            }
        })
        .collect();
    let chosen = AUTOSAVE_CHOICES
        .iter()
        .position(|&m| m == values.autosave)
        .unwrap_or(0);
    if let Some(i) = choices(ui, "autosave", &names, chosen) {
        values.autosave = AUTOSAVE_CHOICES[i];
    }
    note(
        ui,
        color::FAINT,
        "Saves the open project when it has changes, as Ctrl+S does. Imported OBJ files \
         are not saved over.",
    );
    ui.dummy([0.0, 8.0]);
    heading(ui, fonts, "HISTORY");
    label(ui, "Undo steps");
    ui.set_next_item_width(-1.0);
    let mut steps = values.undo_steps as i32;
    if ui
        .slider_config(
            "##undo_steps",
            FEWEST_UNDO_STEPS as i32,
            MOST_UNDO_STEPS as i32,
        )
        .flags(SliderFlags::ALWAYS_CLAMP | SliderFlags::LOGARITHMIC)
        .build(&mut steps)
    {
        values.undo_steps = steps.clamp(FEWEST_UNDO_STEPS as i32, MOST_UNDO_STEPS as i32) as u32;
    }
    label(ui, "Recent projects");
    ui.set_next_item_width(-1.0);
    let mut recent = values.recent as i32;
    if ui
        .slider_config("##recent", FEWEST_RECENT as i32, MOST_RECENT as i32)
        .flags(SliderFlags::ALWAYS_CLAMP)
        .build(&mut recent)
    {
        values.recent = recent.clamp(FEWEST_RECENT as i32, MOST_RECENT as i32) as u32;
    }
    ui.dummy([0.0, 8.0]);
    heading(ui, fonts, "STARTING");
    ui.checkbox(
        "Show the start window when Mixer opens",
        &mut values.start_window,
    );
    if !values.start_window {
        note(
            ui,
            color::FAINT,
            "Mixer then opens your most recent project, or the start window if there is none.",
        );
    }
}

fn export_page(ui: &Ui, fonts: &Fonts, values: &mut Settings) {
    heading(ui, fonts, "NEW PROJECTS");
    label(ui, "Export as");
    let formats = [ExportFormat::Obj, ExportFormat::Glb];
    let names = vec!["OBJ".to_string(), "glTF".to_string()];
    let chosen = formats
        .iter()
        .position(|&f| f == values.export_format)
        .unwrap_or(0);
    if let Some(i) = choices(ui, "format", &names, chosen) {
        values.export_format = formats[i];
    }
    label(ui, "Scale");
    ui.set_next_item_width(-1.0);
    Drag::new("##export_scale")
        .range(0.001, 1000.0)
        .speed(0.01)
        .display_format("%.3f")
        .flags(SliderFlags::ALWAYS_CLAMP)
        .build(ui, &mut values.export_scale);
    note(
        ui,
        color::FAINT,
        "Quick Projects and the New Project form start with these. Each project keeps its own.",
    );
    ui.dummy([0.0, 8.0]);
    heading(ui, fonts, "AFTER EXPORTING");
    ui.checkbox(
        "Show the exported file in its folder",
        &mut values.open_export_folder,
    );
    ui.dummy([0.0, 8.0]);
    heading(ui, fonts, "UPDATES");
    ui.checkbox(
        "Check for updates when the Mixer Launcher opens",
        &mut values.check_updates,
    );
    note(
        ui,
        color::FAINT,
        concat!(
            "This is Mixer ",
            env!("CARGO_PKG_VERSION"),
            ". Start Mixer with mixer-launcher.exe to get updates from GitHub."
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Clicks all over each page, as the editor's test does: nothing may crash, and the
    /// values must stay in range.
    #[test]
    fn every_page_can_be_clicked_through() {
        let _turn = crate::theme::IMGUI_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut context = imgui::Context::create();
        context.set_ini_filename(None);
        let fonts = crate::theme::install(&mut context);
        context.fonts().build_alpha8_texture();
        context.io_mut().display_size = [1280.0, 800.0];
        context.io_mut().delta_time = 1.0 / 60.0;
        let mut state = SettingsState {
            page: 0,
            values: Settings::default(),
            antialiasing_now: true,
            antialiasing_refused: false,
            renderer_now: "opengl2.1".into(),
            renderer_wanted: "opengl2.1".into(),
            engines: Some(Report::parse(
                "version 1\nvulkan no\ndirectx12 software x\n",
            )),
        };
        let mut seen = std::collections::HashSet::new();
        let mut pages = std::collections::HashSet::new();
        let mut frame = |context: &mut imgui::Context, state: &mut SettingsState| {
            let ui = context.new_frame();
            if let Some(a) = build_settings(ui, state, &fonts) {
                seen.insert(format!("{a:?}"));
            }
            context.render();
        };
        for page in 0..PAGES.len() {
            state.page = page;
            frame(&mut context, &mut state);
            for y in (0..800).step_by(9) {
                for x in (250..1030).step_by(17) {
                    let (x, y) = (x as f32, y as f32);
                    context.io_mut().add_mouse_pos_event([x, y]);
                    frame(&mut context, &mut state);
                    context
                        .io_mut()
                        .add_mouse_button_event(imgui::MouseButton::Left, true);
                    frame(&mut context, &mut state);
                    context.io_mut().add_mouse_pos_event([x + 6.0, y]);
                    frame(&mut context, &mut state);
                    context
                        .io_mut()
                        .add_mouse_button_event(imgui::MouseButton::Left, false);
                    frame(&mut context, &mut state);
                    pages.insert(state.page);
                    // Clicking a page button moves to it; go back to keep testing this one.
                    state.page = page;
                }
            }
        }
        assert!(
            seen.contains("Changed") && seen.contains("Close"),
            "{seen:?}"
        );
        assert_eq!(pages.len(), PAGES.len());
        let v = &state.values;
        assert!(INTERFACE_SIZES.contains(&v.interface));
        assert!((SLOWEST_TURN..=FASTEST_TURN).contains(&v.turn_speed));
        assert!((FEWEST_UNDO_STEPS..=MOST_UNDO_STEPS).contains(&v.undo_steps));
        assert!((FEWEST_RECENT..=MOST_RECENT as u32).contains(&v.recent));
        assert!(v.export_scale > 0.0);
        assert!(RENDERERS.iter().any(|r| r.0 == v.renderer));
    }
}
