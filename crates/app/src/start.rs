//! The start window: shown before the editor, to make a new project, make a quick
//! one, or open a recent one.

use imgui::{Condition, Drag, SliderFlags, StyleColor, StyleVar, Ui, WindowFlags};

use crate::export::ExportFormat;
use crate::places::Recent;
use crate::theme::{Fonts, color, icon};
use crate::ui::Shape;

/// What the start window's fields hold.
#[derive(Debug, Clone, PartialEq)]
pub struct StartState {
    pub name: String,
    /// What a new project starts with, if anything.
    pub shape: Option<Shape>,
    pub export_format: ExportFormat,
    pub scale: f32,
    pub recent: Vec<Recent>,
    /// A problem to show, such as a name that is taken.
    pub message: String,
    /// Where new projects go, to show.
    pub projects_folder: String,
    /// The anti-aliasing setting, and whether this run has it.
    pub antialiasing: bool,
    pub antialiasing_now: bool,
    /// Asked for when Mixer started, but the graphics driver refused.
    pub antialiasing_refused: bool,
}

impl StartState {
    pub fn new(
        recent: Vec<Recent>,
        projects_folder: String,
        antialiasing: bool,
        now: bool,
    ) -> Self {
        Self {
            name: String::new(),
            shape: Some(Shape::Cube),
            export_format: ExportFormat::Obj,
            scale: 1.0,
            recent,
            message: String::new(),
            projects_folder,
            antialiasing,
            antialiasing_now: now,
            antialiasing_refused: false,
        }
    }
}

/// What was chosen in the start window this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartAction {
    Quick,
    Create,
    OpenRecent(usize),
    OpenOther,
    OpenFolder,
    SetAntialiasing(bool),
}

const WIDTH: f32 = 860.0;
const HEIGHT: f32 = 600.0;

/// Draws the start window over the whole app window.
pub fn build_start(ui: &Ui, state: &mut StartState, fonts: &Fonts) -> Option<StartAction> {
    let display = ui.io().display_size;
    let mut action = None;
    let size = [WIDTH.min(display[0] - 20.0), HEIGHT.min(display[1] - 20.0)];
    let pos = [
        ((display[0] - size[0]) / 2.0).max(10.0),
        ((display[1] - size[1]) / 2.0).max(10.0),
    ];
    // The scene behind is dimmed so the window stands out.
    ui.get_background_draw_list()
        .add_rect([0.0, 0.0], display, [0.0, 0.0, 0.0, 0.45])
        .filled(true)
        .build();
    let padding = ui.push_style_var(StyleVar::WindowPadding([28.0, 24.0]));
    let rounding = ui.push_style_var(StyleVar::WindowRounding(12.0));
    ui.window("##start")
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
            title(ui, fonts);
            ui.dummy([0.0, 10.0]);
            let column = (ui.content_region_avail()[0] - 32.0) / 2.0;
            ui.child_window("##left")
                .size([column, -44.0])
                .build(|| left_column(ui, state, fonts, &mut action));
            ui.same_line_with_spacing(0.0, 32.0);
            ui.child_window("##right")
                .size([column, -44.0])
                .build(|| right_column(ui, state, fonts, &mut action));
            settings_row(ui, state, &mut action);
        });
    drop(rounding);
    drop(padding);
    action
}

fn title(ui: &Ui, fonts: &Fonts) {
    let start = ui.cursor_screen_pos();
    ui.get_window_draw_list()
        .add_rect(
            [start[0], start[1] + 4.0],
            [start[0] + 26.0, start[1] + 30.0],
            color::ACCENT,
        )
        .filled(true)
        .rounding(7.0)
        .build();
    ui.set_cursor_screen_pos([start[0] + 40.0, start[1]]);
    let font = ui.push_font(fonts.title);
    ui.text("Mixer");
    drop(font);
    ui.same_line_with_spacing(0.0, 14.0);
    ui.set_cursor_screen_pos([ui.cursor_screen_pos()[0], start[1] + 12.0]);
    ui.text_colored(color::MUTED, "Low-poly models for Roblox");
}

/// Colored text that wraps at the edge of its column.
fn wrapped(ui: &Ui, tint: [f32; 4], text: &str) {
    let color = ui.push_style_color(StyleColor::Text, tint);
    ui.text_wrapped(text);
    drop(color);
}

fn heading(ui: &Ui, fonts: &Fonts, text: &str) {
    let font = ui.push_font(fonts.heading);
    ui.text_colored(color::FAINT, text);
    drop(font);
    ui.dummy([0.0, 2.0]);
}

/// A big button with an icon, a title and a line of explanation.
fn tile(ui: &Ui, fonts: &Fonts, id: &str, glyph: char, name: &str, line: &str) -> bool {
    let start = ui.cursor_screen_pos();
    let width = ui.content_region_avail()[0];
    let height = 64.0;
    let clicked = ui.invisible_button(id, [width, height]);
    let (hovered, active) = (ui.is_item_hovered(), ui.is_item_active());
    let list = ui.get_window_draw_list();
    let fill = if active {
        color::FRAME_ACTIVE
    } else if hovered {
        color::FRAME_HOVER
    } else {
        color::FRAME
    };
    list.add_rect(start, [start[0] + width, start[1] + height], fill)
        .filled(true)
        .rounding(10.0)
        .build();
    let font = ui.push_font(fonts.tool);
    list.add_text(
        [start[0] + 16.0, start[1] + 20.0],
        color::ACCENT,
        glyph.to_string(),
    );
    list.add_text([start[0] + 52.0, start[1] + 12.0], color::TEXT, name);
    drop(font);
    list.add_text([start[0] + 52.0, start[1] + 36.0], color::MUTED, line);
    clicked
}

fn left_column(ui: &Ui, state: &mut StartState, fonts: &Fonts, action: &mut Option<StartAction>) {
    heading(ui, fonts, "START");
    if tile(
        ui,
        fonts,
        "##quick",
        icon::CUBE,
        "Quick Project",
        "Start modeling now. Saved in Quick Projects.",
    ) {
        *action = Some(StartAction::Quick);
    }
    ui.dummy([0.0, 8.0]);
    heading(ui, fonts, "NEW PROJECT");
    let label_column = 112.0;
    let row = |label: &str| {
        ui.align_text_to_frame_padding();
        ui.text_colored(color::MUTED, label);
        ui.same_line_with_pos(label_column);
        ui.set_next_item_width(-1.0);
    };
    row("Name");
    let typed = ui
        .input_text("##project_name", &mut state.name)
        .hint("For example: Sword")
        .enter_returns_true(true)
        .build();
    if typed {
        *action = Some(StartAction::Create);
    }
    let shown = if state.name.trim().is_empty() {
        "Name".to_string()
    } else {
        state.name.trim().to_string()
    };
    ui.set_cursor_pos([label_column, ui.cursor_pos()[1]]);
    wrapped(
        ui,
        color::FAINT,
        &format!("Saved in {} > {shown}", state.projects_folder),
    );

    row("Start with");
    let preview = state.shape.map_or("Nothing", Shape::name);
    if let Some(_combo) = ui.begin_combo("##start_shape", preview) {
        if ui
            .selectable_config("Nothing")
            .selected(state.shape.is_none())
            .build()
        {
            state.shape = None;
        }
        for shape in Shape::ALL {
            let label = format!("{}  {}", shape.icon(), shape.name());
            if ui
                .selectable_config(&label)
                .selected(state.shape == Some(shape))
                .build()
            {
                state.shape = Some(shape);
            }
        }
    }

    row("Export as");
    for (i, (name, format)) in [("OBJ", ExportFormat::Obj), ("glTF", ExportFormat::Glb)]
        .into_iter()
        .enumerate()
    {
        if i > 0 {
            ui.same_line();
        }
        if ui.radio_button_bool(name, state.export_format == format) {
            state.export_format = format;
        }
    }
    row("Scale");
    Drag::new("##start_scale")
        .range(0.001, 1000.0)
        .speed(0.01)
        .display_format("%.3f")
        .flags(SliderFlags::ALWAYS_CLAMP)
        .build(ui, &mut state.scale);
    if ui.is_item_hovered() {
        ui.tooltip_text("At 1, one unit is one stud in Roblox.");
    }
    ui.dummy([0.0, 4.0]);
    let accent = [
        ui.push_style_color(StyleColor::Button, color::ACCENT),
        ui.push_style_color(StyleColor::ButtonHovered, color::ACCENT_HOVER),
        ui.push_style_color(StyleColor::ButtonActive, color::ACCENT),
        ui.push_style_color(StyleColor::Text, color::ON_ACCENT),
    ];
    if ui.button_with_size(format!("{}  Create project", icon::NEW), [-1.0, 36.0]) {
        *action = Some(StartAction::Create);
    }
    drop(accent);
    if !state.message.is_empty() {
        wrapped(ui, color::WARNING, &state.message);
    }
}

fn right_column(ui: &Ui, state: &mut StartState, fonts: &Fonts, action: &mut Option<StartAction>) {
    heading(ui, fonts, "RECENT");
    if state.recent.is_empty() {
        ui.text_colored(color::MUTED, "Projects you open or save show up here.");
    }
    for (i, recent) in state.recent.iter().enumerate() {
        let start = ui.cursor_screen_pos();
        let width = ui.content_region_avail()[0];
        if ui.invisible_button(format!("##recent{i}"), [width, 44.0]) {
            *action = Some(StartAction::OpenRecent(i));
        }
        let hovered = ui.is_item_hovered();
        let list = ui.get_window_draw_list();
        if hovered {
            list.add_rect(
                start,
                [start[0] + width, start[1] + 44.0],
                color::FRAME_HOVER,
            )
            .filled(true)
            .rounding(8.0)
            .build();
        }
        let name = recent
            .path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let folder = recent
            .path
            .parent()
            .and_then(|f| f.file_name())
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        list.add_text([start[0] + 10.0, start[1] + 6.0], color::TEXT, &name);
        list.add_text(
            [start[0] + 10.0, start[1] + 24.0],
            color::FAINT,
            format!("{folder}  \u{2022}  {}", recent.age),
        );
        drop(list);
        if hovered {
            ui.tooltip_text(recent.path.display().to_string());
        }
    }
    ui.dummy([0.0, 6.0]);
    let half = (ui.content_region_avail()[0] - 8.0) / 2.0;
    if ui.button_with_size(
        format!("{}  Open other file\u{2026}", icon::FOLDER),
        [half, 32.0],
    ) {
        *action = Some(StartAction::OpenOther);
    }
    ui.same_line_with_spacing(0.0, 8.0);
    if ui.button_with_size(format!("{}  Mixer folder", icon::FOLDER), [half, 32.0]) {
        *action = Some(StartAction::OpenFolder);
    }
}

fn settings_row(ui: &Ui, state: &mut StartState, action: &mut Option<StartAction>) {
    ui.separator();
    let mut smooth = state.antialiasing;
    if ui.checkbox("Smooth edges (anti-aliasing)", &mut smooth) {
        *action = Some(StartAction::SetAntialiasing(smooth));
    }
    ui.same_line_with_spacing(0.0, 12.0);
    let note = if state.antialiasing && state.antialiasing_refused {
        "Your graphics driver did not allow it, so it is off."
    } else if state.antialiasing == state.antialiasing_now {
        "Turn off if Mixer feels slow."
    } else {
        "Changes the next time Mixer starts."
    };
    ui.text_colored(color::FAINT, note);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Clicks all over the start window, as the editor's test does, and checks every
    /// choice can be reached.
    #[test]
    fn every_choice_in_the_start_window_can_be_clicked() {
        let _turn = crate::theme::IMGUI_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut context = imgui::Context::create();
        context.set_ini_filename(None);
        let fonts = crate::theme::install(&mut context);
        context.fonts().build_alpha8_texture();
        context.io_mut().display_size = [1280.0, 800.0];
        context.io_mut().delta_time = 1.0 / 60.0;
        let recent = vec![
            Recent {
                path: PathBuf::from("/p/Sword/Sword.mix"),
                age: "5 minutes ago".to_string(),
            },
            Recent {
                path: PathBuf::from("/p/Quick Projects/Quick 2026-11-08 02.49.mix"),
                age: "2 days ago".to_string(),
            },
        ];
        let mut state = StartState::new(recent, "Documents > Mixer > Projects".into(), true, true);
        state.message = "A project called Sword already exists.".to_string();
        let mut seen = std::collections::HashSet::new();
        let mut frame = |context: &mut imgui::Context, state: &mut StartState| {
            let ui = context.new_frame();
            if let Some(a) = build_start(ui, state, &fonts) {
                seen.insert(format!("{a:?}"));
            }
            context.render();
        };
        frame(&mut context, &mut state);
        for y in (0..800).step_by(8) {
            for x in (0..1280).step_by(20) {
                let (x, y) = (x as f32, y as f32);
                context.io_mut().add_mouse_pos_event([x, y]);
                frame(&mut context, &mut state);
                context
                    .io_mut()
                    .add_mouse_button_event(imgui::MouseButton::Left, true);
                frame(&mut context, &mut state);
                context
                    .io_mut()
                    .add_mouse_button_event(imgui::MouseButton::Left, false);
                frame(&mut context, &mut state);
            }
        }
        for expected in [
            "Quick",
            "Create",
            "OpenRecent(0)",
            "OpenRecent(1)",
            "OpenOther",
            "OpenFolder",
            "SetAntialiasing(false)",
        ] {
            assert!(
                seen.contains(expected),
                "never clicked {expected}: {seen:?}"
            );
        }
        assert!(state.scale > 0.0);
    }
}
