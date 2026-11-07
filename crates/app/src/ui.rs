//! The interface, built with Dear ImGui: a top bar, a tool strip on the left, a settings
//! panel on the right, a status bar, and a view gizmo.
//!
//! Every label in a window must be different, because Dear ImGui builds IDs from them.
//! Two widgets sharing an ID crash it when clicked. The test at the end clicks
//! everywhere to catch that.

use imgui::{
    Condition, Drag, ItemHoveredFlags, ProgressBar, SliderFlags, StyleColor, StyleVar, Ui,
    WindowFlags,
};
use modeler_mesh::{MAX_SUBDIVISIONS, Primitive, ROBLOX_MAX_TRIANGLES};

use crate::editor::Target;
use crate::export::ExportFormat;
use crate::picking::Region;
use crate::theme::{Fonts, color, icon};
use crate::transform::Kind;

pub const TOP_BAR: f32 = 52.0;
pub const STATUS_BAR: f32 = 30.0;
pub const MARGIN: f32 = 10.0;
pub const RIGHT_PANEL: f32 = 300.0;
const TOOL_BUTTON: f32 = 40.0;
const GIZMO: f32 = 104.0;
const LABEL_COLUMN: f32 = 96.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Cube,
    Cone,
    Cylinder,
    UvSphere,
    Icosphere,
    Grid,
    Circle,
}

impl Shape {
    const ALL: [Shape; 7] = [
        Shape::Cube,
        Shape::Cone,
        Shape::Cylinder,
        Shape::UvSphere,
        Shape::Icosphere,
        Shape::Grid,
        Shape::Circle,
    ];

    fn icon(self) -> char {
        match self {
            Shape::Cube => icon::CUBE,
            Shape::Cone => icon::CONE,
            Shape::Cylinder => icon::CYLINDER,
            Shape::UvSphere => icon::UV_SPHERE,
            Shape::Icosphere => icon::ICOSPHERE,
            Shape::Grid => icon::GRID,
            Shape::Circle => icon::CIRCLE,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Shape::Cube => "Cube",
            Shape::Cone => "Cone",
            Shape::Cylinder => "Cylinder",
            Shape::UvSphere => "UV Sphere",
            Shape::Icosphere => "Icosphere",
            Shape::Grid => "Grid",
            Shape::Circle => "Circle",
        }
    }

    fn uses(self, setting: Setting) -> bool {
        match setting {
            Setting::Segments => matches!(
                self,
                Shape::Cone | Shape::Cylinder | Shape::UvSphere | Shape::Circle
            ),
            Setting::Rings => self == Shape::UvSphere,
            Setting::Subdivisions => self == Shape::Icosphere,
            Setting::Cuts => self == Shape::Grid,
        }
    }

    /// The name shown beside Undo and Redo.
    pub fn edit_label(self) -> &'static str {
        match self {
            Shape::Cube => "Add Cube",
            Shape::Cone => "Add Cone",
            Shape::Cylinder => "Add Cylinder",
            Shape::UvSphere => "Add UV Sphere",
            Shape::Icosphere => "Add Icosphere",
            Shape::Grid => "Add Grid",
            Shape::Circle => "Add Circle",
        }
    }
}

/// Sliders that only some shapes use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Setting {
    Segments,
    Rings,
    Subdivisions,
    Cuts,
}

/// The selection mode buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeChoice {
    Part,
    Vertex,
    Edge,
    Face,
}

/// What a left drag in the viewport does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DragTool {
    /// Dragging does nothing, so only clicks select.
    #[default]
    ClickOnly,
    Box,
    Circle,
    Lasso,
}

/// The values the panel edits.
#[derive(Debug, Clone, PartialEq)]
pub struct PanelState {
    pub size: f32,
    pub segments: i32,
    pub rings: i32,
    pub subdivisions: i32,
    pub cuts: i32,
    pub location: [f32; 3],
    pub file_name: String,
    pub scale: f32,
    pub message: String,
    pub tool: DragTool,
    /// Whether the settings for new shapes are open while no shape is selected.
    pub show_defaults: bool,
    /// How tall the panel's contents were last frame, so that it can end below them.
    pub panel_content: f32,
    pub export_format: ExportFormat,
    /// Where there are no system file dialogs, the path being typed.
    pub path_prompt: Option<PathPrompt>,
}

/// What a typed path is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathPurpose {
    Open,
    SaveAs,
}

/// A path being typed, where the system has no file dialog the app can use.
#[derive(Debug, Clone, PartialEq)]
pub struct PathPrompt {
    pub purpose: PathPurpose,
    pub text: String,
}

impl Default for PanelState {
    fn default() -> Self {
        Self {
            size: 2.0,
            segments: 16,
            rings: 8,
            subdivisions: 1,
            cuts: 4,
            location: [0.0; 3],
            file_name: "model".to_string(),
            scale: 1.0,
            message: String::new(),
            tool: DragTool::ClickOnly,
            show_defaults: false,
            panel_content: f32::MAX,
            export_format: ExportFormat::Obj,
            path_prompt: None,
        }
    }
}

impl PanelState {
    /// The shape described by the current values. `size` is the overall width and
    /// height, so all shapes come out about the same size.
    pub fn primitive(&self, shape: Shape) -> Primitive {
        let size = self.size;
        let radius = size / 2.0;
        let segments = self.segments as u32;
        match shape {
            Shape::Cube => Primitive::Cube { size },
            Shape::Cone => Primitive::Cone {
                radius,
                height: size,
                segments,
            },
            Shape::Cylinder => Primitive::Cylinder {
                radius,
                height: size,
                segments,
            },
            Shape::UvSphere => Primitive::UvSphere {
                radius,
                segments,
                rings: self.rings as u32,
            },
            Shape::Icosphere => Primitive::Icosphere {
                radius,
                subdivisions: self.subdivisions as u32,
            },
            Shape::Grid => Primitive::Grid {
                size,
                columns: self.cuts as u32,
                rows: self.cuts as u32,
            },
            Shape::Circle => Primitive::Circle { radius, segments },
        }
    }
}

impl PanelState {
    /// Puts a shape's values on the sliders.
    pub fn show_values(&mut self, primitive: Primitive, center: [f32; 3]) {
        self.location = center;
        match primitive {
            Primitive::Cube { size } => self.size = size,
            Primitive::Cone {
                radius, segments, ..
            }
            | Primitive::Cylinder {
                radius, segments, ..
            }
            | Primitive::Circle { radius, segments } => {
                self.size = radius * 2.0;
                self.segments = segments as i32;
            }
            Primitive::UvSphere {
                radius,
                segments,
                rings,
            } => {
                self.size = radius * 2.0;
                self.segments = segments as i32;
                self.rings = rings as i32;
            }
            Primitive::Icosphere {
                radius,
                subdivisions,
            } => {
                self.size = radius * 2.0;
                self.subdivisions = subdivisions as i32;
            }
            Primitive::Grid { size, columns, .. } => {
                self.size = size;
                self.cuts = columns as i32;
            }
        }
    }
}

/// The camera's directions, for drawing the view gizmo.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewState {
    pub right: [f32; 3],
    pub up: [f32; 3],
    pub toward: [f32; 3],
    pub orthographic: bool,
}

/// What the document looks like, for showing in the interface.
pub struct PanelInfo<'a> {
    pub triangles: usize,
    pub vertices: usize,
    pub faces: usize,
    pub open_edges: usize,
    pub warnings: &'a [String],
    pub target: Target,
    pub mode: ModeChoice,
    pub selected_vertices: usize,
    pub transform_help: Option<(Kind, Option<usize>)>,
    /// While choosing where to loop cut: how many cuts.
    pub loop_cut: Option<usize>,
    /// While bevelling: the width and the number of segments.
    pub bevel: Option<(f32, usize)>,
    pub undo_label: Option<&'static str>,
    pub redo_label: Option<&'static str>,
    pub export_folder: &'a str,
    pub view: ViewState,
    /// The open file's name, or "Untitled".
    pub document: &'a str,
    pub unsaved: bool,
    /// A question about unsaved changes, while one is waiting for an answer.
    pub confirm: Option<&'a str>,
}

/// Standard views, as in Blender's numpad 1, 3 and 7 and their opposites.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewPreset {
    Front,
    Back,
    Right,
    Left,
    Top,
    Bottom,
}

impl ViewPreset {
    /// Yaw and pitch in degrees, as `Camera::set_view` takes them.
    pub fn angles(self) -> (f32, f32) {
        match self {
            ViewPreset::Front => (0.0, 0.0),
            ViewPreset::Back => (180.0, 0.0),
            ViewPreset::Right => (90.0, 0.0),
            ViewPreset::Left => (-90.0, 0.0),
            ViewPreset::Top => (0.0, 90.0),
            ViewPreset::Bottom => (0.0, -90.0),
        }
    }

    /// The view looking from the positive or negative end of an axis.
    fn from_axis(axis: usize, positive: bool) -> Self {
        match (axis, positive) {
            (0, true) => ViewPreset::Right,
            (0, false) => ViewPreset::Left,
            (1, true) => ViewPreset::Back,
            (1, false) => ViewPreset::Front,
            (_, true) => ViewPreset::Top,
            (_, false) => ViewPreset::Bottom,
        }
    }
}

/// What the user asked for this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Add(Shape),
    Undo,
    Redo,
    Export,
    SetMode(ModeChoice),
    Delete,
    Extrude,
    Inset,
    Merge,
    MergeByDistance,
    Fill,
    Subdivide,
    SelectAll,
    SelectNone,
    SelectInvert,
    SelectLinked,
    SelectMore,
    SelectLess,
    Transform(Kind),
    Duplicate,
    Mirror(usize),
    /// Starts choosing where to cut, by hovering over an edge.
    LoopCut,
    Bridge,
    Bevel,
    /// Grows the selected edges to their whole loops.
    SelectLoops,
    /// Grows the selected edges to their whole rings.
    SelectRings,
    View(ViewPreset),
    ToggleOrtho,
    ResetView,
    New,
    Open,
    Save,
    SaveAs,
    /// The typed path in `PanelState::path_prompt` is ready.
    PathChosen,
    PathCancelled,
    /// Answers to "Save changes first?"
    ConfirmSave,
    ConfirmDiscard,
    ConfirmCancel,
    #[cfg(windows)]
    OpenExportFolder,
}

/// The interface's result for one frame.
pub struct UiOutput {
    pub action: Option<Action>,
    /// Screen areas the interface covers, as x, y, width and height. Clicks there do
    /// not reach the viewport.
    pub rects: Vec<[f32; 4]>,
}

/// The part of the window the 3D view shows through, between the panels.
pub fn viewport_area(display: [f32; 2]) -> [f32; 4] {
    let left = MARGIN * 2.0 + TOOL_BUTTON + 16.0;
    let right = display[0] - RIGHT_PANEL - MARGIN * 2.0;
    let top = TOP_BAR;
    let bottom = display[1] - STATUS_BAR;
    [left, top, (right - left).max(1.0), (bottom - top).max(1.0)]
}

fn with_alpha(mut c: [f32; 4], alpha: f32) -> [f32; 4] {
    c[3] *= alpha;
    c
}

fn tooltip(ui: &Ui, name: &str, key: &str) {
    if ui.is_item_hovered_with_flags(ItemHoveredFlags::ALLOW_WHEN_DISABLED) {
        ui.tooltip(|| {
            ui.text(name);
            if !key.is_empty() {
                ui.same_line_with_spacing(0.0, 10.0);
                ui.text_colored(color::MUTED, key);
            }
        });
    }
}

/// A square button showing an icon, lit when `active`.
fn icon_button(ui: &Ui, id: &str, glyph: char, active: bool, enabled: bool, size: f32) -> bool {
    let colors = active.then(|| {
        [
            ui.push_style_color(StyleColor::Button, color::ACCENT_SOFT),
            ui.push_style_color(StyleColor::ButtonHovered, with_alpha(color::ACCENT, 0.32)),
            ui.push_style_color(StyleColor::Text, color::ACCENT),
        ]
    });
    let quiet = (!active).then(|| ui.push_style_color(StyleColor::Button, [0.0; 4]));
    let disabled = ui.begin_disabled(!enabled);
    let clicked = ui.button_with_size(format!("{glyph}##{id}"), [size, size]);
    disabled.end();
    drop(quiet);
    drop(colors);
    clicked
}

/// A row of joined buttons, one of which is lit. Returns the index clicked.
fn segmented(ui: &Ui, id: &str, items: &[(char, &str, bool, &str)], height: f32) -> Option<usize> {
    let pad = 10.0;
    let widths: Vec<f32> = items
        .iter()
        .map(|(_, label, _, _)| {
            let text = if label.is_empty() {
                0.0
            } else {
                ui.calc_text_size(label)[0] + 6.0
            };
            (16.0 + text + pad * 2.0).max(height)
        })
        .collect();
    let gap = 2.0;
    let total: f32 = widths.iter().sum::<f32>() + gap * (items.len() as f32 + 1.0);
    let start = ui.cursor_screen_pos();
    ui.get_window_draw_list()
        .add_rect(
            start,
            [start[0] + total, start[1] + height + 4.0],
            color::FRAME,
        )
        .filled(true)
        .rounding(8.0)
        .build();
    let mut clicked = None;
    let spacing = ui.push_style_var(StyleVar::ItemSpacing([gap, 0.0]));
    let rounding = ui.push_style_var(StyleVar::FrameRounding(6.0));
    ui.set_cursor_screen_pos([start[0] + gap, start[1] + 2.0]);
    for (i, ((glyph, label, active, tip), width)) in items.iter().zip(&widths).enumerate() {
        if i > 0 {
            ui.same_line();
        }
        let colors = if *active {
            vec![
                ui.push_style_color(StyleColor::Button, color::ACCENT_SOFT),
                ui.push_style_color(StyleColor::ButtonHovered, with_alpha(color::ACCENT, 0.30)),
                ui.push_style_color(StyleColor::Text, color::ACCENT),
            ]
        } else {
            vec![
                ui.push_style_color(StyleColor::Button, [0.0; 4]),
                ui.push_style_color(StyleColor::Text, color::MUTED),
            ]
        };
        let text = if label.is_empty() {
            format!("{glyph}##{id}{i}")
        } else {
            format!("{glyph}  {label}##{id}{i}")
        };
        if ui.button_with_size(text, [*width, height]) {
            clicked = Some(i);
        }
        drop(colors);
        tooltip(ui, tip, "");
    }
    drop(rounding);
    drop(spacing);
    ui.set_cursor_screen_pos([start[0] + total, start[1]]);
    ui.dummy([0.0, height + 4.0]);
    clicked
}

fn heading(ui: &Ui, fonts: &Fonts, text: &str) {
    ui.dummy([0.0, 2.0]);
    let font = ui.push_font(fonts.heading);
    ui.text_colored(color::FAINT, text);
    drop(font);
}

/// A muted label on the left, and the widget filling the rest of the row.
fn row(ui: &Ui, label: &str) {
    ui.align_text_to_frame_padding();
    ui.text_colored(color::MUTED, label);
    ui.same_line_with_pos(LABEL_COLUMN);
    ui.set_next_item_width(-1.0);
}

/// Equal-width buttons filling a row.
/// A row of equal buttons: label, action, and a tooltip if not empty.
fn button_row(
    ui: &Ui,
    action: &mut Option<Action>,
    buttons: &[(&str, Action, &str)],
    enabled: bool,
) {
    let gap = 6.0;
    let count = buttons.len() as f32;
    let width = ((ui.content_region_avail()[0] - gap * (count - 1.0)) / count).floor();
    let spacing = ui.push_style_var(StyleVar::ItemSpacing([gap, 8.0]));
    let disabled = ui.begin_disabled(!enabled);
    for (i, (label, choice, help)) in buttons.iter().enumerate() {
        if i > 0 {
            ui.same_line();
        }
        if ui.button_with_size(label, [width, 30.0]) {
            *action = Some(*choice);
        }
        if !help.is_empty() {
            tooltip(ui, help, "");
        }
    }
    disabled.end();
    drop(spacing);
}

fn note(ui: &Ui, glyph: char, tint: [f32; 4], text: &str) {
    let start = ui.cursor_screen_pos();
    let width = ui.content_region_avail()[0];
    let list = ui.get_window_draw_list();
    list.channels_split(2, |channels| {
        channels.set_current(1);
        ui.set_cursor_screen_pos([start[0] + 10.0, start[1] + 8.0]);
        ui.text_colored(tint, glyph.to_string());
        ui.same_line();
        let wrap = ui.push_text_wrap_pos_with_pos(ui.cursor_pos()[0] + width - 44.0);
        ui.text_colored(color::MUTED, text);
        drop(wrap);
        let end = ui.item_rect_max();
        channels.set_current(0);
        list.add_rect(
            start,
            [start[0] + width, end[1] + 8.0],
            with_alpha(tint, 0.10),
        )
        .filled(true)
        .rounding(8.0)
        .build();
        ui.set_cursor_screen_pos([start[0], end[1] + 16.0]);
    });
    ui.dummy([0.0, 0.0]);
}

fn fixed_window<'a>(
    ui: &'a Ui,
    name: &'a str,
    pos: [f32; 2],
    size: [f32; 2],
) -> imgui::Window<'a, 'a, &'a str> {
    ui.window(name)
        .position(pos, Condition::Always)
        .size(size, Condition::Always)
        .flags(
            WindowFlags::NO_TITLE_BAR
                | WindowFlags::NO_RESIZE
                | WindowFlags::NO_MOVE
                | WindowFlags::NO_COLLAPSE
                | WindowFlags::NO_SAVED_SETTINGS,
        )
}

/// Builds the whole interface for one frame.
pub fn build(ui: &Ui, state: &mut PanelState, info: &PanelInfo, fonts: &Fonts) -> UiOutput {
    let display = ui.io().display_size;
    let mut action = None;
    let mut rects = Vec::new();
    let record = |ui: &Ui| {
        let [x, y] = ui.window_pos();
        let [w, h] = ui.window_size();
        [x, y, w, h]
    };

    // Bars run edge to edge, so they have no rounding or border.
    {
        let flat = [
            ui.push_style_var(StyleVar::WindowRounding(0.0)),
            ui.push_style_var(StyleVar::WindowBorderSize(0.0)),
        ];
        let bg = ui.push_style_color(StyleColor::WindowBg, color::BAR);
        let padding = ui.push_style_var(StyleVar::WindowPadding([12.0, 8.0]));
        if let Some(rect) = fixed_window(ui, "##top", [0.0, 0.0], [display[0], TOP_BAR])
            .flags(
                WindowFlags::NO_TITLE_BAR
                    | WindowFlags::NO_RESIZE
                    | WindowFlags::NO_MOVE
                    | WindowFlags::NO_SCROLLBAR
                    | WindowFlags::NO_SAVED_SETTINGS,
            )
            .build(|| {
                top_bar(ui, state, info, fonts, &mut action);
                record(ui)
            })
        {
            rects.push(rect);
        }
        let padding_status = ui.push_style_var(StyleVar::WindowPadding([12.0, 6.0]));
        if let Some(rect) = fixed_window(
            ui,
            "##status",
            [0.0, display[1] - STATUS_BAR],
            [display[0], STATUS_BAR],
        )
        .flags(
            WindowFlags::NO_TITLE_BAR
                | WindowFlags::NO_RESIZE
                | WindowFlags::NO_MOVE
                | WindowFlags::NO_SCROLLBAR
                | WindowFlags::NO_SAVED_SETTINGS
                | WindowFlags::NO_INPUTS,
        )
        .build(|| {
            status_bar(ui, state, info);
            record(ui)
        }) {
            rects.push(rect);
        }
        drop(padding_status);
        drop(padding);
        drop(bg);
        drop(flat);
    }

    // Tool strip.
    {
        let padding = ui.push_style_var(StyleVar::WindowPadding([8.0, 8.0]));
        let spacing = ui.push_style_var(StyleVar::ItemSpacing([0.0, 4.0]));
        if let Some(rect) = ui
            .window("##tools")
            .position([MARGIN, TOP_BAR + MARGIN], Condition::Always)
            .flags(
                WindowFlags::NO_TITLE_BAR
                    | WindowFlags::NO_RESIZE
                    | WindowFlags::NO_MOVE
                    | WindowFlags::NO_SCROLLBAR
                    | WindowFlags::NO_SAVED_SETTINGS
                    | WindowFlags::ALWAYS_AUTO_RESIZE,
            )
            .build(|| {
                tool_strip(ui, info, fonts, &mut action);
                record(ui)
            })
        {
            rects.push(rect);
        }
        drop(spacing);
        drop(padding);
    }

    // Settings panel.
    {
        let padding = ui.push_style_var(StyleVar::WindowPadding([14.0, 14.0]));
        let pos = [display[0] - RIGHT_PANEL - MARGIN, TOP_BAR + MARGIN];
        // As tall as its contents, but never past the status bar; then it scrolls.
        let room = (display[1] - TOP_BAR - STATUS_BAR - MARGIN * 2.0).max(60.0);
        let size = [RIGHT_PANEL, state.panel_content.min(room)];
        if let Some(rect) = fixed_window(ui, "##panel", pos, size).build(|| {
            panel(ui, state, info, fonts, &mut action);
            // Where the next item would go, plus the bottom padding, minus the
            // spacing the last item left below itself.
            let content = ui.cursor_pos()[1] + 14.0 - 8.0;
            state.panel_content = content.max(60.0);
            record(ui)
        }) {
            rects.push(rect);
        }
        drop(padding);
    }

    // View gizmo, without a background.
    {
        let padding = ui.push_style_var(StyleVar::WindowPadding([0.0, 0.0]));
        let pos = [
            display[0] - RIGHT_PANEL - MARGIN * 2.0 - GIZMO,
            TOP_BAR + MARGIN,
        ];
        if let Some(rect) = fixed_window(ui, "##view", pos, [GIZMO, GIZMO + 40.0])
            .flags(
                WindowFlags::NO_TITLE_BAR
                    | WindowFlags::NO_RESIZE
                    | WindowFlags::NO_MOVE
                    | WindowFlags::NO_SCROLLBAR
                    | WindowFlags::NO_SAVED_SETTINGS
                    | WindowFlags::NO_BACKGROUND,
            )
            .build(|| {
                gizmo(ui, info, &mut action);
                record(ui)
            })
        {
            rects.push(rect);
        }
        drop(padding);
    }

    dialogs(ui, state, info, &mut action);
    viewport_overlays(ui, info, fonts, display);
    UiOutput { action, rects }
}

fn top_bar(
    ui: &Ui,
    state: &mut PanelState,
    info: &PanelInfo,
    fonts: &Fonts,
    action: &mut Option<Action>,
) {
    let height = 30.0;
    let start = ui.cursor_screen_pos();
    // A small mark, then the name. Only one window draw list may exist at a time.
    ui.get_window_draw_list()
        .add_rect(
            [start[0], start[1] + 8.0],
            [start[0] + 18.0, start[1] + 26.0],
            color::ACCENT,
        )
        .filled(true)
        .rounding(5.0)
        .build();
    ui.set_cursor_screen_pos([start[0] + 28.0, start[1] + 7.0]);
    let font = ui.push_font(fonts.tool);
    ui.text("Low-End Modeler");
    drop(font);
    ui.same_line_with_spacing(0.0, 16.0);
    ui.set_cursor_screen_pos([ui.cursor_screen_pos()[0], start[1] + 2.0]);
    file_menu(ui, info, action, height);
    ui.same_line_with_spacing(0.0, 10.0);
    ui.set_cursor_screen_pos([ui.cursor_screen_pos()[0], start[1] + 9.0]);
    // The file's name, shortened if long, with a dot while there are unsaved changes.
    let mut name: String = info.document.chars().take(22).collect();
    if info.document.chars().count() > 22 {
        name.push('\u{2026}');
    }
    ui.text_colored(color::MUTED, &name);
    tooltip(ui, info.document, "");
    if info.unsaved {
        ui.same_line_with_spacing(0.0, 6.0);
        ui.text_colored(color::ACCENT, "\u{2022}");
        tooltip(ui, "Unsaved changes", "Ctrl+S");
    }
    ui.same_line_with_spacing(0.0, 24.0);
    ui.set_cursor_screen_pos([ui.cursor_screen_pos()[0], start[1]]);

    let modes = [
        (
            icon::PART,
            "Part",
            ModeChoice::Part,
            "Part mode (4): a click selects a whole shape",
        ),
        (
            icon::VERTEX,
            "Vertex",
            ModeChoice::Vertex,
            "Vertex mode (1)",
        ),
        (icon::EDGE, "Edge", ModeChoice::Edge, "Edge mode (2)"),
        (icon::FACE, "Face", ModeChoice::Face, "Face mode (3)"),
    ];
    let items: Vec<_> = modes
        .iter()
        .map(|(glyph, label, mode, tip)| (*glyph, *label, info.mode == *mode, *tip))
        .collect();
    if let Some(i) = segmented(ui, "mode", &items, height) {
        *action = Some(Action::SetMode(modes[i].2));
    }

    ui.same_line_with_spacing(0.0, 16.0);
    let tools = [
        (
            icon::CLICK,
            DragTool::ClickOnly,
            "Click to select, dragging does nothing",
        ),
        (icon::BOX_SELECT, DragTool::Box, "Drag a box to select"),
        (
            icon::CIRCLE_SELECT,
            DragTool::Circle,
            "Drag a circle brush to select",
        ),
        (icon::LASSO, DragTool::Lasso, "Draw a lasso to select"),
    ];
    let items: Vec<_> = tools
        .iter()
        .map(|(glyph, tool, tip)| (*glyph, "", state.tool == *tool, *tip))
        .collect();
    if let Some(i) = segmented(ui, "drag", &items, height) {
        state.tool = tools[i].1;
    }

    ui.same_line_with_spacing(0.0, 16.0);
    if icon_button(
        ui,
        "undo",
        icon::UNDO,
        false,
        info.undo_label.is_some(),
        height + 4.0,
    ) {
        *action = Some(Action::Undo);
    }
    let undo = info
        .undo_label
        .map_or("Nothing to undo".to_string(), |l| format!("Undo {l}"));
    tooltip(ui, &undo, "Ctrl+Z");
    ui.same_line_with_spacing(0.0, 4.0);
    if icon_button(
        ui,
        "redo",
        icon::REDO,
        false,
        info.redo_label.is_some(),
        height + 4.0,
    ) {
        *action = Some(Action::Redo);
    }
    let redo = info
        .redo_label
        .map_or("Nothing to redo".to_string(), |l| format!("Redo {l}"));
    tooltip(ui, &redo, "Ctrl+Y");

    // Shortcuts, then Export, at the right edge.
    let width = 112.0;
    ui.same_line_with_pos(ui.window_size()[0] - width - 12.0 - (height + 4.0) - 8.0);
    if icon_button(ui, "help", icon::KEYBOARD, false, true, height + 4.0) {
        ui.open_popup("##help_popup");
    }
    tooltip(ui, "Keyboard and mouse", "");
    help_popup(ui, fonts);
    ui.same_line_with_pos(ui.window_size()[0] - width - 12.0);
    let colors = [
        ui.push_style_color(StyleColor::Button, color::ACCENT),
        ui.push_style_color(StyleColor::ButtonHovered, color::ACCENT_HOVER),
        ui.push_style_color(StyleColor::ButtonActive, color::ACCENT_HOVER),
        ui.push_style_color(StyleColor::Text, color::ON_ACCENT),
    ];
    let font = ui.push_font(fonts.tool);
    if ui.button_with_size(
        format!("{}  Export##export", icon::EXPORT),
        [width, height + 4.0],
    ) {
        ui.open_popup("##export_popup");
    }
    drop(font);
    drop(colors);
    tooltip(ui, "Export for Roblox Studio", "Ctrl+E");
    export_popup(ui, state, info, fonts, action);
}

fn export_popup(
    ui: &Ui,
    state: &mut PanelState,
    info: &PanelInfo,
    fonts: &Fonts,
    action: &mut Option<Action>,
) {
    let padding = ui.push_style_var(StyleVar::WindowPadding([16.0, 14.0]));
    if let Some(_popup) = ui.begin_popup("##export_popup") {
        ui.dummy([300.0, 0.0]);
        heading(ui, fonts, "EXPORT FOR ROBLOX");
        let formats = [
            (
                icon::EXPORT,
                "OBJ",
                ExportFormat::Obj,
                "One mesh. Works everywhere.",
            ),
            (
                icon::EXPORT,
                "glTF (.glb)",
                ExportFormat::Glb,
                "Each separate part becomes its own MeshPart in Roblox.",
            ),
        ];
        let items: Vec<_> = formats
            .iter()
            .map(|(glyph, label, format, tip)| {
                (*glyph, *label, state.export_format == *format, *tip)
            })
            .collect();
        if let Some(i) = segmented(ui, "format", &items, 28.0) {
            state.export_format = formats[i].2;
        }
        row(ui, "File name");
        ui.input_text("##file_name", &mut state.file_name).build();
        row(ui, "Scale");
        Drag::new("##scale")
            .range(0.001, 1000.0)
            .speed(0.01)
            .display_format("%.3f")
            .flags(SliderFlags::ALWAYS_CLAMP)
            .build(ui, &mut state.scale);
        ui.text_colored(color::FAINT, "At scale 1, one unit is one stud in Roblox.");
        ui.spacing();
        triangle_budget(ui, info.triangles);
        for warning in info.warnings {
            note(ui, icon::WARNING, color::WARNING, warning);
        }
        let colors = [
            ui.push_style_color(StyleColor::Button, color::ACCENT),
            ui.push_style_color(StyleColor::ButtonHovered, color::ACCENT_HOVER),
            ui.push_style_color(StyleColor::ButtonActive, color::ACCENT_HOVER),
            ui.push_style_color(StyleColor::Text, color::ON_ACCENT),
        ];
        let disabled = ui.begin_disabled(info.triangles == 0);
        let label = match state.export_format {
            ExportFormat::Obj => "Save OBJ file##save",
            ExportFormat::Glb => "Save GLB file##save",
        };
        if ui.button_with_size(label, [-1.0, 34.0]) {
            *action = Some(Action::Export);
        }
        disabled.end();
        drop(colors);
        if !state.message.is_empty() && state.message.starts_with("Saved") {
            ui.text_colored(color::ACCENT, format!("{}  {}", icon::CHECK, state.message));
            #[cfg(windows)]
            if ui.button_with_size(format!("{}  Open folder##open", icon::FOLDER), [-1.0, 30.0]) {
                *action = Some(Action::OpenExportFolder);
            }
        }
        let wrap = ui.push_text_wrap_pos_with_pos(ui.cursor_pos()[0] + 300.0);
        ui.text_colored(color::FAINT, format!("Saved in {}", info.export_folder));
        drop(wrap);
    }
    drop(padding);
}

fn file_menu(ui: &Ui, info: &PanelInfo, action: &mut Option<Action>, height: f32) {
    if ui.button_with_size(format!("{}  File##file", icon::MENU), [76.0, height]) {
        ui.open_popup("##file_menu");
    }
    let padding = ui.push_style_var(StyleVar::WindowPadding([8.0, 8.0]));
    let spacing = ui.push_style_var(StyleVar::ItemSpacing([8.0, 6.0]));
    if let Some(_menu) = ui.begin_popup("##file_menu") {
        let items = [
            (icon::NEW, "New", "Ctrl+N", Action::New),
            (icon::FOLDER, "Open\u{2026}", "Ctrl+O", Action::Open),
            (icon::SAVE, "Save", "Ctrl+S", Action::Save),
            (
                icon::SAVE,
                "Save as\u{2026}",
                "Ctrl+Shift+S",
                Action::SaveAs,
            ),
        ];
        for (glyph, label, key, choice) in items {
            if ui
                .menu_item_config(format!("{glyph}  {label}"))
                .shortcut(key)
                .build()
            {
                *action = Some(choice);
            }
        }
        ui.separator();
        if ui
            .menu_item_config(format!("{}  Export\u{2026}", icon::EXPORT))
            .shortcut("Ctrl+E")
            .enabled(info.triangles > 0)
            .build()
        {
            *action = Some(Action::Export);
        }
        ui.text_colored(color::FAINT, "Open reads .lem projects and .obj files.");
    }
    drop(spacing);
    drop(padding);
}

/// Every shortcut, for looking up.
fn help_popup(ui: &Ui, fonts: &Fonts) {
    let padding = ui.push_style_var(StyleVar::WindowPadding([16.0, 14.0]));
    if let Some(_popup) = ui.begin_popup("##help_popup") {
        let groups: [(&str, &[(&str, &str)]); 5] = [
            (
                "VIEW",
                &[
                    ("Turn", "Right-drag"),
                    ("Slide", "Hold the wheel and drag"),
                    ("Zoom", "Scroll"),
                    ("Front, right, top", "F1, F3, F7 (Ctrl: opposite)"),
                    ("Flat view", "F5"),
                    ("Reset view", "Home"),
                ],
            ),
            (
                "SELECT",
                &[
                    ("Vertex, edge, face, part", "1, 2, 3, 4"),
                    ("Select, add, remove", "Click, Shift, Ctrl"),
                    ("Loop", "Double-click an edge"),
                    ("All", "A"),
                    ("Linked", "L or Ctrl+L"),
                    ("More, less", "Ctrl + plus, Ctrl + minus"),
                    ("Invert", "Ctrl+I"),
                ],
            ),
            (
                "CHANGE",
                &[
                    ("Move, rotate, scale", "G, R, S (then X, Y, Z)"),
                    ("Slide edges", "G G"),
                    ("Extrude, inset", "E, I"),
                    ("Loop cut", "Ctrl+R"),
                    ("Bevel", "Ctrl+B"),
                    ("Duplicate", "Shift+D"),
                    ("Merge, fill, delete", "M, F, X"),
                ],
            ),
            (
                "WHILE DRAGGING",
                &[
                    ("Snap", "Hold Ctrl"),
                    ("Confirm, cancel", "Click or Enter, Esc or right-click"),
                    ("More cuts or segments", "Scroll"),
                ],
            ),
            (
                "FILE",
                &[
                    ("New, open, save", "Ctrl+N, Ctrl+O, Ctrl+S"),
                    ("Save as", "Ctrl+Shift+S"),
                    ("Export", "Ctrl+E"),
                    ("Undo, redo", "Ctrl+Z, Ctrl+Y"),
                ],
            ),
        ];
        // Two columns, so the sheet fits on small screens.
        let column = |groups: &[(&str, &[(&str, &str)])]| {
            ui.group(|| {
                for (title, rows) in groups {
                    heading(ui, fonts, title);
                    for (what, keys) in *rows {
                        ui.text_colored(color::MUTED, what);
                        // Measured from the group's start.
                        ui.same_line_with_pos(170.0);
                        ui.text(keys);
                    }
                    ui.dummy([0.0, 4.0]);
                }
            });
        };
        column(&groups[..2]);
        ui.same_line_with_spacing(0.0, 32.0);
        column(&groups[2..]);
    }
    drop(padding);
}

/// True if this popup is open. Opening an open popup again would reset it, which
/// loses typing in progress. The bindings lack this call, so it goes to Dear ImGui.
fn popup_is_open(id: &str) -> bool {
    let id = std::ffi::CString::new(id).unwrap_or_default();
    // SAFETY: a context exists while the interface is built, and `id` ends in a zero.
    unsafe { imgui::sys::igIsPopupOpen(id.as_ptr(), 0) }
}

/// Asks what to do about unsaved changes, and takes typed paths where there is no
/// system file dialog. Both block the rest of the interface while open.
fn dialogs(ui: &Ui, state: &mut PanelState, info: &PanelInfo, action: &mut Option<Action>) {
    let padding = ui.push_style_var(StyleVar::WindowPadding([18.0, 16.0]));
    if let Some(question) = info.confirm {
        if !popup_is_open("Unsaved changes") {
            ui.open_popup("Unsaved changes");
        }
        ui.modal_popup_config("Unsaved changes")
            .always_auto_resize(true)
            .movable(false)
            .build(|| {
                ui.text(question);
                // Wide enough for three readable buttons.
                ui.dummy([340.0, 6.0]);
                if ui.is_key_pressed(imgui::Key::Escape) {
                    *action = Some(Action::ConfirmCancel);
                }
                button_row(
                    ui,
                    action,
                    &[
                        ("Save##confirm", Action::ConfirmSave, ""),
                        ("Don't save##confirm", Action::ConfirmDiscard, ""),
                        ("Cancel##confirm", Action::ConfirmCancel, ""),
                    ],
                    true,
                );
            });
    }
    if let Some(prompt) = &mut state.path_prompt {
        let title = match prompt.purpose {
            PathPurpose::Open => "Open a file",
            PathPurpose::SaveAs => "Save as",
        };
        if !popup_is_open(title) {
            ui.open_popup(title);
        }
        ui.modal_popup_config(title)
            .always_auto_resize(true)
            .movable(false)
            .build(|| {
                ui.text_colored(color::MUTED, "Path to the file");
                ui.set_next_item_width(420.0);
                // Ready for typing, so Enter alone accepts the suggested path.
                if ui.is_window_appearing() {
                    ui.set_keyboard_focus_here();
                }
                if ui.is_key_pressed(imgui::Key::Escape) {
                    *action = Some(Action::PathCancelled);
                }
                if ui
                    .input_text("##path", &mut prompt.text)
                    .enter_returns_true(true)
                    .build()
                {
                    *action = Some(Action::PathChosen);
                }
                button_row(
                    ui,
                    action,
                    &[
                        ("OK##path", Action::PathChosen, ""),
                        ("Cancel##path", Action::PathCancelled, ""),
                    ],
                    true,
                );
            });
    }
    drop(padding);
}

fn triangle_budget(ui: &Ui, triangles: usize) {
    let fraction = triangles as f32 / ROBLOX_MAX_TRIANGLES as f32;
    ui.text_colored(color::MUTED, "Triangles");
    ui.same_line();
    let text = format!("{triangles} / {ROBLOX_MAX_TRIANGLES}");
    let width = ui.calc_text_size(&text)[0];
    ui.same_line_with_pos(
        ui.cursor_pos()[0].max(ui.content_region_avail()[0] + ui.cursor_pos()[0] - width),
    );
    let over = triangles > ROBLOX_MAX_TRIANGLES;
    ui.text_colored(if over { color::DANGER } else { color::TEXT }, text);
    let bar = ui.push_style_color(
        StyleColor::PlotHistogram,
        if over { color::DANGER } else { color::ACCENT },
    );
    ProgressBar::new(fraction.min(1.0))
        .size([-1.0, 5.0])
        .overlay_text("")
        .build(ui);
    drop(bar);
}

/// A tool button: ID, icon, action, name and shortcut.
type Tool = (&'static str, char, Action, &'static str, &'static str);

fn tool_strip(ui: &Ui, info: &PanelInfo, fonts: &Fonts, action: &mut Option<Action>) {
    let any = info.selected_vertices > 0;
    let active = info.transform_help.map(|(kind, _)| kind);
    let font = ui.push_font(fonts.tool);
    let groups: [&[Tool]; 5] = [
        &[
            (
                "move",
                icon::MOVE,
                Action::Transform(Kind::Move),
                "Move",
                "G",
            ),
            (
                "rotate",
                icon::ROTATE,
                Action::Transform(Kind::Rotate),
                "Rotate",
                "R",
            ),
            (
                "scale",
                icon::SCALE,
                Action::Transform(Kind::Scale),
                "Scale",
                "S",
            ),
        ],
        &[
            (
                "extrude",
                icon::EXTRUDE,
                Action::Extrude,
                "Extrude faces",
                "E",
            ),
            ("inset", icon::INSET, Action::Inset, "Inset faces", "I"),
            (
                "duplicate",
                icon::DUPLICATE,
                Action::Duplicate,
                "Duplicate",
                "Shift+D",
            ),
        ],
        &[
            ("merge", icon::MERGE, Action::Merge, "Merge at center", "M"),
            (
                "fill",
                icon::FILL,
                Action::Fill,
                "Fill with an edge or face",
                "F",
            ),
            (
                "subdivide",
                icon::SUBDIVIDE,
                Action::Subdivide,
                "Subdivide",
                "",
            ),
        ],
        &[
            (
                "loopcut",
                icon::LOOP_CUT,
                Action::LoopCut,
                "Loop cut: point at an edge, then click",
                "Ctrl+R",
            ),
            (
                "slide",
                icon::SLIDE,
                Action::Transform(Kind::Slide),
                "Slide edges along their neighbors",
                "G G",
            ),
            (
                "bridge",
                icon::BRIDGE,
                Action::Bridge,
                "Bridge two edge loops",
                "",
            ),
            (
                "bevel",
                icon::BEVEL,
                Action::Bevel,
                "Bevel edges: drag for width, scroll for segments",
                "Ctrl+B",
            ),
        ],
        &[("delete", icon::DELETE, Action::Delete, "Delete", "X")],
    ];
    for (g, group) in groups.iter().enumerate() {
        if g > 0 {
            ui.dummy([0.0, 2.0]);
            let p = ui.cursor_screen_pos();
            ui.get_window_draw_list()
                .add_line(
                    [p[0] + 8.0, p[1]],
                    [p[0] + TOOL_BUTTON - 8.0, p[1]],
                    color::BORDER,
                )
                .build();
            ui.dummy([0.0, 4.0]);
        }
        for (id, glyph, choice, name, key) in group.iter() {
            let lit = matches!(choice, Action::Transform(kind) if Some(*kind) == active)
                || (*choice == Action::Inset && active == Some(Kind::Inset))
                || (*choice == Action::LoopCut && info.loop_cut.is_some())
                || (*choice == Action::Bevel && active == Some(Kind::Bevel));
            // A loop cut picks its own edge, so it needs only something to cut.
            let enabled = if *choice == Action::LoopCut {
                info.vertices > 0
            } else {
                any
            };
            if icon_button(ui, id, *glyph, lit, enabled, TOOL_BUTTON) {
                *action = Some(*choice);
            }
            tooltip(ui, name, key);
        }
        if g == 1 {
            if icon_button(ui, "mirror", icon::MIRROR, false, any, TOOL_BUTTON) {
                ui.open_popup("##mirror_popup");
            }
            tooltip(ui, "Mirror", "");
        }
    }
    drop(font);
    if let Some(_popup) = ui.begin_popup("##mirror_popup") {
        ui.text_colored(color::MUTED, "Mirror across");
        for (axis, name) in ["X", "Y", "Z"].iter().enumerate() {
            let text = ui.push_style_color(StyleColor::Text, color::AXIS[axis]);
            if ui.button_with_size(format!("{name}##mirror{axis}"), [44.0, 32.0]) {
                *action = Some(Action::Mirror(axis));
                ui.close_current_popup();
            }
            drop(text);
            if axis < 2 {
                ui.same_line();
            }
        }
    }
}

fn panel(
    ui: &Ui,
    state: &mut PanelState,
    info: &PanelInfo,
    fonts: &Fonts,
    action: &mut Option<Action>,
) {
    // Add shape: a grid of tiles.
    heading(ui, fonts, "ADD SHAPE");
    let gap = 6.0;
    let columns = 4.0;
    let tile_width = ((ui.content_region_avail()[0] - gap * (columns - 1.0)) / columns).floor();
    let tile = [tile_width, 56.0];
    for (i, shape) in Shape::ALL.into_iter().enumerate() {
        if i % 4 != 0 {
            ui.same_line_with_spacing(0.0, gap);
        }
        let start = ui.cursor_screen_pos();
        if ui.invisible_button(format!("##add{}", shape.name()), tile) {
            *action = Some(Action::Add(shape));
        }
        let hovered = ui.is_item_hovered();
        let pressed = ui.is_item_active();
        let list = ui.get_window_draw_list();
        let fill = if pressed {
            color::FRAME_ACTIVE
        } else if hovered {
            color::FRAME_HOVER
        } else {
            color::FRAME
        };
        list.add_rect(start, [start[0] + tile[0], start[1] + tile[1]], fill)
            .filled(true)
            .rounding(8.0)
            .build();
        let font = ui.push_font(fonts.tool);
        let glyph = shape.icon().to_string();
        let size = ui.calc_text_size(&glyph);
        let tint = if hovered { color::ACCENT } else { color::TEXT };
        list.add_text(
            [start[0] + (tile[0] - size[0]) / 2.0, start[1] + 8.0],
            tint,
            &glyph,
        );
        drop(font);
        let font = ui.push_font(fonts.heading);
        let size = ui.calc_text_size(shape.name());
        list.add_text(
            [start[0] + (tile[0] - size[0]) / 2.0, start[1] + 36.0],
            color::MUTED,
            shape.name(),
        );
        drop(font);
        tooltip(ui, shape.edit_label(), "");
        if i % 4 == 3 {
            ui.dummy([0.0, 0.0]);
        }
    }
    ui.dummy([0.0, 6.0]);

    // Shape settings.
    let editing = match info.target {
        Target::Shape(shape) => Some(shape),
        _ => None,
    };
    let title = match info.target {
        Target::Shape(shape) => shape.name().to_uppercase(),
        _ => "NEW SHAPE SETTINGS".to_string(),
    };
    heading(ui, fonts, &title);
    if editing.is_none() {
        // With no shape selected these only matter for the next shape added, so they
        // stay folded away unless asked for.
        ui.same_line_with_pos(ui.content_region_max()[0] - 40.0);
        let font = ui.push_font(fonts.heading);
        let label = if state.show_defaults { "HIDE" } else { "SHOW" };
        ui.text_colored(color::ACCENT, label);
        drop(font);
        let toggle = ui.is_item_clicked();
        tooltip(ui, "Settings used by the next shape you add", "");
        if toggle {
            state.show_defaults = !state.show_defaults;
        }
    }
    if editing.is_some() {
        ui.same_line();
        let font = ui.push_font(fonts.heading);
        ui.text_colored(color::ACCENT, "LIVE");
        drop(font);
        tooltip(ui, "Changes apply to the selected shape", "");
    }
    match info.target {
        Target::Edited(shape) => note(
            ui,
            icon::INFO,
            color::MUTED,
            &format!(
                "This {} was reshaped, so these settings are for the next shape.",
                shape.name().to_lowercase()
            ),
        ),
        Target::Several => note(
            ui,
            icon::INFO,
            color::MUTED,
            "Several shapes are selected. Select one to change it.",
        ),
        _ => {}
    }
    let open = editing.is_some() || state.show_defaults;
    let shows = |setting: Setting| open && editing.is_none_or(|s| s.uses(setting));
    if open {
        shape_settings(ui, state, shows);
    }
    ui.dummy([0.0, 6.0]);

    // Selection.
    heading(ui, fonts, "SELECTION");
    button_row(
        ui,
        action,
        &[
            ("All##sel", Action::SelectAll, ""),
            ("None##sel", Action::SelectNone, ""),
            ("Invert##sel", Action::SelectInvert, ""),
        ],
        true,
    );
    button_row(
        ui,
        action,
        &[
            ("Linked##sel", Action::SelectLinked, "Whole connected parts"),
            ("More##sel", Action::SelectMore, ""),
            ("Less##sel", Action::SelectLess, ""),
        ],
        true,
    );
    let edge_modes = matches!(info.mode, ModeChoice::Vertex | ModeChoice::Edge);
    button_row(
        ui,
        action,
        &[
            (
                "Loop##sel",
                Action::SelectLoops,
                "Grow selected edges along their loops. Double-click an edge for the same.",
            ),
            (
                "Ring##sel",
                Action::SelectRings,
                "Grow selected edges across their faces, side by side",
            ),
        ],
        edge_modes && info.selected_vertices > 0,
    );
    let disabled = ui.begin_disabled(info.selected_vertices == 0);
    if ui.button_with_size(
        format!("{}  Merge close vertices##clean", icon::MERGE_CLOSE),
        [-1.0, 30.0],
    ) {
        *action = Some(Action::MergeByDistance);
    }
    disabled.end();
}

/// The size, detail and location fields of the shape settings.
fn shape_settings(ui: &Ui, state: &mut PanelState, shows: impl Fn(Setting) -> bool) {
    row(ui, "Size");
    Drag::new("##size")
        .range(0.01, 1000.0)
        .speed(0.05)
        .display_format("%.2f")
        .flags(SliderFlags::ALWAYS_CLAMP)
        .build(ui, &mut state.size);
    let int_slider = |label: &str, id: &str, setting, value: &mut i32, min, max| {
        if shows(setting) {
            row(ui, label);
            ui.slider_config(id, min, max)
                .flags(SliderFlags::ALWAYS_CLAMP)
                .build(value);
        }
    };
    int_slider(
        "Segments",
        "##segments",
        Setting::Segments,
        &mut state.segments,
        3,
        64,
    );
    int_slider("Rings", "##rings", Setting::Rings, &mut state.rings, 2, 32);
    int_slider(
        "Subdivisions",
        "##subdivisions",
        Setting::Subdivisions,
        &mut state.subdivisions,
        0,
        MAX_SUBDIVISIONS.min(4) as i32,
    );
    int_slider("Grid cuts", "##cuts", Setting::Cuts, &mut state.cuts, 1, 64);
    row(ui, "Location");
    ui.input_float3("##location", &mut state.location)
        .display_format("%.2f")
        .build();
}

fn status_bar(ui: &Ui, state: &PanelState, info: &PanelInfo) {
    let hint = if info.mode == ModeChoice::Part {
        "Right-drag to orbit  \u{2022}  Hold the wheel to pan  \u{2022}  Scroll to zoom"
    } else {
        "Right-drag to orbit  \u{2022}  Hold the wheel to pan  \u{2022}  Scroll to zoom  \u{2022}  Double-click an edge for its loop"
    };
    if state.message.is_empty() {
        ui.text_colored(color::FAINT, hint);
    } else {
        ui.text_colored(color::MUTED, &state.message);
    }
    let mut parts: Vec<(String, [f32; 4])> = Vec::new();
    if info.open_edges > 0 {
        parts.push((
            format!("{}  {} open edges", icon::WARNING, info.open_edges),
            color::WARNING,
        ));
    }
    parts.push((
        format!(
            "{} of {} vertices selected",
            info.selected_vertices, info.vertices
        ),
        color::MUTED,
    ));
    parts.push((format!("Faces {}", info.faces), color::MUTED));
    let over = info.triangles > ROBLOX_MAX_TRIANGLES;
    parts.push((
        format!("Triangles {}", info.triangles),
        if over { color::DANGER } else { color::MUTED },
    ));
    let gap = 24.0;
    let total: f32 = parts
        .iter()
        .map(|(t, _)| ui.calc_text_size(t)[0])
        .sum::<f32>()
        + gap * (parts.len() as f32 - 1.0);
    ui.same_line_with_pos((ui.window_size()[0] - total - 12.0).max(0.0));
    for (i, (text, tint)) in parts.iter().enumerate() {
        if i > 0 {
            ui.same_line_with_spacing(0.0, gap);
        }
        ui.text_colored(*tint, text);
    }
}

/// Where each axis end lands on the gizmo, nearest last so it draws on top.
fn gizmo_points(
    view: &ViewState,
    center: [f32; 2],
    radius: f32,
) -> Vec<(usize, bool, [f32; 2], f32)> {
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let mut points = Vec::with_capacity(6);
    for axis in 0..3 {
        let mut e = [0.0; 3];
        e[axis] = 1.0;
        let (sx, sy, depth) = (dot(e, view.right), -dot(e, view.up), dot(e, view.toward));
        for positive in [true, false] {
            let sign = if positive { 1.0 } else { -1.0 };
            let at = [
                center[0] + sx * radius * sign,
                center[1] + sy * radius * sign,
            ];
            points.push((axis, positive, at, depth * sign));
        }
    }
    points.sort_by(|a, b| a.3.total_cmp(&b.3));
    points
}

fn gizmo(ui: &Ui, info: &PanelInfo, action: &mut Option<Action>) {
    let start = ui.cursor_screen_pos();
    let center = [start[0] + GIZMO / 2.0, start[1] + GIZMO / 2.0];
    let radius = GIZMO / 2.0 - 14.0;
    ui.invisible_button("##gizmo", [GIZMO, GIZMO]);
    let hovered = ui.is_item_hovered();
    let clicked = ui.is_item_clicked();
    let mouse = ui.io().mouse_pos;
    let list = ui.get_window_draw_list();
    if hovered {
        list.add_circle(center, GIZMO / 2.0 - 2.0, [1.0, 1.0, 1.0, 0.06])
            .filled(true)
            .build();
    }
    let points = gizmo_points(&info.view, center, radius);
    let near = |p: [f32; 2]| ((p[0] - mouse[0]).powi(2) + (p[1] - mouse[1]).powi(2)).sqrt();
    let target = points
        .iter()
        .filter(|(_, _, at, _)| near(*at) <= 12.0)
        .max_by(|a, b| a.3.total_cmp(&b.3))
        .map(|(axis, positive, _, _)| (*axis, *positive));
    for (axis, positive, at, _) in &points {
        let tint = color::AXIS[*axis];
        let lit = target == Some((*axis, *positive));
        if *positive {
            list.add_line(center, *at, with_alpha(tint, 0.9))
                .thickness(2.0)
                .build();
            list.add_circle(*at, 9.0, if lit { [1.0; 4] } else { tint })
                .filled(true)
                .build();
            let label = ["X", "Y", "Z"][*axis];
            let size = ui.calc_text_size(label);
            list.add_text(
                [at[0] - size[0] / 2.0, at[1] - size[1] / 2.0],
                [0.08, 0.08, 0.09, 1.0],
                label,
            );
        } else {
            list.add_circle(*at, 7.0, with_alpha(tint, if lit { 0.8 } else { 0.30 }))
                .filled(true)
                .build();
            list.add_circle(*at, 7.0, with_alpha(tint, 0.8))
                .thickness(1.5)
                .build();
        }
    }
    if clicked && let Some((axis, positive)) = target {
        *action = Some(Action::View(ViewPreset::from_axis(axis, positive)));
    }
    if hovered && let Some((axis, positive)) = target {
        let name = match ViewPreset::from_axis(axis, positive) {
            ViewPreset::Front => "Front view (F1)",
            ViewPreset::Back => "Back view (Ctrl+F1)",
            ViewPreset::Right => "Right view (F3)",
            ViewPreset::Left => "Left view (Ctrl+F3)",
            ViewPreset::Top => "Top view (F7)",
            ViewPreset::Bottom => "Bottom view (Ctrl+F7)",
        };
        ui.tooltip_text(name);
    }

    let size = 32.0;
    ui.set_cursor_screen_pos([center[0] - size - 3.0, start[1] + GIZMO + 4.0]);
    if icon_button(ui, "home", icon::HOME, false, true, size) {
        *action = Some(Action::ResetView);
    }
    tooltip(ui, "Reset view", "Home");
    ui.same_line_with_spacing(0.0, 6.0);
    if icon_button(ui, "ortho", icon::ORTHO, info.view.orthographic, true, size) {
        *action = Some(Action::ToggleOrtho);
    }
    tooltip(
        ui,
        if info.view.orthographic {
            "Flat view (orthographic)"
        } else {
            "Depth view (perspective)"
        },
        "F5",
    );
}

fn viewport_overlays(ui: &Ui, info: &PanelInfo, fonts: &Fonts, display: [f32; 2]) {
    let area = viewport_area(display);
    let list = ui.get_background_draw_list();
    let center_x = area[0] + area[2] / 2.0;

    let mut parts: Vec<(String, [f32; 4])> = Vec::new();
    if let Some(cuts) = info.loop_cut {
        parts.push(("Loop cut".to_string(), color::ACCENT));
        let plural = if cuts == 1 { "cut" } else { "cuts" };
        parts.push((format!("{cuts} {plural}"), color::TEXT));
        parts.push(("Scroll or +/- for more".to_string(), color::FAINT));
        parts.push(("Click an edge to cut".to_string(), color::FAINT));
        parts.push(("Esc cancel".to_string(), color::FAINT));
    }
    if let Some((kind, axis)) = info.transform_help {
        parts.push((kind.name().to_string(), color::ACCENT));
        if let Some((width, segments)) = info.bevel {
            let plural = if segments == 1 { "segment" } else { "segments" };
            parts.push((format!("Width {width:.2}"), color::TEXT));
            parts.push((format!("{segments} {plural}"), color::TEXT));
            parts.push(("Scroll for more".to_string(), color::FAINT));
        }
        if !matches!(kind, Kind::Inset | Kind::Slide | Kind::Bevel) {
            for (i, name) in ["X", "Y", "Z"].iter().enumerate() {
                let lit = axis == Some(i);
                parts.push((
                    name.to_string(),
                    if lit { color::AXIS[i] } else { color::MUTED },
                ));
            }
            parts.push(("lock axis".to_string(), color::FAINT));
        }
        parts.push(("Ctrl snap".to_string(), color::FAINT));
        parts.push(("Click confirm".to_string(), color::FAINT));
        parts.push(("Esc cancel".to_string(), color::FAINT));
    }
    if !parts.is_empty() {
        let gap = 12.0;
        let total = parts
            .iter()
            .map(|(t, _)| ui.calc_text_size(t)[0])
            .sum::<f32>()
            + gap * (parts.len() as f32 - 1.0);
        let top = area[1] + 14.0;
        let left = center_x - total / 2.0;
        list.add_rect(
            [left - 16.0, top],
            [left + total + 16.0, top + 34.0],
            color::OVERLAY,
        )
        .filled(true)
        .rounding(17.0)
        .build();
        let mut x = left;
        for (text, tint) in &parts {
            list.add_text([x, top + 8.0], *tint, text);
            x += ui.calc_text_size(text)[0] + gap;
        }
    }

    if info.vertices == 0 && parts.is_empty() {
        let middle = area[1] + area[3] / 2.0 - 40.0;
        let font = ui.push_font(fonts.title);
        let glyph = icon::CUBE.to_string();
        let size = ui.calc_text_size(&glyph);
        list.add_text(
            [center_x - size[0] / 2.0, middle - 44.0],
            color::ACCENT,
            &glyph,
        );
        let title = "Start with a shape";
        let size = ui.calc_text_size(title);
        list.add_text([center_x - size[0] / 2.0, middle], color::TEXT, title);
        drop(font);
        let line = "Pick one under Add shape on the right.";
        let size = ui.calc_text_size(line);
        list.add_text(
            [center_x - size[0] / 2.0, middle + 36.0],
            color::MUTED,
            line,
        );
    }
}

/// Draws a selection shape being dragged.
/// Lines on top of the 3D view, such as where a loop cut will go.
pub fn draw_lines(ui: &Ui, lines: &[[[f32; 2]; 2]]) {
    let list = ui.get_foreground_draw_list();
    for &[a, b] in lines {
        list.add_line(a, b, [1.0, 0.85, 0.25, 1.0])
            .thickness(2.0)
            .build();
    }
}

pub fn draw_region(ui: &Ui, region: &Region) {
    let list = ui.get_foreground_draw_list();
    let line = [1.0, 0.72, 0.36, 1.0];
    let fill = [1.0, 0.62, 0.2, 0.08];
    match region {
        Region::Box { a, b } => {
            let min = [a[0].min(b[0]), a[1].min(b[1])];
            let max = [a[0].max(b[0]), a[1].max(b[1])];
            list.add_rect(min, max, fill).filled(true).build();
            list.add_rect(min, max, line).build();
        }
        Region::Brush { path, radius } => {
            if let Some(last) = path.last() {
                list.add_circle(*last, *radius, fill).filled(true).build();
                list.add_circle(*last, *radius, line).build();
            }
        }
        Region::Lasso { points } => {
            list.add_polyline(points.clone(), line)
                .thickness(1.5)
                .build();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use modeler_mesh::Mesh;

    #[test]
    fn default_values_make_a_valid_version_of_every_shape() {
        let state = PanelState::default();
        for shape in Shape::ALL {
            let mut mesh = Mesh::new();
            mesh.add_primitive(state.primitive(shape), state.location)
                .unwrap();
            mesh.validate().unwrap();
            assert!(mesh.face_count() > 0);
            assert!(shape.edit_label().starts_with("Add "));
            assert!(shape.edit_label().ends_with(shape.name()));
        }
    }

    #[test]
    fn the_slider_ends_are_all_valid_shapes() {
        for (segments, rings, subdivisions, cuts) in [(3, 2, 0, 1), (64, 32, 4, 64)] {
            let state = PanelState {
                segments,
                rings,
                subdivisions,
                cuts,
                ..Default::default()
            };
            for shape in Shape::ALL {
                let mut mesh = Mesh::new();
                mesh.add_primitive(state.primitive(shape), [0.0; 3])
                    .unwrap();
                mesh.validate().unwrap();
            }
        }
    }

    #[test]
    fn shape_values_go_to_the_sliders_and_back() {
        let samples = PanelState {
            size: 3.0,
            segments: 7,
            rings: 5,
            subdivisions: 2,
            cuts: 6,
            ..Default::default()
        };
        for shape in Shape::ALL {
            let primitive = samples.primitive(shape);
            let mut state = PanelState::default();
            state.show_values(primitive, [1.0, 2.0, 3.0]);
            assert_eq!(state.primitive(shape), primitive, "{shape:?}");
            assert_eq!(state.location, [1.0, 2.0, 3.0]);
        }
    }

    #[test]
    fn each_shape_uses_the_right_sliders() {
        let uses = |s: Shape| {
            [
                Setting::Segments,
                Setting::Rings,
                Setting::Subdivisions,
                Setting::Cuts,
            ]
            .map(|setting| s.uses(setting))
        };
        assert_eq!(uses(Shape::Cube), [false, false, false, false]);
        assert_eq!(uses(Shape::Cone), [true, false, false, false]);
        assert_eq!(uses(Shape::Cylinder), [true, false, false, false]);
        assert_eq!(uses(Shape::UvSphere), [true, true, false, false]);
        assert_eq!(uses(Shape::Icosphere), [false, false, true, false]);
        assert_eq!(uses(Shape::Grid), [false, false, false, true]);
        assert_eq!(uses(Shape::Circle), [true, false, false, false]);
    }

    fn info<'a>(target: Target, warnings: &'a [String], transform: bool) -> PanelInfo<'a> {
        PanelInfo {
            triangles: 12,
            vertices: 8,
            faces: 6,
            open_edges: 4,
            warnings,
            target,
            mode: ModeChoice::Part,
            selected_vertices: 8,
            transform_help: transform.then_some((Kind::Move, Some(0))),
            loop_cut: None,
            bevel: None,
            undo_label: Some("Add Cube"),
            redo_label: Some("Add Cube"),
            export_folder: "exports",
            view: ViewState {
                right: [0.77, 0.64, 0.0],
                up: [-0.32, 0.38, 0.87],
                toward: [0.55, -0.66, 0.5],
                orthographic: false,
            },
            document: "a rather long project name that needs shortening.lem",
            unsaved: true,
            confirm: None,
        }
    }

    /// Clicks and drags all over the window, with no window or graphics card. Two widgets
    /// sharing an ID crashed the app when clicked, and this would crash the same way. It
    /// also opens the export and mirror popups and clicks inside them.
    #[test]
    fn clicking_and_dragging_everywhere_is_safe() {
        let _turn = crate::theme::IMGUI_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut context = imgui::Context::create();
        context.set_ini_filename(None);
        let fonts = crate::theme::install(&mut context);
        context.fonts().build_alpha8_texture();
        context.io_mut().display_size = [1280.0, 800.0];
        context.io_mut().delta_time = 1.0 / 60.0;
        let warnings = vec!["A warning".to_string()];

        let mut actions = std::collections::HashSet::new();
        // The last two cases show the save-changes question and the path box, which
        // block everything else while open.
        for (case, (target, transform)) in [
            (Target::Nothing, false),
            (Target::Shape(Shape::Cube), true),
            (Target::Edited(Shape::Icosphere), false),
            (Target::Several, false),
            (Target::Nothing, false),
            (Target::Nothing, false),
        ]
        .into_iter()
        .enumerate()
        {
            let mut info = info(target, &warnings, transform);
            if case == 4 {
                info.confirm = Some("Save changes to sword.lem first?");
            }
            // Some buttons only work in edge mode, and the loop cut hint must draw too.
            if target == Target::Several {
                info.mode = ModeChoice::Edge;
                info.loop_cut = Some(2);
            }
            if target == Target::Edited(Shape::Icosphere) {
                info.transform_help = Some((Kind::Bevel, None));
                info.bevel = Some((0.25, 3));
            }
            let mut state = PanelState {
                message: "Saved model.obj".to_string(),
                export_format: if case == 2 {
                    ExportFormat::Glb
                } else {
                    ExportFormat::Obj
                },
                path_prompt: (case == 5).then(|| PathPrompt {
                    purpose: PathPurpose::SaveAs,
                    text: "/somewhere/model.lem".to_string(),
                }),
                ..Default::default()
            };
            let mut frame = |context: &mut imgui::Context, state: &mut PanelState| {
                let ui = context.new_frame();
                let output = build(ui, state, &info, &fonts);
                assert!(
                    output.rects.len() >= 4,
                    "every area must report its rectangle"
                );
                context.render();
                if let Some(action) = output.action {
                    actions.insert(format!("{action:?}"));
                }
            };
            frame(&mut context, &mut state);
            frame(&mut context, &mut state);
            for y in (0..800).step_by(9) {
                for x in (0..1280).step_by(23) {
                    let (x, y) = (x as f32, y as f32);
                    context.io_mut().add_mouse_pos_event([x, y]);
                    frame(&mut context, &mut state);
                    context
                        .io_mut()
                        .add_mouse_button_event(imgui::MouseButton::Left, true);
                    frame(&mut context, &mut state);
                    // A short drag: enough to move a slider, small enough to release on a button.
                    context.io_mut().add_mouse_pos_event([x + 8.0, y]);
                    frame(&mut context, &mut state);
                    context
                        .io_mut()
                        .add_mouse_button_event(imgui::MouseButton::Left, false);
                    frame(&mut context, &mut state);
                    frame(&mut context, &mut state);
                }
            }
            // Popups close on a click elsewhere, so the sweep above rarely finds them open.
            // Open each one, then click one spot inside, for every spot near it.
            let mut click = |context: &mut imgui::Context, state: &mut PanelState, x, y| {
                context.io_mut().add_mouse_pos_event([x, y]);
                frame(context, state);
                context
                    .io_mut()
                    .add_mouse_button_event(imgui::MouseButton::Left, true);
                frame(context, state);
                context
                    .io_mut()
                    .add_mouse_button_event(imgui::MouseButton::Left, false);
                frame(context, state);
                frame(context, state);
            };
            // The export button (top right) and the mirror tool (in the tool strip).
            for (opener, xs, ys) in [
                ([1215.0, 25.0], 900..1280, 52..480),
                ([38.0, 365.0], 60..320, 300..520),
                // The File menu, beside the name at the top left.
                ([200.0, 25.0], 150..460, 40..260),
            ] {
                for y in ys.step_by(11) {
                    for x in xs.clone().step_by(13) {
                        click(&mut context, &mut state, opener[0], opener[1]);
                        click(&mut context, &mut state, x as f32, y as f32);
                    }
                }
                // Close whatever is still open.
                click(&mut context, &mut state, 640.0, 790.0);
            }
            assert!(state.size.is_finite() && state.size > 0.0);
            assert!((3..=64).contains(&state.segments));
            assert!((2..=32).contains(&state.rings));
            assert!((0..=4).contains(&state.subdivisions));
            assert!((1..=64).contains(&state.cuts));
            assert!(state.scale > 0.0);
        }
        // The clicks must really reach the controls, including inside the popups.
        for expected in [
            "Add(Cube)",
            "Undo",
            "Redo",
            "Extrude",
            "SelectAll",
            "ResetView",
            "Export",
            "Mirror(1)",
            "LoopCut",
            "Bridge",
            "SelectLoops",
            "SelectRings",
            "Transform(Slide)",
            "Bevel",
            "New",
            "Open",
            "Save",
            "SaveAs",
            "ConfirmSave",
            "ConfirmDiscard",
            "ConfirmCancel",
            "PathChosen",
            "PathCancelled",
        ] {
            assert!(
                actions.contains(expected),
                "never clicked {expected}: {actions:?}"
            );
        }
        assert!(
            actions.iter().any(|a| a.starts_with("View(")),
            "the gizmo was never clicked"
        );
        assert!(actions.iter().any(|a| a.starts_with("SetMode(")));
    }

    #[test]
    fn the_panel_ends_below_its_contents_but_never_past_the_status_bar() {
        let _turn = crate::theme::IMGUI_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut context = imgui::Context::create();
        context.set_ini_filename(None);
        let fonts = crate::theme::install(&mut context);
        context.fonts().build_alpha8_texture();
        context.io_mut().delta_time = 1.0 / 60.0;
        let warnings = Vec::new();
        let panel_height = |context: &mut imgui::Context, target, height: f32| {
            context.io_mut().display_size = [1280.0, height];
            let info = info(target, &warnings, false);
            let mut state = PanelState::default();
            let mut rect = [0.0; 4];
            for _ in 0..3 {
                let ui = context.new_frame();
                // The panel comes after the two bars and the tool strip.
                rect = build(ui, &mut state, &info, &fonts).rects[3];
                context.render();
            }
            rect[3]
        };
        let empty = panel_height(&mut context, Target::Nothing, 800.0);
        let sphere = panel_height(&mut context, Target::Shape(Shape::UvSphere), 800.0);
        assert!(empty < sphere, "{empty} vs {sphere}");
        assert!(sphere < 800.0 - TOP_BAR - STATUS_BAR - MARGIN * 2.0);
        let small = panel_height(&mut context, Target::Shape(Shape::UvSphere), 400.0);
        assert_eq!(small, 400.0 - TOP_BAR - STATUS_BAR - MARGIN * 2.0);
    }

    #[test]
    fn gizmo_axes_point_where_the_camera_sees_them() {
        // Front view: X to the right, Z up, Y away from the viewer.
        let view = ViewState {
            right: [1.0, 0.0, 0.0],
            up: [0.0, 0.0, 1.0],
            toward: [0.0, -1.0, 0.0],
            orthographic: false,
        };
        let points = gizmo_points(&view, [100.0, 100.0], 40.0);
        let find = |axis, positive| {
            points
                .iter()
                .find(|p| p.0 == axis && p.1 == positive)
                .unwrap()
        };
        assert_eq!(find(0, true).2, [140.0, 100.0]);
        assert_eq!(
            find(2, true).2,
            [100.0, 60.0],
            "up on screen is a smaller y"
        );
        assert_eq!(find(1, false).3, 1.0, "-Y faces the viewer");
        assert_eq!(
            points.last().map(|p| (p.0, p.1)),
            Some((1, false)),
            "nearest is drawn last"
        );
    }

    #[test]
    fn gizmo_ends_give_matching_views() {
        assert_eq!(ViewPreset::from_axis(0, true), ViewPreset::Right);
        assert_eq!(ViewPreset::from_axis(0, false), ViewPreset::Left);
        assert_eq!(ViewPreset::from_axis(1, true), ViewPreset::Back);
        assert_eq!(ViewPreset::from_axis(1, false), ViewPreset::Front);
        assert_eq!(ViewPreset::from_axis(2, true), ViewPreset::Top);
        assert_eq!(ViewPreset::from_axis(2, false), ViewPreset::Bottom);
        // A view from an axis's end must really put the camera there.
        for (axis, positive) in [
            (0, true),
            (0, false),
            (1, true),
            (1, false),
            (2, true),
            (2, false),
        ] {
            let (yaw, pitch) = ViewPreset::from_axis(axis, positive).angles();
            let mut camera = crate::camera::Camera::default();
            camera.set_view(yaw, pitch);
            let toward = camera.toward_camera();
            let sign = if positive { 1.0 } else { -1.0 };
            assert!(
                (toward[axis] - sign).abs() < 1e-4,
                "{axis} {positive}: {toward:?}"
            );
        }
    }

    #[test]
    fn the_viewport_area_sits_between_the_panels() {
        let [x, y, w, h] = viewport_area([1280.0, 800.0]);
        assert!(x > TOOL_BUTTON && y == TOP_BAR);
        assert!(x + w < 1280.0 - RIGHT_PANEL && y + h == 800.0 - STATUS_BAR);
        let tiny = viewport_area([100.0, 50.0]);
        assert!(tiny[2] >= 1.0 && tiny[3] >= 1.0);
    }

    #[test]
    fn size_means_overall_width_for_each_shape() {
        let state = PanelState {
            size: 4.0,
            ..Default::default()
        };
        for shape in Shape::ALL {
            let mut mesh = Mesh::new();
            mesh.add_primitive(state.primitive(shape), [0.0; 3])
                .unwrap();
            let widest = mesh
                .vertices()
                .map(|(_, v)| v.position()[0].abs().max(v.position()[1].abs()))
                .fold(0.0, f32::max);
            assert!((widest - 2.0).abs() < 0.2, "{shape:?}: {widest}");
        }
    }
}
