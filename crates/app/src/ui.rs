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
use mixer_mesh::{
    AUTO_SMOOTH_ANGLE, BevelSettings, BooleanOp, CustomProfile, DEFAULT_MERGE_DISTANCE,
    Intersection, MAX_ARRAY_COUNT, MAX_BEVEL_SEGMENTS, MAX_SPREAD, MAX_SUBDIVISIONS, Material,
    MaterialId, Miter, Modifier, ModifierId, Primitive, ProfilePreset, ProfileType, WidthType,
};

use crate::editor::Target;
use crate::export::ExportFormat;
use crate::picking::Region;
use crate::search::{self, SearchState};
use crate::theme::{Fonts, color, icon};
use crate::transform::{Kind, SNAP_LIMITS, SnapSteps};

pub const TOP_BAR: f32 = 52.0;
pub const STATUS_BAR: f32 = 30.0;
pub const MARGIN: f32 = 10.0;
pub const RIGHT_PANEL: f32 = 300.0;
const TOOL_BUTTON: f32 = 40.0;
/// Buttons in the tool strip, counting Mirror, and the gaps between their groups.
const TOOL_COUNT: f32 = 18.0;
const TOOL_GROUPS: f32 = 5.0;

/// The tool strip's button size: full size when the window is tall enough, smaller on
/// a short screen so that every tool still fits.
pub fn tool_button_size(display_height: f32) -> f32 {
    let room = display_height - TOP_BAR - STATUS_BAR - MARGIN * 2.0 - 16.0;
    let gaps = (TOOL_GROUPS - 1.0) * 14.0;
    ((room - gaps) / TOOL_COUNT - 4.0)
        .clamp(24.0, TOOL_BUTTON)
        .floor()
}
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
    pub const ALL: [Shape; 7] = [
        Shape::Cube,
        Shape::Cone,
        Shape::Cylinder,
        Shape::UvSphere,
        Shape::Icosphere,
        Shape::Grid,
        Shape::Circle,
    ];

    pub fn icon(self) -> char {
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
    /// The Properties panel's content height, as for `panel_content`.
    pub props_content: f32,
    /// The Materials panel's content height.
    pub materials_content: f32,
    /// Where the two side panels are, and how big.
    pub dock: Dock,
    /// Feature search, while it is open.
    pub search: Option<SearchState>,
    /// The material the Materials panel shows; `None` is the default look.
    pub active_material: Option<MaterialId>,
    /// The active material as being edited, before it is saved.
    pub material_draft: Option<(MaterialId, Material)>,
    /// The snapping popup's steps while they are being changed.
    pub snap_draft: Option<SnapSteps>,
    /// The name field's text.
    pub material_name: String,
    pub export_format: ExportFormat,
    /// Where there are no system file dialogs, the path being typed.
    pub path_prompt: Option<PathPrompt>,
    /// The settings of the next bevel, and of the last one while its panel shows.
    pub bevel: BevelSettings,
    /// Whether the bevel panel is folded down to its title.
    pub bevel_folded: bool,
    /// Whether the bevel panel's less common options show.
    pub bevel_more: bool,
    /// The custom profile point being dragged, if any.
    pub profile_drag: Option<usize>,
    /// A modifier's new settings, for [`Action::ChangeModifier`].
    pub modifier_edit: Option<(ModifierId, Modifier)>,
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
            props_content: f32::MAX,
            materials_content: f32::MAX,
            dock: Dock::default(),
            search: None,
            active_material: None,
            material_draft: None,
            snap_draft: None,
            material_name: String::new(),
            export_format: ExportFormat::Obj,
            path_prompt: None,
            bevel: BevelSettings {
                segments: 1,
                ..BevelSettings::default()
            },
            bevel_folded: false,
            bevel_more: false,
            profile_drag: None,
            modifier_edit: None,
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
/// One line of the Materials panel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MaterialRow {
    pub id: MaterialId,
    pub material: Material,
    /// Faces using it, and how many of them are selected.
    pub faces: usize,
    pub selected: usize,
}

pub struct PanelInfo<'a> {
    /// The project's materials, in order.
    pub materials: Vec<MaterialRow>,
    /// Faces with no material, and how many of them are selected.
    pub plain_faces: (usize, usize),
    pub triangles: usize,
    /// How far Ctrl-held moves, turns and scales step.
    pub snap: SnapSteps,
    /// The triangle limit from the settings, or `None` when it is off.
    pub triangle_limit: Option<usize>,
    pub vertices: usize,
    pub faces: usize,
    pub open_edges: usize,
    pub warnings: &'a [String],
    pub target: Target,
    pub mode: ModeChoice,
    pub selected_vertices: usize,
    pub transform_help: Option<(Kind, Option<usize>)>,
    /// A bevel waiting for the left button to be held and dragged.
    pub transform_waiting: bool,
    /// A drag that ends when the left button comes up.
    pub transform_dragging: bool,
    /// While choosing where to loop cut: how many cuts.
    pub loop_cut: Option<usize>,
    /// While bevelling: the width, as its type measures it, the number of segments,
    /// and the type.
    pub bevel: Option<(f32, usize, WidthType)>,
    pub undo_label: Option<&'static str>,
    pub redo_label: Option<&'static str>,
    pub export_folder: &'a str,
    pub view: ViewState,
    /// The open file's name, or "Untitled".
    pub document: &'a str,
    pub unsaved: bool,
    /// A question about unsaved changes, while one is waiting for an answer.
    pub confirm: Option<&'a str>,
    /// The handle tool that is on, if any.
    pub gizmo_tool: Option<Kind>,
    /// Whether the last bevel can still be changed with the bevel panel.
    pub bevel_panel: bool,
    /// While drawing a knife path: whether it cuts through to hidden faces.
    pub knife: Option<bool>,
    /// While dragging a bisect line.
    pub bisect: bool,
    /// View mode: lines and points hidden.
    pub view_only: bool,
    /// How many parts have something selected.
    pub parts_selected: usize,
    /// The modifiers of the one selected part, in the order they run.
    pub modifiers: Vec<(ModifierId, Modifier)>,
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
    /// Bevels the selected vertices instead of edges.
    BevelVertices,
    /// Redoes the last bevel with the settings in `PanelState::bevel`.
    AdjustBevel,
    /// Turns the handles for moving, rotating or scaling on or off.
    GizmoTool(Kind),
    /// Starts drawing a knife path.
    Knife,
    /// Starts dragging a line to cut the selected faces along.
    Bisect,
    /// Joins the selected parts so they act as one.
    Join,
    /// Undoes joining.
    Separate,
    /// Union, subtract or intersect of the selected parts.
    Boolean(BooleanOp),
    /// Switches between Edit mode and View mode, which hides lines and points.
    ToggleView,
    /// Opens feature search (Ctrl+F).
    Search,
    /// A new material, made the active one.
    AddMaterial,
    RemoveMaterial(MaterialId),
    /// Shows `PanelState::material_draft` without saving it, while a slider or the color
    /// picker is dragged.
    PreviewMaterial,
    /// Saves `PanelState::snap_draft` as the snapping steps.
    ChangeSnap,
    /// Saves `PanelState::material_draft` as an undo step.
    ChangeMaterial,
    /// Gives the selected faces a material, or the default look.
    AssignMaterial(Option<MaterialId>),
    /// Selects (true) or deselects the faces with a material, or with none.
    SelectMaterial(Option<MaterialId>, bool),
    /// Back to the start window, asking about unsaved changes first.
    StartWindow,
    /// Opens the settings window.
    Settings,
    /// Shading for the selected faces: 0 flat, otherwise smooth up to that angle.
    Shade(u8),
    /// Marks the selected edges sharp, or clears them.
    MarkSharp(bool),
    /// Adds a modifier to the selected part.
    AddModifier(ModifierKind),
    /// Sets the modifier in `PanelState::modifier_edit`.
    ChangeModifier,
    RemoveModifier(ModifierId),
    /// Makes the selected part's modifiers real geometry.
    ApplyModifiers,
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

/// The kinds of modifier there are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModifierKind {
    Mirror,
    Array,
}

/// The interface's result for one frame.
pub struct UiOutput {
    pub action: Option<Action>,
    /// Screen areas the interface covers, as x, y, width and height. Clicks there do
    /// not reach the viewport.
    pub rects: Vec<[f32; 4]>,
}

/// Where a side panel is: in the column on the right, or floating where it was put.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Place {
    Docked,
    Floating { pos: [f32; 2], size: [f32; 2] },
}

/// The two side panels, Add Shapes (0) and Properties (1), as Blender's sidebars:
/// docked in a column on the right, stacked, or floating. Drag a panel's title to take
/// it out; drop it on the right edge to put it back.
#[derive(Debug, Clone, PartialEq)]
pub struct Dock {
    /// The column's width.
    pub width: f32,
    /// Each docked panel's height as dragged; `None` fits it to its contents. The
    /// lowest docked panel takes what is left.
    pub heights: [Option<f32>; PANELS],
    pub places: [Place; PANELS],
    /// The panel whose title is being dragged, and where in it the mouse took hold.
    pub dragging: Option<(usize, [f32; 2])>,
}

impl Default for Dock {
    fn default() -> Self {
        Self {
            width: RIGHT_PANEL,
            heights: [None; PANELS],
            places: [Place::Docked; PANELS],
            dragging: None,
        }
    }
}

/// The side panels: Add Shapes (0), Properties (1) and Materials (2).
pub const PANELS: usize = 3;
pub const PANEL_NAMES: [&str; PANELS] = ["Add Shapes", "Properties", "Materials"];
/// Top to bottom when docked.
const DOCK_ORDER: [usize; PANELS] = [0, 2, 1];
const NARROWEST_DOCK: f32 = 240.0;
const WIDEST_DOCK: f32 = 640.0;
/// How far a title must be dragged before its panel comes out of the column.
const UNDOCK_DRAG: f32 = 14.0;
/// Dropping a panel this close to the window's right edge docks it.
const DOCK_REACH: f32 = 60.0;
const PANEL_HEADER: f32 = 26.0;

impl Dock {
    pub fn any_docked(&self) -> bool {
        self.places.contains(&Place::Docked)
    }

    /// The width the docked column takes from the right of the window.
    pub fn reserved(&self) -> f32 {
        if self.any_docked() {
            self.width + MARGIN * 2.0
        } else {
            MARGIN
        }
    }
}

/// The part of the window the 3D view shows through, between the panels.
pub fn viewport_area(display: [f32; 2], right_reserved: f32) -> [f32; 4] {
    let left = MARGIN * 2.0 + tool_button_size(display[1]) + 16.0;
    let right = display[0] - right_reserved;
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
                ui.text_colored(color::MUTED(), key);
            }
        });
    }
}

/// A square button showing an icon, lit when `active`.
fn icon_button(ui: &Ui, id: &str, glyph: char, active: bool, enabled: bool, size: f32) -> bool {
    let colors = active.then(|| {
        [
            ui.push_style_color(StyleColor::Button, color::ACCENT_SOFT()),
            ui.push_style_color(StyleColor::ButtonHovered, with_alpha(color::ACCENT(), 0.32)),
            ui.push_style_color(StyleColor::Text, color::ACCENT()),
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
            color::FRAME(),
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
                ui.push_style_color(StyleColor::Button, color::ACCENT_SOFT()),
                ui.push_style_color(StyleColor::ButtonHovered, with_alpha(color::ACCENT(), 0.30)),
                ui.push_style_color(StyleColor::Text, color::ACCENT()),
            ]
        } else {
            vec![
                ui.push_style_color(StyleColor::Button, [0.0; 4]),
                ui.push_style_color(StyleColor::Text, color::MUTED()),
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
    ui.text_colored(color::FAINT(), text);
    drop(font);
}

/// A muted label on the left, and the widget filling the rest of the row.
fn row(ui: &Ui, label: &str) {
    ui.align_text_to_frame_padding();
    ui.text_colored(color::MUTED(), label);
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
        ui.text_colored(color::MUTED(), text);
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
        let bg = ui.push_style_color(StyleColor::WindowBg, color::BAR());
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

    // The two side panels: Add Shapes and Properties, docked or floating.
    side_panels(ui, state, info, fonts, &mut action, &mut rects, display);

    // View gizmo, without a background.
    {
        let padding = ui.push_style_var(StyleVar::WindowPadding([0.0, 0.0]));
        let pos = [
            display[0] - state.dock.reserved() - MARGIN - GIZMO,
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
    search_window(ui, state, fonts, &mut action, &mut rects, display);
    viewport_overlays(ui, info, display, state.dock.reserved());
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
            color::ACCENT(),
        )
        .filled(true)
        .rounding(5.0)
        .build();
    ui.set_cursor_screen_pos([start[0] + 28.0, start[1] + 7.0]);
    let font = ui.push_font(fonts.tool);
    ui.text("Mixer");
    drop(font);
    ui.same_line_with_spacing(0.0, 16.0);
    ui.set_cursor_screen_pos([ui.cursor_screen_pos()[0], start[1] + 2.0]);
    file_menu(ui, info, action, height);
    ui.same_line_with_spacing(0.0, 10.0);
    ui.set_cursor_screen_pos([ui.cursor_screen_pos()[0], start[1] + 9.0]);
    // On a narrow window, or with a big interface size, the bar gets shorter: a shorter
    // name, and the mode buttons without their words.
    let narrow = ui.window_size()[0] < 1240.0;
    let longest = if narrow { 12 } else { 22 };
    // The file's name, shortened if long, with a dot while there are unsaved changes.
    let mut name: String = info.document.chars().take(longest).collect();
    if info.document.chars().count() > longest {
        name.push('\u{2026}');
    }
    ui.text_colored(color::MUTED(), &name);
    tooltip(ui, info.document, "");
    if info.unsaved {
        ui.same_line_with_spacing(0.0, 6.0);
        ui.text_colored(color::ACCENT(), "\u{2022}");
        tooltip(ui, "Unsaved changes", "Ctrl+S");
    }
    ui.same_line_with_spacing(0.0, 24.0);
    ui.set_cursor_screen_pos([ui.cursor_screen_pos()[0], start[1]]);

    // One button that shows the current look and switches to the other.
    let look = if info.view_only {
        [(
            icon::VIEW,
            "View",
            true,
            "View mode: just the shape. Click for Edit mode (Tab)",
        )]
    } else {
        [(
            icon::EDIT,
            "Edit",
            true,
            "Edit mode: lines, points and selection. Click for View mode (Tab)",
        )]
    };
    if segmented(ui, "look", &look, height).is_some() {
        *action = Some(Action::ToggleView);
    }
    ui.same_line_with_spacing(0.0, 12.0);

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
        .map(|(glyph, label, mode, tip)| {
            let label = if narrow { "" } else { *label };
            (*glyph, label, info.mode == *mode, *tip)
        })
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

    ui.same_line_with_spacing(0.0, 16.0);
    if icon_button(ui, "snap", icon::SNAP, false, true, height + 4.0) {
        ui.open_popup("##snap_popup");
    }
    tooltip(ui, "Snapping steps", "Hold Ctrl");
    snap_popup(ui, state, info, fonts, action);

    // Settings, shortcuts, then Export, at the right edge. On a narrow window, or with a
    // big interface size, the two small buttons give way to the ones before them.
    // Settings is in the File menu too.
    let width = 112.0;
    let used = ui.item_rect_max()[0] - ui.window_pos()[0] + 8.0;
    let settings_x = ui.window_size()[0] - width - 12.0 - 2.0 * (height + 4.0) - 12.0;
    let help_x = ui.window_size()[0] - width - 12.0 - (height + 4.0) - 8.0;
    if used <= settings_x {
        ui.same_line_with_pos(settings_x);
        if icon_button(ui, "settings", icon::SETTINGS, false, true, height + 4.0) {
            *action = Some(Action::Settings);
        }
        tooltip(ui, "Settings", "Ctrl+,");
    }
    if used <= help_x {
        ui.same_line_with_pos(help_x);
        if icon_button(ui, "help", icon::KEYBOARD, false, true, height + 4.0) {
            ui.open_popup("##help_popup");
        }
        tooltip(ui, "Keyboard and mouse", "");
    }
    help_popup(ui, fonts);
    ui.same_line_with_pos(ui.window_size()[0] - width - 12.0);
    let colors = [
        ui.push_style_color(StyleColor::Button, color::ACCENT()),
        ui.push_style_color(StyleColor::ButtonHovered, color::ACCENT_HOVER()),
        ui.push_style_color(StyleColor::ButtonActive, color::ACCENT_HOVER()),
        ui.push_style_color(StyleColor::Text, color::ON_ACCENT()),
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
    tooltip(ui, "Export as OBJ or glTF", "Ctrl+E");
    export_popup(ui, state, info, fonts, action);
}

/// Blender's snap increments: how far each step goes while Ctrl is held.
fn snap_popup(
    ui: &Ui,
    state: &mut PanelState,
    info: &PanelInfo,
    fonts: &Fonts,
    action: &mut Option<Action>,
) {
    let padding = ui.push_style_var(StyleVar::WindowPadding([16.0, 14.0]));
    if let Some(_popup) = ui.begin_popup("##snap_popup") {
        ui.dummy([300.0, 0.0]);
        heading(ui, fonts, "SNAPPING");
        ui.text_colored(color::MUTED(), "Hold Ctrl while moving, rotating or");
        ui.text_colored(color::MUTED(), "scaling to go in these steps.");
        ui.dummy([0.0, 4.0]);
        let mut steps = state.snap_draft.unwrap_or(info.snap);
        let mut changed = false;
        let mut done = false;
        let fields: [(&str, &str, &str, f32, &mut f32); 3] = [
            ("Move", "##snap_move", "%.3f", 0.01, &mut steps.move_by),
            (
                "Rotate",
                "##snap_rotate",
                "%.1f°",
                0.2,
                &mut steps.rotate_by,
            ),
            ("Scale", "##snap_scale", "%.3f", 0.005, &mut steps.scale_by),
        ];
        for (i, (label, id, format, speed, value)) in fields.into_iter().enumerate() {
            row(ui, label);
            let (low, high) = SNAP_LIMITS[i];
            changed |= Drag::new(id)
                .range(low, high)
                .speed(speed)
                .display_format(format)
                .flags(SliderFlags::ALWAYS_CLAMP)
                .build(ui, value);
            done |= ui.is_item_deactivated_after_edit();
            tooltip(ui, "Drag sideways, or double-click to type", "");
        }
        ui.dummy([0.0, 4.0]);
        if ui.button_with_size("Reset to 1, 15° and 0.1", [-1.0, 0.0]) {
            steps = SnapSteps::default();
            changed = true;
            done = true;
        }
        if changed {
            state.snap_draft = Some(steps);
        }
        if done {
            *action = Some(Action::ChangeSnap);
        }
        // Esc, or starting to move something, puts it away.
        if info.transform_help.is_some()
            || (ui.is_key_pressed(imgui::Key::Escape) && !ui.is_any_item_active())
        {
            ui.close_current_popup();
        }
    }
    drop(padding);
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
        heading(ui, fonts, "EXPORT");
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
                "Each separate part becomes its own object in the file.",
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
        ui.text_colored(
            color::FAINT(),
            "At scale 1, sizes are exported as they are.",
        );
        ui.spacing();
        triangle_budget(ui, info.triangles, info.triangle_limit);
        for warning in info.warnings {
            note(ui, icon::WARNING, color::WARNING(), warning);
        }
        let colors = [
            ui.push_style_color(StyleColor::Button, color::ACCENT()),
            ui.push_style_color(StyleColor::ButtonHovered, color::ACCENT_HOVER()),
            ui.push_style_color(StyleColor::ButtonActive, color::ACCENT_HOVER()),
            ui.push_style_color(StyleColor::Text, color::ON_ACCENT()),
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
            ui.text_colored(
                color::ACCENT(),
                format!("{}  {}", icon::CHECK, state.message),
            );
            #[cfg(windows)]
            if ui.button_with_size(format!("{}  Open folder##open", icon::FOLDER), [-1.0, 30.0]) {
                *action = Some(Action::OpenExportFolder);
            }
        }
        let wrap = ui.push_text_wrap_pos_with_pos(ui.cursor_pos()[0] + 300.0);
        ui.text_colored(color::FAINT(), format!("Saved in {}", info.export_folder));
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
            (icon::MENU, "Start window\u{2026}", "", Action::StartWindow),
            (
                icon::MENU,
                "Search features\u{2026}",
                "Ctrl+F",
                Action::Search,
            ),
            (
                icon::SETTINGS,
                "Settings\u{2026}",
                "Ctrl+,",
                Action::Settings,
            ),
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
        ui.text_colored(
            color::FAINT(),
            "Open reads .mix projects, older .lem ones, and .obj files.",
        );
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
                    ("Handles to drag", "Move, Rotate, Scale buttons"),
                    ("Slide edges", "G G"),
                    ("Extrude, inset", "E, I"),
                    ("Loop cut", "Ctrl+R"),
                    ("Knife", "K, then click, Enter"),
                    ("Join, separate shapes", "Ctrl+J, P"),
                    ("Mirror, array", "4, click, Modifiers"),
                    ("Edit or View mode", "Tab"),
                    ("Bevel edges", "Ctrl+B"),
                    ("Bevel vertices", "Ctrl+Shift+B"),
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
                    ("Settings", "Ctrl+comma"),
                    ("Search features", "Ctrl+F"),
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
                        ui.text_colored(color::MUTED(), what);
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
                ui.text_colored(color::MUTED(), "Path to the file");
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

fn triangle_budget(ui: &Ui, triangles: usize, limit: Option<usize>) {
    ui.text_colored(color::MUTED(), "Triangles");
    ui.same_line();
    let Some(limit) = limit else {
        let text = triangles.to_string();
        let width = ui.calc_text_size(&text)[0];
        ui.same_line_with_pos(
            ui.cursor_pos()[0].max(ui.content_region_avail()[0] + ui.cursor_pos()[0] - width),
        );
        ui.text(text);
        return;
    };
    let fraction = triangles as f32 / limit.max(1) as f32;
    let text = format!("{triangles} / {limit}");
    let width = ui.calc_text_size(&text)[0];
    ui.same_line_with_pos(
        ui.cursor_pos()[0].max(ui.content_region_avail()[0] + ui.cursor_pos()[0] - width),
    );
    let over = triangles > limit;
    ui.text_colored(if over { color::DANGER() } else { color::TEXT() }, text);
    let bar = ui.push_style_color(
        StyleColor::PlotHistogram,
        if over {
            color::DANGER()
        } else {
            color::ACCENT()
        },
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
    let size = tool_button_size(ui.io().display_size[1]);
    let any = info.selected_vertices > 0;
    let active = info.transform_help.map(|(kind, _)| kind);
    let font = ui.push_font(fonts.tool);
    let groups: [&[Tool]; 5] = [
        &[
            (
                "move",
                icon::MOVE,
                Action::GizmoTool(Kind::Move),
                "Move tool: drag the arrows",
                "G moves with the mouse",
            ),
            (
                "rotate",
                icon::ROTATE,
                Action::GizmoTool(Kind::Rotate),
                "Rotate tool: drag the rings",
                "R rotates with the mouse",
            ),
            (
                "scale",
                icon::SCALE,
                Action::GizmoTool(Kind::Scale),
                "Scale tool: drag the handles",
                "S scales with the mouse",
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
                "knife",
                icon::KNIFE,
                Action::Knife,
                "Knife: click points across faces, then Enter",
                "K",
            ),
            (
                "bisect",
                icon::BISECT,
                Action::Bisect,
                "Bisect: drag a line to cut the selected faces straight through",
                "",
            ),
            (
                "bevel",
                icon::BEVEL,
                Action::Bevel,
                "Bevel: drag for width, scroll for segments",
                "Ctrl+B, vertices Ctrl+Shift+B",
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
                    [p[0] + size - 8.0, p[1]],
                    color::BORDER(),
                )
                .build();
            ui.dummy([0.0, 4.0]);
        }
        for (id, glyph, choice, name, key) in group.iter() {
            let lit = matches!(choice, Action::Transform(kind) if Some(*kind) == active)
                || matches!(choice, Action::GizmoTool(kind)
                    if Some(*kind) == active || Some(*kind) == info.gizmo_tool)
                || (*choice == Action::Inset && active == Some(Kind::Inset))
                || (*choice == Action::LoopCut && info.loop_cut.is_some())
                || (*choice == Action::Knife && info.knife.is_some())
                || (*choice == Action::Bisect && info.bisect)
                || (*choice == Action::Bevel && active == Some(Kind::Bevel));
            // A loop cut picks its own edge, so it needs only something to cut. The
            // handle tools can be turned on before anything is selected.
            let enabled = if matches!(choice, Action::LoopCut | Action::Knife) {
                info.vertices > 0
            } else if matches!(choice, Action::GizmoTool(_)) {
                true
            } else {
                any
            };
            if icon_button(ui, id, *glyph, lit, enabled, size) {
                *action = Some(*choice);
            }
            tooltip(ui, name, key);
        }
        if g == 1 {
            if icon_button(ui, "mirror", icon::MIRROR, false, any, size) {
                ui.open_popup("##mirror_popup");
            }
            tooltip(ui, "Mirror", "");
        }
    }
    drop(font);
    if let Some(_popup) = ui.begin_popup("##mirror_popup") {
        ui.text_colored(color::MUTED(), "Mirror across");
        for (axis, name) in ["X", "Y", "Z"].iter().enumerate() {
            let text = ui.push_style_color(StyleColor::Text, color::AXIS()[axis]);
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

fn tools_panel(
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
            color::FRAME_ACTIVE()
        } else if hovered {
            color::FRAME_HOVER()
        } else {
            color::FRAME()
        };
        list.add_rect(start, [start[0] + tile[0], start[1] + tile[1]], fill)
            .filled(true)
            .rounding(8.0)
            .build();
        let font = ui.push_font(fonts.tool);
        let glyph = shape.icon().to_string();
        let size = ui.calc_text_size(&glyph);
        let tint = if hovered {
            color::ACCENT()
        } else {
            color::TEXT()
        };
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
            color::MUTED(),
            shape.name(),
        );
        drop(font);
        tooltip(ui, shape.edit_label(), "");
        if i % 4 == 3 {
            ui.dummy([0.0, 0.0]);
        }
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
    // Part mode works on whole shapes, so it shows ways to combine them instead of
    // edge loops. That keeps the panel short enough for a small screen.
    if info.mode != ModeChoice::Part {
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
    }
    let disabled = ui.begin_disabled(info.selected_vertices == 0);
    if ui.button_with_size(
        format!("{}  Merge close vertices##clean", icon::MERGE_CLOSE),
        [-1.0, 30.0],
    ) {
        *action = Some(Action::MergeByDistance);
    }
    disabled.end();
    button_row(
        ui,
        action,
        &[
            (
                "Flat##shade",
                Action::Shade(0),
                "Light each face on its own, showing every edge",
            ),
            (
                "Smooth##shade",
                Action::Shade(180),
                "Light the selected faces as one smooth surface",
            ),
            (
                "Auto smooth##shade",
                Action::Shade(AUTO_SMOOTH_ANGLE),
                "Smooth where faces meet gently (under 30\u{b0}), keep sharp corners sharp. Good after a bevel.",
            ),
        ],
        info.selected_vertices > 0,
    );
    if info.mode == ModeChoice::Edge {
        button_row(
            ui,
            action,
            &[
                (
                    "Mark sharp##sharp",
                    Action::MarkSharp(true),
                    "Smooth shading stops at the selected edges",
                ),
                (
                    "Clear sharp##sharp",
                    Action::MarkSharp(false),
                    "Smooth shading crosses the selected edges again",
                ),
            ],
            info.selected_vertices > 0,
        );
    }
    if info.mode != ModeChoice::Part {
        return;
    }

    ui.dummy([0.0, 6.0]);
    heading(ui, fonts, "COMBINE SHAPES");
    let two = info.parts_selected >= 2;
    let label = |glyph: char, name: &str| format!("{glyph}  {name}##combine");
    let (join, separate) = (label(icon::JOIN, "Join"), label(icon::SEPARATE, "Separate"));
    button_row(
        ui,
        action,
        &[
            (
                join.as_str(),
                Action::Join,
                "Keep the shapes as they are, but select, move and export them as one (Ctrl+J)",
            ),
            (
                separate.as_str(),
                Action::Separate,
                "Undo Join: each connected piece is its own part again (P)",
            ),
        ],
        info.parts_selected >= 1,
    );
    let (union, subtract, intersect) = (
        label(icon::UNION, "Union"),
        label(icon::SUBTRACT, "Subtract"),
        label(icon::INTERSECT, "Intersect"),
    );
    button_row(
        ui,
        action,
        &[
            (
                union.as_str(),
                Action::Boolean(BooleanOp::Union),
                "Melt the shapes into one solid",
            ),
            (
                subtract.as_str(),
                Action::Boolean(BooleanOp::Subtract),
                "Cut the shape you clicked last out of the others",
            ),
            (
                intersect.as_str(),
                Action::Boolean(BooleanOp::Intersect),
                "Keep only where the shapes overlap",
            ),
        ],
        two,
    );
    if !two {
        ui.text_colored(color::FAINT(), "Shift+click two or more shapes.");
    }
    modifiers_section(ui, state, info, fonts, action);
}

/// Mirror and array on the selected part: buttons to add them, and each one's settings.
fn modifiers_section(
    ui: &Ui,
    state: &mut PanelState,
    info: &PanelInfo,
    fonts: &Fonts,
    action: &mut Option<Action>,
) {
    // Only for one shape, which also keeps the panel short.
    if info.parts_selected != 1 {
        return;
    }
    ui.dummy([0.0, 6.0]);
    heading(ui, fonts, "MODIFIERS");
    let mirror = format!("{}  Mirror##add_modifier", icon::MIRROR);
    let array = format!("{}  Array##add_modifier", icon::DUPLICATE);
    button_row(
        ui,
        action,
        &[
            (
                mirror.as_str(),
                Action::AddModifier(ModifierKind::Mirror),
                "Mirror the shape across the middle of the world. Edit one side, the other follows.",
            ),
            (
                array.as_str(),
                Action::AddModifier(ModifierKind::Array),
                "Repeat the shape in a row. Edit one, the copies follow.",
            ),
        ],
        true,
    );
    for (i, &(id, modifier)) in info.modifiers.iter().enumerate() {
        let _id = ui.push_id_usize(i);
        ui.dummy([0.0, 2.0]);
        let name = match modifier {
            Modifier::Mirror { .. } => "Mirror",
            Modifier::Array { .. } => "Array",
        };
        ui.align_text_to_frame_padding();
        ui.text(format!("{}. {name}", i + 1));
        let remove = "Remove##modifier";
        let width = ui.calc_text_size(remove.split("##").next().unwrap_or(""))[0] + 16.0;
        ui.same_line_with_pos(ui.content_region_max()[0] - width);
        if ui.button_with_size(remove, [width, 0.0]) {
            *action = Some(Action::RemoveModifier(id));
        }
        tooltip(
            ui,
            "Delete this modifier. The shape goes back to how it is without it.",
            "",
        );
        let mut edited = modifier;
        match &mut edited {
            Modifier::Mirror { axes, merge } => {
                row(ui, "Axis");
                for (k, letter) in ["X", "Y", "Z"].into_iter().enumerate() {
                    if k > 0 {
                        ui.same_line();
                    }
                    ui.checkbox(format!("{letter}##mirror_axis"), &mut axes[k]);
                }
                tooltip(ui, "Which way to mirror. Red X, green Y, blue Z.", "");
                merge_row(
                    ui,
                    merge,
                    "Weld the middle",
                    "Join points on the middle so the halves become one closed shape",
                );
            }
            Modifier::Array {
                count,
                offset,
                merge,
            } => {
                row(ui, "Count");
                let mut n = *count as i32;
                if ui
                    .slider_config("##array_count", 1, MAX_ARRAY_COUNT as i32)
                    .flags(SliderFlags::ALWAYS_CLAMP)
                    .build(&mut n)
                {
                    *count = n.clamp(1, MAX_ARRAY_COUNT as i32) as u32;
                }
                tooltip(ui, "How many there are, counting the original", "");
                row(ui, "Offset");
                Drag::new("##array_offset")
                    .range(-100.0, 100.0)
                    .speed(0.01)
                    .display_format("%.2f")
                    .flags(SliderFlags::ALWAYS_CLAMP)
                    .build_array(ui, offset);
                tooltip(
                    ui,
                    "How far each copy moves along X, Y and Z, in shape sizes. 1 puts them side by side.",
                    "",
                );
                merge_row(
                    ui,
                    merge,
                    "Weld touching",
                    "Join copies where they touch, so they become one shape",
                );
            }
        }
        if edited != modifier {
            state.modifier_edit = Some((id, edited));
            *action = Some(Action::ChangeModifier);
        }
    }
    if !info.modifiers.is_empty() {
        ui.dummy([0.0, 2.0]);
        button_row(
            ui,
            action,
            &[(
                "Apply##modifiers",
                Action::ApplyModifiers,
                "Turn the copies into real shape that you can edit point by point. The modifiers go away.",
            )],
            true,
        );
    }
}

/// A "Merge" checkbox row for a modifier's weld distance.
fn merge_row(ui: &Ui, merge: &mut Option<f32>, label: &str, help: &str) {
    ui.align_text_to_frame_padding();
    ui.text_colored(color::MUTED(), "Merge");
    ui.same_line_with_pos(LABEL_COLUMN);
    let mut on = merge.is_some();
    if ui.checkbox(format!("{label}##modifier_merge"), &mut on) {
        *merge = on.then_some(DEFAULT_MERGE_DISTANCE);
    }
    tooltip(ui, help, "");
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
        ui.text_colored(color::FAINT(), hint);
    } else {
        ui.text_colored(color::MUTED(), &state.message);
    }
    let mut parts: Vec<(String, [f32; 4])> = Vec::new();
    if info.open_edges > 0 {
        parts.push((
            format!("{}  {} open edges", icon::WARNING, info.open_edges),
            color::WARNING(),
        ));
    }
    parts.push((
        format!(
            "{} of {} vertices selected",
            info.selected_vertices, info.vertices
        ),
        color::MUTED(),
    ));
    parts.push((format!("Faces {}", info.faces), color::MUTED()));
    let over = info
        .triangle_limit
        .is_some_and(|limit| info.triangles > limit);
    parts.push((
        format!("Triangles {}", info.triangles),
        if over {
            color::DANGER()
        } else {
            color::MUTED()
        },
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
        let tint = color::AXIS()[*axis];
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

fn viewport_overlays(ui: &Ui, info: &PanelInfo, display: [f32; 2], reserved: f32) {
    let area = viewport_area(display, reserved);
    let list = ui.get_background_draw_list();
    let center_x = area[0] + area[2] / 2.0;

    let mut parts: Vec<(String, [f32; 4])> = Vec::new();
    if let Some(cuts) = info.loop_cut {
        parts.push(("Loop cut".to_string(), color::ACCENT()));
        let plural = if cuts == 1 { "cut" } else { "cuts" };
        parts.push((format!("{cuts} {plural}"), color::TEXT()));
        parts.push(("Scroll or +/- for more".to_string(), color::FAINT()));
        parts.push(("Click an edge to cut".to_string(), color::FAINT()));
        parts.push(("Esc cancel".to_string(), color::FAINT()));
    }
    if let Some(through) = info.knife {
        parts.push(("Knife".to_string(), color::ACCENT()));
        parts.push(("Click to add points".to_string(), color::FAINT()));
        parts.push(("Enter cut".to_string(), color::FAINT()));
        parts.push(("Backspace undo point".to_string(), color::FAINT()));
        let z = if through {
            "Z: cuts hidden faces too"
        } else {
            "Z: front faces only"
        };
        parts.push((z.to_string(), color::TEXT()));
        parts.push(("Esc cancel".to_string(), color::FAINT()));
    }
    if info.bisect {
        parts.push(("Bisect".to_string(), color::ACCENT()));
        parts.push((
            "Drag a line across the selection".to_string(),
            color::FAINT(),
        ));
        parts.push(("Esc cancel".to_string(), color::FAINT()));
    }
    if info.view_only && parts.is_empty() {
        parts.push(("View mode".to_string(), color::ACCENT()));
        parts.push(("Tab to edit".to_string(), color::FAINT()));
    }
    if let Some((kind, axis)) = info.transform_help {
        parts.push((kind.name().to_string(), color::ACCENT()));
        if let Some((width, segments, width_type)) = info.bevel {
            let plural = if segments == 1 { "segment" } else { "segments" };
            let shown = if width_type == WidthType::Percent {
                format!("{} {width:.1}%", width_type.name())
            } else {
                format!("{} {width:.2}", width_type.name())
            };
            parts.push((shown, color::TEXT()));
            parts.push((format!("{segments} {plural}"), color::TEXT()));
            parts.push(("Scroll for more".to_string(), color::FAINT()));
        }
        if !matches!(kind, Kind::Inset | Kind::Slide | Kind::Bevel) {
            for (i, name) in ["X", "Y", "Z"].iter().enumerate() {
                let lit = axis == Some(i);
                parts.push((
                    name.to_string(),
                    if lit {
                        color::AXIS()[i]
                    } else {
                        color::MUTED()
                    },
                ));
            }
            parts.push(("lock axis".to_string(), color::FAINT()));
        }
        if info.transform_waiting {
            parts.push(("Hold left click and drag".to_string(), color::TEXT()));
        } else {
            parts.push(("Ctrl snap".to_string(), color::FAINT()));
            parts.push((
                if info.transform_dragging {
                    "Release to confirm"
                } else {
                    "Click confirm"
                }
                .to_string(),
                color::FAINT(),
            ));
        }
        parts.push(("Esc cancel".to_string(), color::FAINT()));
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
            color::OVERLAY(),
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
}

/// Feature search: a field to type in and the matching commands below it. Enter or a
/// click runs one; the arrow keys choose; Esc closes.
fn search_window(
    ui: &Ui,
    state: &mut PanelState,
    fonts: &Fonts,
    action: &mut Option<Action>,
    rects: &mut Vec<[f32; 4]>,
    display: [f32; 2],
) {
    let Some(search) = state.search.as_mut() else {
        return;
    };
    let width = 480.0_f32.min(display[0] - 2.0 * MARGIN);
    let commands = search::commands();
    let found = search::matching(&search.query, &commands);
    search.selected = search.selected.min(found.len().saturating_sub(1));
    let mut close = false;
    let padding = ui.push_style_var(StyleVar::WindowPadding([12.0, 12.0]));
    let built = ui
        .window("##search")
        .position(
            [(display[0] - width) * 0.5, TOP_BAR + 30.0],
            Condition::Always,
        )
        .size([width, 0.0], Condition::Always)
        .flags(
            WindowFlags::NO_TITLE_BAR
                | WindowFlags::NO_RESIZE
                | WindowFlags::NO_MOVE
                | WindowFlags::NO_SAVED_SETTINGS
                | WindowFlags::ALWAYS_AUTO_RESIZE,
        )
        .build(|| {
            if search.focus {
                ui.set_keyboard_focus_here();
                search.focus = false;
            }
            ui.set_next_item_width(-1.0);
            let font = ui.push_font(fonts.heading);
            let typed = ui
                .input_text("##search_text", &mut search.query)
                .hint("Search for a feature\u{2026}")
                .enter_returns_true(true)
                .build();
            drop(font);
            if ui.is_item_edited() {
                search.selected = 0;
            }
            if ui.is_key_pressed(imgui::Key::DownArrow) {
                search.selected = (search.selected + 1).min(found.len().saturating_sub(1));
            }
            if ui.is_key_pressed(imgui::Key::UpArrow) {
                search.selected = search.selected.saturating_sub(1);
            }
            if ui.is_key_pressed(imgui::Key::Escape) {
                close = true;
            }
            if typed && let Some(&i) = found.get(search.selected) {
                *action = Some(commands[i].action);
                close = true;
            }
            ui.dummy([0.0, 4.0]);
            if found.is_empty() {
                ui.text_colored(color::FAINT(), "Nothing matches. Try another word.");
            }
            // At most twelve at a time, scrolling with the chosen one.
            let shown = 12usize;
            let first = search.selected.saturating_sub(shown - 1);
            for (row, &i) in found.iter().enumerate().skip(first).take(shown) {
                let command = &commands[i];
                let lit = row == search.selected;
                let colors = if lit {
                    vec![ui.push_style_color(StyleColor::Button, color::ACCENT_SOFT())]
                } else {
                    vec![ui.push_style_color(StyleColor::Button, [0.0; 4])]
                };
                let align = ui.push_style_var(StyleVar::ButtonTextAlign([0.0, 0.5]));
                let start = ui.cursor_screen_pos();
                if ui.button_with_size(
                    format!("{}   {}##search{row}", command.icon, command.name),
                    [-1.0, 30.0],
                ) {
                    *action = Some(command.action);
                    close = true;
                }
                if ui.is_item_hovered() && ui.io().mouse_delta != [0.0, 0.0] {
                    search.selected = row;
                }
                drop(align);
                drop(colors);
                if !command.keys.is_empty() {
                    let keys_width = ui.calc_text_size(command.keys)[0];
                    ui.get_window_draw_list().add_text(
                        [
                            start[0] + ui.item_rect_size()[0] - keys_width - 10.0,
                            start[1] + 7.0,
                        ],
                        color::FAINT(),
                        command.keys,
                    );
                }
            }
            if found.len() > shown {
                ui.text_colored(
                    color::FAINT(),
                    format!("{} more; type more of the name.", found.len() - shown),
                );
            }
            let [x, y] = ui.window_pos();
            let [w, h] = ui.window_size();
            [x, y, w, h]
        });
    drop(padding);
    if let Some(rect) = built {
        rects.push(rect);
        // A click outside closes it, as menus do.
        let mouse = ui.io().mouse_pos;
        let inside = mouse[0] >= rect[0]
            && mouse[0] <= rect[0] + rect[2]
            && mouse[1] >= rect[1]
            && mouse[1] <= rect[1] + rect[3];
        if !inside && ui.is_mouse_clicked(imgui::MouseButton::Left) {
            close = true;
        }
    }
    if close {
        state.search = None;
    }
}

/// How tall each docked panel is when it fits its contents. When they do not all fit,
/// the tallest give way first, down to the same height, so that a short panel stays
/// whole and the bevel options in Properties still get their share beside a long
/// Add Shapes panel. Panels whose edge was dragged keep their height.
fn fit_heights(
    docked: &[usize],
    contents: &[f32; PANELS],
    dragged: &[Option<f32>; PANELS],
    room: f32,
) -> [f32; PANELS] {
    let mut heights = [0.0f32; PANELS];
    let gaps = MARGIN * docked.len().saturating_sub(1) as f32;
    let mut left = room - gaps;
    let mut free: Vec<usize> = Vec::new();
    for (k, &panel) in docked.iter().enumerate() {
        match dragged[panel] {
            // The lowest panel always takes what is left, dragged or not.
            Some(h) if k + 1 < docked.len() => {
                heights[panel] = h;
                left -= h;
            }
            _ => free.push(panel),
        }
    }
    let want = |p: usize| contents[p].max(80.0);
    let total: f32 = free.iter().map(|&p| want(p)).sum();
    if total <= left {
        for &p in &free {
            heights[p] = want(p);
        }
        return heights;
    }
    // The cap c where the panels, each the smaller of its contents and c, fill the room.
    let mut sizes: Vec<f32> = free.iter().map(|&p| want(p)).collect();
    sizes.sort_by(f32::total_cmp);
    let mut cap = (left / free.len().max(1) as f32).max(80.0);
    let mut used = 0.0;
    for (i, &size) in sizes.iter().enumerate() {
        let rest = (sizes.len() - i) as f32;
        if used + size * rest >= left {
            cap = ((left - used) / rest).max(80.0);
            break;
        }
        used += size;
    }
    for &p in &free {
        heights[p] = want(p).min(cap);
    }
    heights
}

/// Lays out and draws the two side panels, and lets them be dragged out, resized, and
/// docked again.
#[allow(clippy::too_many_arguments)]
fn side_panels(
    ui: &Ui,
    state: &mut PanelState,
    info: &PanelInfo,
    fonts: &Fonts,
    action: &mut Option<Action>,
    rects: &mut Vec<[f32; 4]>,
    display: [f32; 2],
) {
    let dock = &mut state.dock;
    dock.width = dock.width.clamp(
        NARROWEST_DOCK,
        WIDEST_DOCK.min((display[0] * 0.5).max(NARROWEST_DOCK)),
    );
    let column_x = display[0] - dock.width - MARGIN;
    let top = TOP_BAR + MARGIN;
    let room = (display[1] - TOP_BAR - STATUS_BAR - MARGIN * 2.0).max(80.0);
    // Docked panels stack in DOCK_ORDER. Each fits its contents unless its lower edge
    // was dragged; the lowest takes what is left.
    let contents = [
        state.panel_content,
        state.props_content,
        state.materials_content,
    ];
    let docked: Vec<usize> = DOCK_ORDER
        .into_iter()
        .filter(|&p| dock.places[p] == Place::Docked)
        .collect();
    let fitted = fit_heights(&docked, &contents, &dock.heights, room);
    let mut docked_rects = [[0.0f32; 4]; PANELS];
    let mut y = top;
    let bottom = top + room;
    for (k, &panel) in docked.iter().enumerate() {
        let below = docked.len() - k - 1;
        let h = if docked.len() == 1 {
            contents[panel].min(room)
        } else if below == 0 {
            bottom - y
        } else {
            let most = (bottom - y - below as f32 * (80.0 + MARGIN)).max(80.0);
            dock.heights[panel]
                .unwrap_or(fitted[panel])
                .clamp(80.0_f32.min(most), most)
        };
        docked_rects[panel] = [column_x, y, dock.width, h.max(60.0)];
        y += h + MARGIN;
    }
    let rects_wanted: Vec<[f32; 4]> = (0..PANELS)
        .map(|panel| match dock.places[panel] {
            Place::Docked => docked_rects[panel],
            Place::Floating { pos, size } => [pos[0], pos[1], size[0], size[1]],
        })
        .collect();

    let mouse = ui.io().mouse_pos;
    for panel in 0..PANELS {
        let [x, y, w, h] = rects_wanted[panel];
        let docked = state.dock.places[panel] == Place::Docked;
        let mut flags = WindowFlags::NO_TITLE_BAR
            | WindowFlags::NO_MOVE
            | WindowFlags::NO_COLLAPSE
            | WindowFlags::NO_SAVED_SETTINGS;
        if docked {
            flags |= WindowFlags::NO_RESIZE;
        }
        let padding = ui.push_style_var(StyleVar::WindowPadding([14.0, 10.0]));
        let name = ["##panel", "##properties", "##materials"][panel];
        let built = ui
            .window(name)
            .position([x, y], Condition::Always)
            .size([w, h.max(60.0)], Condition::Always)
            .size_constraints([NARROWEST_DOCK, 80.0], [WIDEST_DOCK, room.max(80.0)])
            .flags(flags)
            .build(|| {
                panel_header(ui, state, fonts, panel);
                // The contents scroll below the title, which stays in reach.
                let header = ui.cursor_pos()[1];
                let mut inner = 0.0;
                ui.child_window(format!("##panel_body{panel}"))
                    .size([0.0, 0.0])
                    .build(|| {
                        match panel {
                            0 => tools_panel(ui, state, info, fonts, action),
                            1 => properties(ui, state, info, fonts, action),
                            _ => materials_panel(ui, state, info, fonts, action),
                        }
                        inner = ui.cursor_pos()[1];
                    });
                // The title, then the contents, plus the bottom padding, minus the
                // spacing the last item left below itself.
                let content = (header + inner + 10.0 - 8.0).max(60.0);
                match panel {
                    0 => state.panel_content = content,
                    1 => state.props_content = content,
                    _ => state.materials_content = content,
                }
                let [px, py] = ui.window_pos();
                let [pw, ph] = ui.window_size();
                [px, py, pw, ph]
            });
        drop(padding);
        if let Some(rect) = built {
            rects.push(rect);
            // A floating panel keeps the size it was resized to.
            if let Place::Floating { size, .. } = &mut state.dock.places[panel] {
                *size = [rect[2], rect[3]];
            }
        }
    }

    // A panel's title being dragged: it follows the mouse; near the right edge it
    // shows where it would dock.
    if let Some((panel, grab)) = state.dock.dragging {
        if ui.is_mouse_down(imgui::MouseButton::Left) {
            let near_edge = mouse[0] > display[0] - DOCK_REACH;
            if let Place::Floating { pos, .. } = &mut state.dock.places[panel] {
                *pos = [
                    (mouse[0] - grab[0]).clamp(0.0, (display[0] - 60.0).max(0.0)),
                    (mouse[1] - grab[1])
                        .clamp(TOP_BAR, (display[1] - STATUS_BAR - 30.0).max(TOP_BAR)),
                ];
            }
            if near_edge {
                let list = ui.get_foreground_draw_list();
                let w = state.dock.width;
                let a = [display[0] - w - MARGIN, top];
                let b = [display[0] - MARGIN, top + room];
                list.add_rect(a, b, with_alpha(color::ACCENT(), 0.18))
                    .filled(true)
                    .rounding(8.0)
                    .build();
                list.add_rect(a, b, color::ACCENT())
                    .rounding(8.0)
                    .thickness(2.0)
                    .build();
            }
        } else {
            if mouse[0] > display[0] - DOCK_REACH {
                state.dock.places[panel] = Place::Docked;
            }
            state.dock.dragging = None;
        }
    }

    // Splitters: the column's left edge sets its width; the gap between two docked
    // panels sets how they share the height.
    let dock = &state.dock;
    if dock.any_docked() && dock.dragging.is_none() {
        let edge = [column_x - 6.0, top, 8.0, room];
        if let Some(dx) = splitter(ui, "##dock_width", edge, true) {
            state.dock.width = (state.dock.width - dx).clamp(NARROWEST_DOCK, WIDEST_DOCK);
        }
        // Below each docked panel but the lowest.
        for pair in docked.windows(2) {
            let [x, y, w, h] = docked_rects[pair[0]];
            let gap = [x, y + h, w, MARGIN];
            if let Some(dy) = splitter(ui, &format!("##dock_split{}", pair[0]), gap, false) {
                state.dock.heights[pair[0]] = Some(h + dy);
            }
        }
    }
}

/// An invisible handle to drag, shown by the mouse cursor. Returns how far it moved.
fn splitter(ui: &Ui, id: &str, rect: [f32; 4], sideways: bool) -> Option<f32> {
    let padding = ui.push_style_var(StyleVar::WindowPadding([0.0, 0.0]));
    let size = ui.push_style_var(StyleVar::WindowMinSize([1.0, 1.0]));
    let mut moved = None;
    ui.window(id)
        .position([rect[0], rect[1]], Condition::Always)
        .size([rect[2], rect[3]], Condition::Always)
        .flags(
            WindowFlags::NO_TITLE_BAR
                | WindowFlags::NO_RESIZE
                | WindowFlags::NO_MOVE
                | WindowFlags::NO_SCROLLBAR
                | WindowFlags::NO_SAVED_SETTINGS
                | WindowFlags::NO_BACKGROUND,
        )
        .build(|| {
            ui.invisible_button("##grip", [rect[2], rect[3]]);
            if ui.is_item_hovered() || ui.is_item_active() {
                ui.set_mouse_cursor(Some(if sideways {
                    imgui::MouseCursor::ResizeEW
                } else {
                    imgui::MouseCursor::ResizeNS
                }));
            }
            if ui.is_item_active() {
                let delta = ui.io().mouse_delta;
                let d = if sideways { delta[0] } else { delta[1] };
                if d != 0.0 {
                    moved = Some(d);
                }
            }
        });
    drop(size);
    drop(padding);
    moved
}

/// A panel's title: drag it to take the panel out or put it back; double-click to
/// switch between docked and floating.
fn panel_header(ui: &Ui, state: &mut PanelState, fonts: &Fonts, panel: usize) {
    let start = ui.cursor_screen_pos();
    let width = ui.content_region_avail()[0];
    ui.invisible_button("##panel_title", [width, PANEL_HEADER]);
    let hovered = ui.is_item_hovered();
    let window_pos = ui.window_pos();
    let display = ui.io().display_size;
    if ui.is_item_active() && state.dock.dragging.is_none() {
        let drag = ui.mouse_drag_delta();
        if drag[0].hypot(drag[1]) > UNDOCK_DRAG {
            let mouse = ui.io().mouse_pos;
            let grab = [mouse[0] - window_pos[0], mouse[1] - window_pos[1]];
            if state.dock.places[panel] == Place::Docked {
                let size = [state.dock.width, ui.window_size()[1].min(display[1] * 0.7)];
                state.dock.places[panel] = Place::Floating {
                    pos: window_pos,
                    size,
                };
            }
            state.dock.dragging = Some((panel, grab));
        }
    }
    if hovered && ui.is_mouse_double_clicked(imgui::MouseButton::Left) {
        state.dock.places[panel] = match state.dock.places[panel] {
            Place::Docked => Place::Floating {
                pos: [display[0] * 0.5 - 160.0, display[1] * 0.25],
                size: [state.dock.width, display[1] * 0.5],
            },
            Place::Floating { .. } => Place::Docked,
        };
    }
    if hovered && state.dock.dragging.is_none() {
        ui.set_mouse_cursor(Some(imgui::MouseCursor::ResizeAll));
        tooltip(
            ui,
            "Drag to move this panel out; drop it on the right edge to dock it. \
             Double-click to switch.",
            "",
        );
    }
    let list = ui.get_window_draw_list();
    // Six dots, a grip, then the title.
    for row in 0..3 {
        for col in 0..2 {
            list.add_circle(
                [
                    start[0] + 4.0 + col as f32 * 5.0,
                    start[1] + 8.0 + row as f32 * 5.0,
                ],
                1.3,
                if hovered {
                    color::TEXT()
                } else {
                    color::FAINT()
                },
            )
            .filled(true)
            .build();
        }
    }
    let font = ui.push_font(fonts.heading);
    list.add_text(
        [start[0] + 18.0, start[1] + 6.0],
        color::TEXT(),
        PANEL_NAMES[panel].to_uppercase(),
    );
    drop(font);
    drop(list);
}

/// The Materials panel, as Blender's material tab: the project's materials, the active
/// one's settings, and buttons to put it on the selected faces.
fn materials_panel(
    ui: &Ui,
    state: &mut PanelState,
    info: &PanelInfo,
    fonts: &Fonts,
    action: &mut Option<Action>,
) {
    // A material removed, or undone away, is no longer active.
    if let Some(id) = state.active_material
        && !info.materials.iter().any(|r| r.id == id)
    {
        state.active_material = None;
    }
    let active = state
        .active_material
        .and_then(|id| info.materials.iter().find(|r| r.id == id).copied());
    // The draft follows the active material unless it is being edited.
    match (&state.material_draft, active) {
        (Some((id, _)), Some(row)) if *id == row.id => {}
        (_, Some(row)) => {
            state.material_draft = Some((row.id, row.material));
            state.material_name = row.material.name().to_string();
        }
        (_, None) => state.material_draft = None,
    }

    // The list: the default look first, then each material.
    let line = |ui: &Ui, label: &str, swatch: [f32; 4], count: (usize, usize), on: bool| -> bool {
        let start = ui.cursor_screen_pos();
        let width = ui.content_region_avail()[0];
        let clicked = ui
            .selectable_config(format!("##{label}"))
            .selected(on)
            .size([width, 26.0])
            .build();
        let list = ui.get_window_draw_list();
        list.add_rect(
            [start[0] + 4.0, start[1] + 5.0],
            [start[0] + 20.0, start[1] + 21.0],
            [swatch[0], swatch[1], swatch[2], 1.0],
        )
        .filled(true)
        .rounding(4.0)
        .build();
        list.add_rect(
            [start[0] + 4.0, start[1] + 5.0],
            [start[0] + 20.0, start[1] + 21.0],
            color::BORDER(),
        )
        .rounding(4.0)
        .build();
        let name = label.split("##").next().unwrap_or(label);
        list.add_text([start[0] + 28.0, start[1] + 5.0], color::TEXT(), name);
        let plural = if count.0 == 1 { "face" } else { "faces" };
        let faces = if count.1 > 0 {
            format!("{} {plural}, {} selected", count.0, count.1)
        } else {
            format!("{} {plural}", count.0)
        };
        let faces_width = ui.calc_text_size(&faces)[0];
        list.add_text(
            [start[0] + width - faces_width - 6.0, start[1] + 5.0],
            color::FAINT(),
            &faces,
        );
        clicked
    };
    let face = crate::palette::get(crate::palette::FACE);
    if line(
        ui,
        "Default look##plain",
        face,
        info.plain_faces,
        active.is_none(),
    ) {
        state.active_material = None;
    }
    tooltip(
        ui,
        "Faces without a material. Its color is the theme's face color.",
        "",
    );
    for row in &info.materials {
        let label = format!("{}##material{:?}", row.material.name(), row.id);
        if line(
            ui,
            &label,
            row.material.color,
            (row.faces, row.selected),
            active.map(|a| a.id) == Some(row.id),
        ) {
            state.active_material = Some(row.id);
        }
    }
    ui.dummy([0.0, 4.0]);
    let half = (ui.content_region_avail()[0] - 8.0) / 2.0;
    if ui.button_with_size("+ New material", [half, 0.0]) {
        *action = Some(Action::AddMaterial);
    }
    tooltip(ui, "Adds a material, a copy of the one chosen above", "");
    ui.same_line_with_spacing(0.0, 8.0);
    let disabled = ui.begin_disabled(active.is_none());
    if ui.button_with_size("Remove", [half, 0.0])
        && let Some(row) = active
    {
        *action = Some(Action::RemoveMaterial(row.id));
    }
    drop(disabled);
    if active.is_some() {
        tooltip(
            ui,
            "Removes the material; its faces get the default look",
            "",
        );
    }

    // The active material's settings.
    if let Some((id, draft)) = state.material_draft.as_mut() {
        let id = *id;
        ui.dummy([0.0, 6.0]);
        heading(ui, fonts, "SETTINGS");
        row(ui, "Name");
        ui.set_next_item_width(-1.0);
        if ui
            .input_text("##material_name", &mut state.material_name)
            .enter_returns_true(true)
            .build()
            || ui.is_item_deactivated_after_edit()
        {
            draft.set_name(&state.material_name);
            state.material_name = draft.name().to_string();
            *action = Some(Action::ChangeMaterial);
        }
        let mut changed = false;
        let mut finished = false;
        row(ui, "Color");
        ui.set_next_item_width(-1.0);
        changed |= ui
            .color_edit4_config("##material_color", &mut draft.color)
            .alpha_bar(true)
            .build();
        finished |= ui.is_item_deactivated_after_edit();
        tooltip(
            ui,
            "The color; its opacity shows in exports, not in Mixer's view",
            "",
        );
        row(ui, "Metal");
        ui.set_next_item_width(-1.0);
        changed |= ui
            .slider_config("##material_metal", 0.0, 1.0)
            .display_format("%.2f")
            .build(&mut draft.metal);
        finished |= ui.is_item_deactivated_after_edit();
        tooltip(ui, "0 for paint, plastic or wood; 1 for bare metal", "");
        row(ui, "Roughness");
        ui.set_next_item_width(-1.0);
        changed |= ui
            .slider_config("##material_rough", 0.0, 1.0)
            .display_format("%.2f")
            .build(&mut draft.roughness);
        finished |= ui.is_item_deactivated_after_edit();
        tooltip(ui, "0 is a mirror finish, 1 is dull", "");
        if finished {
            *action = Some(Action::ChangeMaterial);
        } else if changed && action.is_none() {
            *action = Some(Action::PreviewMaterial);
        }
        let _ = id;
    }

    // Putting it on faces.
    ui.dummy([0.0, 6.0]);
    let chosen = active.map(|r| r.id);
    let selected: usize =
        info.materials.iter().map(|r| r.selected).sum::<usize>() + info.plain_faces.1;
    let third = (ui.content_region_avail()[0] - 16.0) / 3.0;
    let disabled = ui.begin_disabled(selected == 0);
    if ui.button_with_size("Assign", [third, 0.0]) {
        *action = Some(Action::AssignMaterial(chosen));
    }
    drop(disabled);
    tooltip(
        ui,
        if selected == 0 {
            "Select faces first, then give them this material"
        } else {
            "Gives the selected faces this material"
        },
        "",
    );
    ui.same_line_with_spacing(0.0, 8.0);
    if ui.button_with_size("Select", [third, 0.0]) {
        *action = Some(Action::SelectMaterial(chosen, true));
    }
    tooltip(ui, "Selects the faces with this material", "");
    ui.same_line_with_spacing(0.0, 8.0);
    if ui.button_with_size("Deselect", [third, 0.0]) {
        *action = Some(Action::SelectMaterial(chosen, false));
    }
    tooltip(ui, "Deselects the faces with this material", "");
}

/// The Properties panel: what the last tool or the selected shape can still change,
/// as Blender's "Adjust Last Operation" panel and sidebar do.
fn properties(
    ui: &Ui,
    state: &mut PanelState,
    info: &PanelInfo,
    fonts: &Fonts,
    action: &mut Option<Action>,
) {
    if info.bevel_panel {
        bevel_panel(ui, state, info, fonts, action);
        ui.dummy([0.0, 8.0]);
    }
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
        ui.text_colored(color::ACCENT(), label);
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
        ui.text_colored(color::ACCENT(), "LIVE");
        drop(font);
        tooltip(ui, "Changes apply to the selected shape", "");
    }
    match info.target {
        Target::Edited(shape) => note(
            ui,
            icon::INFO,
            color::MUTED(),
            &format!(
                "This {} was reshaped, so these settings are for the next shape.",
                shape.name().to_lowercase()
            ),
        ),
        Target::Several => note(
            ui,
            icon::INFO,
            color::MUTED(),
            "Several shapes are selected. Select one to change it.",
        ),
        _ => {}
    }
    let open = editing.is_some() || state.show_defaults;
    let shows = |setting: Setting| open && editing.is_none_or(|s| s.uses(setting));
    if open {
        shape_settings(ui, state, shows);
    }
}

/// The bevel panel: a title that folds it, then the settings. Any change redoes the bevel.
fn bevel_panel(
    ui: &Ui,
    state: &mut PanelState,
    info: &PanelInfo,
    fonts: &Fonts,
    action: &mut Option<Action>,
) {
    let start = ui.cursor_screen_pos();
    let width = ui.content_region_avail()[0];
    if ui.invisible_button("##bevel_fold", [width, 24.0]) {
        state.bevel_folded = !state.bevel_folded;
    }
    let hovered = ui.is_item_hovered();
    let list = ui.get_window_draw_list();
    // A small arrow pointing down when open and right when folded.
    let tint = if hovered {
        color::TEXT()
    } else {
        color::MUTED()
    };
    let (x, y) = (start[0] + 6.0, start[1] + 12.0);
    let arrow = if state.bevel_folded {
        [[x - 3.0, y - 5.0], [x + 4.0, y], [x - 3.0, y + 5.0]]
    } else {
        [[x - 5.0, y - 3.0], [x + 5.0, y - 3.0], [x, y + 4.0]]
    };
    list.add_triangle(arrow[0], arrow[1], arrow[2], tint)
        .filled(true)
        .build();
    let font = ui.push_font(fonts.heading);
    let size = ui.calc_text_size("Bevel");
    list.add_text(
        [start[0] + 20.0, start[1] + 12.0 - size[1] / 2.0],
        color::TEXT(),
        "Bevel",
    );
    drop(font);
    // Only one window draw list may be held at a time, and the controls below take it.
    drop(list);
    if state.bevel_folded {
        return;
    }
    ui.dummy([0.0, 2.0]);
    let before = state.bevel;
    let settings = &mut state.bevel;

    row(ui, "Affect");
    let items = [
        (
            icon::VERTEX,
            "Vertices",
            settings.vertices,
            "Round off the selected corners",
        ),
        (
            icon::EDGE,
            "Edges",
            !settings.vertices,
            "Round off the selected edges",
        ),
    ];
    if let Some(i) = segmented(ui, "affect", &items, 26.0) {
        settings.vertices = i == 0;
    }
    row(ui, "Width type");
    let names: Vec<&str> = WidthType::ALL.iter().map(|t| t.name()).collect();
    let mut chosen = WidthType::ALL
        .iter()
        .position(|&t| t == settings.width_type)
        .unwrap_or(0);
    if ui.combo_simple_string("##bevel_width_type", &mut chosen, &names) {
        // Keeps the bevel about the same size: percent of the edges, or a distance.
        let was_percent = settings.width_type == WidthType::Percent;
        settings.width_type = WidthType::ALL[chosen];
        let is_percent = settings.width_type == WidthType::Percent;
        if was_percent != is_percent {
            settings.width = if is_percent {
                (settings.width * 100.0).min(50.0)
            } else {
                settings.width / 100.0
            };
        }
    }
    tooltip(
        ui,
        match settings.width_type {
            WidthType::Offset => "From the old edge to each new edge, across the faces",
            WidthType::Width => "Straight across the bevel, between its two new edges",
            WidthType::Depth => "From the old edge straight in to the new face",
            WidthType::Percent => "A percentage of the edges the new points move along",
            WidthType::Absolute => "How far the new points move along the edges next to it",
        },
        "",
    );
    row(ui, settings.width_type.name());
    let percent = settings.width_type == WidthType::Percent;
    Drag::new("##bevel_width")
        .range(0.0, if percent { 100.0 } else { 1000.0 })
        .speed(if percent { 0.2 } else { 0.005 })
        .display_format(if percent { "%.1f%%" } else { "%.3f" })
        .flags(SliderFlags::ALWAYS_CLAMP)
        .build(ui, &mut settings.width);
    tooltip(ui, "Drag sideways, or double-click to type", "");
    row(ui, "Segments");
    let mut segments = settings.segments as i32;
    if ui
        .slider_config("##bevel_segments", 1, MAX_BEVEL_SEGMENTS as i32)
        .flags(SliderFlags::ALWAYS_CLAMP)
        .build(&mut segments)
    {
        settings.segments = segments.clamp(1, MAX_BEVEL_SEGMENTS as i32) as usize;
    }

    // The profile.
    row(ui, "Profile");
    let kinds = [
        (
            icon::BEVEL,
            "Superellipse",
            settings.profile_type == ProfileType::Superellipse,
            "Round, flat or pointed, set by Shape",
        ),
        (
            icon::EDIT,
            "Custom",
            settings.profile_type == ProfileType::Custom,
            "Any shape: pick a preset or drag the points",
        ),
    ];
    if let Some(i) = segmented(ui, "profile_type", &kinds, 26.0) {
        settings.profile_type = if i == 0 {
            ProfileType::Superellipse
        } else {
            ProfileType::Custom
        };
    }
    match settings.profile_type {
        ProfileType::Superellipse => {
            row(ui, "Shape");
            ui.slider_config("##bevel_shape", 0.0, 1.0)
                .display_format("%.2f")
                .flags(SliderFlags::ALWAYS_CLAMP)
                .build(&mut settings.shape);
            tooltip(ui, "0.5 is round, lower is flatter, higher is sharper", "");
        }
        ProfileType::Custom => {
            row(ui, "Preset");
            let names: Vec<&str> = ProfilePreset::ALL.iter().map(|p| p.name()).collect();
            let current = settings.custom.matching_preset();
            let preview = current.map_or("Edited", |p| p.name());
            if let Some(_combo) = ui.begin_combo("##bevel_preset", preview) {
                for (k, preset) in ProfilePreset::ALL.into_iter().enumerate() {
                    if ui
                        .selectable_config(names[k])
                        .selected(current == Some(preset))
                        .build()
                    {
                        let keep = (
                            settings.custom.sample_straight,
                            settings.custom.even_lengths,
                        );
                        settings.custom = CustomProfile::preset(preset);
                        (
                            settings.custom.sample_straight,
                            settings.custom.even_lengths,
                        ) = keep;
                    }
                }
            }
            let segments = settings.segments;
            profile_editor(ui, &mut settings.custom, segments, &mut state.profile_drag);
            let settings = &mut state.bevel;
            ui.checkbox(
                "Sample straight edges##bevel",
                &mut settings.custom.sample_straight,
            );
            tooltip(
                ui,
                "Put segments on straight parts too, not only on curves",
                "",
            );
            ui.checkbox(
                "Sample even lengths##bevel",
                &mut settings.custom.even_lengths,
            );
            tooltip(
                ui,
                "Spread the segments evenly along the profile, even if that skips points",
                "",
            );
        }
    }
    let settings = &mut state.bevel;

    // Less common options, folded away by default to keep the panel short.
    ui.dummy([0.0, 2.0]);
    let label = if state.bevel_more {
        "Fewer options##bevel_more"
    } else {
        "More options##bevel_more"
    };
    if ui.small_button(label) {
        state.bevel_more = !state.bevel_more;
    }
    if state.bevel_more {
        let check = |ui: &Ui, label: &str, name: &str, value: &mut bool, help: &str| {
            ui.align_text_to_frame_padding();
            ui.text_colored(color::MUTED(), label);
            ui.same_line_with_pos(LABEL_COLUMN);
            ui.checkbox(name, value);
            tooltip(ui, help, "");
        };
        check(
            ui,
            "Clamp",
            "Clamp overlap##bevel_clamp",
            &mut settings.clamp,
            "Stop the width where bevels would run into each other",
        );
        check(
            ui,
            "Slide",
            "Loop slide##bevel_slide",
            &mut settings.loop_slide,
            "New points slide along the edges next to the bevel. Off: an even width instead",
        );
        check(
            ui,
            "Shading",
            "Mark sharp##bevel_sharp",
            &mut settings.mark_sharp,
            "The bevel's borders become sharp edges, so smooth shading stops there",
        );
        row(ui, "Miter outer");
        let outer = [Miter::Sharp, Miter::Patch, Miter::Arc];
        let mut chosen = outer
            .iter()
            .position(|&m| m == settings.miter_outer)
            .unwrap_or(0);
        if ui.combo_simple_string("##miter_outer", &mut chosen, &["Sharp", "Patch", "Arc"]) {
            settings.miter_outer = outer[chosen];
        }
        tooltip(
            ui,
            "Where two beveled edges turn a corner: the outside of the turn",
            "",
        );
        row(ui, "Miter inner");
        let mut chosen = usize::from(settings.miter_inner == Miter::Arc);
        if ui.combo_simple_string("##miter_inner", &mut chosen, &["Sharp", "Arc"]) {
            settings.miter_inner = if chosen == 1 {
                Miter::Arc
            } else {
                Miter::Sharp
            };
        }
        tooltip(ui, "The inside of the turn", "");
        let mitered = settings.miter_outer != Miter::Sharp || settings.miter_inner != Miter::Sharp;
        let disabled = ui.begin_disabled(!mitered);
        row(ui, "Spread");
        ui.slider_config("##bevel_spread", 0.0, MAX_SPREAD)
            .display_format("%.2f")
            .flags(SliderFlags::ALWAYS_CLAMP)
            .build(&mut settings.spread);
        tooltip(
            ui,
            "How far a patch or arc spreads along the edges, as a share of the width",
            "",
        );
        drop(disabled);
        row(ui, "Intersection");
        let mut chosen = usize::from(settings.intersection == Intersection::Cutoff);
        if ui.combo_simple_string(
            "##bevel_intersection",
            &mut chosen,
            &["Grid Fill", "Cutoff"],
        ) {
            settings.intersection = if chosen == 1 {
                Intersection::Cutoff
            } else {
                Intersection::GridFill
            };
        }
        tooltip(
            ui,
            "Where three or more beveled edges meet: a rounded patch, or each end cut off flat",
            "",
        );
        // Blender's Material Index.
        row(ui, "Material");
        let mut names = vec!["From the faces around".to_string()];
        names.extend(info.materials.iter().map(|r| r.material.name().to_string()));
        let mut chosen = settings
            .material
            .and_then(|m| info.materials.iter().position(|r| r.id == m))
            .map_or(0, |i| i + 1);
        if ui.combo_simple_string("##bevel_material", &mut chosen, &names) {
            settings.material = chosen
                .checked_sub(1)
                .and_then(|i| info.materials.get(i))
                .map(|r| r.id);
        }
        tooltip(ui, "The material the bevel's new faces get", "");
    }

    settings.width = settings.width.max(0.0);
    settings.shape = settings.shape.clamp(0.0, 1.0);
    settings.spread = settings.spread.clamp(0.0, MAX_SPREAD);
    if *settings != before {
        *action = Some(Action::AdjustBevel);
    }
}

/// The custom profile, drawn in its square: the old sharp edge at the bottom left, the
/// shape's inside toward the top right. Drag a point to move it; click the line to add
/// one; right-click a point to remove it; double-click it to make it sharp or smooth.
fn profile_editor(
    ui: &Ui,
    profile: &mut CustomProfile,
    segments: usize,
    dragging: &mut Option<usize>,
) {
    let side = ui.content_region_avail()[0].min(190.0);
    let pad = 10.0;
    let start = ui.cursor_screen_pos();
    let origin = [
        start[0] + (ui.content_region_avail()[0] - side) / 2.0,
        start[1],
    ];
    let inner = side - 2.0 * pad;
    let to_screen = |p: [f32; 2]| {
        [
            origin[0] + pad + p[0] * inner,
            origin[1] + pad + (1.0 - p[1]) * inner,
        ]
    };
    let from_screen = |q: [f32; 2]| {
        [
            (q[0] - origin[0] - pad) / inner,
            1.0 - (q[1] - origin[1] - pad) / inner,
        ]
    };
    ui.set_cursor_screen_pos(origin);
    ui.invisible_button("##profile_editor", [side, side]);
    let hovered = ui.is_item_hovered();
    let active = ui.is_item_active();
    let mouse = ui.io().mouse_pos;
    let points: Vec<[f32; 2]> = profile.points().iter().map(|p| [p.x, p.y]).collect();
    let near_point = |q: [f32; 2]| {
        points
            .iter()
            .enumerate()
            .map(|(i, &p)| {
                let s = to_screen(p);
                (i, (s[0] - q[0]).hypot(s[1] - q[1]))
            })
            .filter(|&(_, d)| d < 9.0)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    };
    if hovered && ui.is_mouse_clicked(imgui::MouseButton::Left) {
        *dragging = near_point(mouse).or_else(|| {
            // A click on the line adds a point there.
            let (line, starts) = profile.line(16);
            let mut best: Option<(usize, f32)> = None;
            for (piece, &from) in starts.iter().enumerate() {
                let to = starts.get(piece + 1).copied().unwrap_or(line.len() - 1);
                for k in from..to {
                    let (a, b) = (to_screen(line[k]), to_screen(line[k + 1]));
                    let d = distance_to_segment(mouse, a, b);
                    if d < 7.0 && best.is_none_or(|(_, bd)| d < bd) {
                        best = Some((piece, d));
                    }
                }
            }
            best.and_then(|(piece, _)| {
                let p = from_screen(mouse);
                profile.insert_after(piece, p[0], p[1])
            })
        });
    }
    if hovered
        && ui.is_mouse_double_clicked(imgui::MouseButton::Left)
        && let Some(i) = near_point(mouse)
    {
        profile.toggle_sharp(i);
        *dragging = None;
    }
    if hovered
        && ui.is_mouse_clicked(imgui::MouseButton::Right)
        && let Some(i) = near_point(mouse)
    {
        profile.remove(i);
        *dragging = None;
    }
    if active && let Some(i) = *dragging {
        let p = from_screen(mouse);
        profile.move_point(i, p[0], p[1]);
    } else if !active {
        *dragging = None;
    }

    let list = ui.get_window_draw_list();
    let (a, b) = ([origin[0], origin[1]], [origin[0] + side, origin[1] + side]);
    list.add_rect(a, b, color::FRAME())
        .filled(true)
        .rounding(6.0)
        .build();
    // The shape's inside, and the two faces meeting at the old sharp edge.
    let corner = to_screen([0.0, 0.0]);
    let (right, top) = (to_screen([1.0, 0.0]), to_screen([0.0, 1.0]));
    list.add_line(corner, right, color::MUTED())
        .thickness(2.0)
        .build();
    list.add_line(corner, top, color::MUTED())
        .thickness(2.0)
        .build();
    list.add_line(right, top, with_alpha(color::MUTED(), 0.25))
        .build();
    let (line, _) = profile.line(16);
    let screen: Vec<[f32; 2]> = line.iter().map(|&p| to_screen(p)).collect();
    list.add_polyline(screen, color::ACCENT())
        .thickness(2.0)
        .build();
    // Where the bevel's new edges will run.
    for p in profile.sample(segments) {
        list.add_circle(to_screen(p), 3.0, color::ACCENT_HOVER())
            .thickness(1.5)
            .build();
    }
    for (i, point) in profile.points().iter().enumerate() {
        let s = to_screen([point.x, point.y]);
        let lit = *dragging == Some(i) || (hovered && near_point(mouse) == Some(i));
        let tint = if lit {
            color::ACCENT_HOVER()
        } else {
            color::TEXT()
        };
        if point.sharp {
            list.add_rect([s[0] - 4.0, s[1] - 4.0], [s[0] + 4.0, s[1] + 4.0], tint)
                .filled(true)
                .build();
        } else {
            list.add_circle(s, 4.5, tint).filled(true).build();
        }
    }
    drop(list);
    if hovered && dragging.is_none() {
        ui.tooltip_text(
            "Drag a point to move it. Click the line to add a point.\nRight-click a point to \
             remove it, double-click it for sharp or smooth.\nRings show where the new edges go.",
        );
    }
}

fn distance_to_segment(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let length = ab[0] * ab[0] + ab[1] * ab[1];
    let t = if length > 0.0 {
        (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let q = [a[0] + ab[0] * t, a[1] + ab[1] * t];
    (p[0] - q[0]).hypot(p[1] - q[1])
}

/// Draws the move, rotate or scale handles over the 3D view.
/// Window pixels to the interface's units, which differ when the interface is drawn
/// bigger (Settings > Interface size). The 3D view and the mouse work in pixels.
fn pixels_to_ui(ui: &Ui) -> impl Fn([f32; 2]) -> [f32; 2] + use<> {
    let k = 1.0 / ui.io().display_framebuffer_scale[0].max(0.01);
    move |p: [f32; 2]| [p[0] * k, p[1] * k]
}

fn pixel_length(ui: &Ui, length: f32) -> f32 {
    length / ui.io().display_framebuffer_scale[0].max(0.01)
}

pub fn draw_gizmo(ui: &Ui, parts: &[crate::gizmo::Part], hover: Option<crate::gizmo::Handle>) {
    use crate::gizmo::{Handle, Shape};
    if parts.is_empty() {
        return;
    }
    let at = pixels_to_ui(ui);
    let list = ui.get_background_draw_list();
    for part in parts {
        let lit = hover == Some(part.handle);
        let base = match part.handle {
            Handle::Axis(axis) => color::AXIS()[axis],
            Handle::View => [0.92, 0.92, 0.92, 0.85],
        };
        let tint = if lit {
            let mut c = base;
            for v in c.iter_mut().take(3) {
                *v = (*v * 0.6 + 0.4).min(1.0);
            }
            c[3] = 1.0;
            c
        } else {
            base
        };
        let thickness = if lit { 4.0 } else { 2.5 };
        for shape in &part.shapes {
            match shape {
                Shape::Line(points) => {
                    for pair in points.windows(2) {
                        list.add_line(at(pair[0]), at(pair[1]), tint)
                            .thickness(thickness)
                            .build();
                    }
                }
                Shape::Triangle([a, b, c]) => {
                    list.add_triangle(at(*a), at(*b), at(*c), tint)
                        .filled(true)
                        .build();
                }
                Shape::Square(center, half) => {
                    let h = pixel_length(ui, if lit { half + 1.5 } else { *half });
                    let center = at(*center);
                    list.add_rect(
                        [center[0] - h, center[1] - h],
                        [center[0] + h, center[1] + h],
                        tint,
                    )
                    .filled(true)
                    .build();
                }
                Shape::Circle(center, radius) => {
                    list.add_circle(at(*center), pixel_length(ui, *radius), tint)
                        .num_segments(64)
                        .thickness(thickness)
                        .build();
                }
            }
        }
    }
}

/// A knife path being drawn, and where it will cut.
pub fn draw_knife(ui: &Ui, path: &[[f32; 2]], cuts: &[[f32; 2]]) {
    let at = pixels_to_ui(ui);
    let list = ui.get_foreground_draw_list();
    let line = [1.0, 0.85, 0.25, 1.0];
    for pair in path.windows(2) {
        list.add_line(at(pair[0]), at(pair[1]), line)
            .thickness(2.0)
            .build();
    }
    for &p in &path[..path.len().saturating_sub(1)] {
        list.add_circle(at(p), 3.5, line).filled(true).build();
    }
    for &p in cuts {
        list.add_circle(at(p), 4.5, [0.35, 0.95, 0.45, 1.0])
            .filled(true)
            .build();
        list.add_circle(at(p), 4.5, [0.05, 0.08, 0.05, 1.0]).build();
    }
}

/// Blender's dashed line from the selection's center to the mouse, during a bevel,
/// rotation or scale. While a bevel drag holds the real cursor still, a crosshair
/// stands in for it where the mouse would be.
pub fn draw_guide(ui: &Ui, center: [f32; 2], pointer: [f32; 2], crosshair: bool) {
    let at = pixels_to_ui(ui);
    let list = ui.get_foreground_draw_list();
    let (a, b) = (at(center), at(pointer));
    let length = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
    let dash = 5.0;
    if length > 1.0 {
        let step = [(b[0] - a[0]) / length, (b[1] - a[1]) / length];
        let point = |d: f32| [a[0] + step[0] * d, a[1] + step[1] * d];
        // Light under dark, so the line shows on both light faces and the dark grid.
        list.add_line(a, b, [1.0, 1.0, 1.0, 0.35])
            .thickness(1.0)
            .build();
        let mut d = 0.0;
        while d < length {
            list.add_line(
                point(d),
                point((d + dash).min(length)),
                [0.0, 0.0, 0.0, 0.85],
            )
            .thickness(1.5)
            .build();
            d += dash * 2.0;
        }
    }
    if crosshair {
        let (arm, gap) = (9.0, 3.0);
        for (dark, width) in [(true, 3.5), (false, 1.5)] {
            let tint = if dark {
                [0.0, 0.0, 0.0, 0.8]
            } else {
                [1.0, 1.0, 1.0, 1.0]
            };
            for [dx, dy] in [[1.0, 0.0], [-1.0, 0.0], [0.0, 1.0], [0.0, -1.0]] {
                list.add_line(
                    [b[0] + dx * gap, b[1] + dy * gap],
                    [b[0] + dx * arm, b[1] + dy * arm],
                    tint,
                )
                .thickness(width)
                .build();
            }
        }
    }
}

/// Lines on top of the 3D view, such as where a loop cut will go.
pub fn draw_lines(ui: &Ui, lines: &[[[f32; 2]; 2]]) {
    let at = pixels_to_ui(ui);
    let list = ui.get_foreground_draw_list();
    for &[a, b] in lines {
        list.add_line(at(a), at(b), [1.0, 0.85, 0.25, 1.0])
            .thickness(2.0)
            .build();
    }
}

/// Draws a selection shape being dragged.
pub fn draw_region(ui: &Ui, region: &Region) {
    let at = pixels_to_ui(ui);
    let list = ui.get_foreground_draw_list();
    let line = [1.0, 0.72, 0.36, 1.0];
    let fill = [1.0, 0.62, 0.2, 0.08];
    match region {
        Region::Box { a, b } => {
            let (a, b) = (at(*a), at(*b));
            let min = [a[0].min(b[0]), a[1].min(b[1])];
            let max = [a[0].max(b[0]), a[1].max(b[1])];
            list.add_rect(min, max, fill).filled(true).build();
            list.add_rect(min, max, line).build();
        }
        Region::Brush { path, radius } => {
            if let Some(last) = path.last() {
                let (center, radius) = (at(*last), pixel_length(ui, *radius));
                list.add_circle(center, radius, fill).filled(true).build();
                list.add_circle(center, radius, line).build();
            }
        }
        Region::Lasso { points } => {
            list.add_polyline(points.iter().map(|&p| at(p)).collect::<Vec<_>>(), line)
                .thickness(1.5)
                .build();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crowded_panels_share_the_column_and_short_ones_stay_whole() {
        let all = [0, 2, 1];
        let none = [None; PANELS];
        // Add Shapes 900 tall, Materials 180, Properties with bevel options 500, in 700.
        let h = fit_heights(&all, &[900.0, 500.0, 180.0], &none, 700.0);
        assert_eq!(h[2], 180.0);
        assert!((h[0] - h[1]).abs() < 0.5 && h[1] > 230.0, "{h:?}");
        assert!((h.iter().sum::<f32>() + 2.0 * MARGIN - 700.0).abs() < 0.5);
        // Room for everything: each fits.
        let h = fit_heights(&all, &[300.0, 120.0, 180.0], &none, 900.0);
        assert_eq!(h, [300.0, 120.0, 180.0]);
        // A dragged panel keeps its height.
        let mut dragged = none;
        dragged[0] = Some(200.0);
        let h = fit_heights(&all, &[900.0, 500.0, 180.0], &dragged, 700.0);
        assert_eq!(h[0], 200.0);
    }
    use mixer_mesh::Mesh;

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
            materials: vec![MaterialRow {
                id: mixer_mesh::Mesh::new()
                    .add_material(Material::new("Red"))
                    .unwrap(),
                material: Material::new("Red"),
                faces: 4,
                selected: 2,
            }],
            plain_faces: (8, 1),
            snap: SnapSteps::default(),
            triangles: 12,
            transform_waiting: false,
            transform_dragging: false,
            triangle_limit: Some(20_000),
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
            document: "a rather long project name that needs shortening.mix",
            unsaved: true,
            confirm: None,
            gizmo_tool: None,
            bevel_panel: false,
            knife: None,
            bisect: false,
            view_only: false,
            parts_selected: 2,
            modifiers: Vec::new(),
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
        let modifier_list = {
            let mut mesh = mixer_mesh::Mesh::new();
            let faces = mesh
                .add_primitive(Primitive::Cube { size: 1.0 }, [0.0; 3])
                .unwrap();
            let faces: Vec<_> = faces.into_iter().map(mixer_mesh::Element::Face).collect();
            mesh.select_elements(
                mixer_mesh::SelectMode::Face,
                &faces,
                mixer_mesh::SelectOp::Replace,
            )
            .unwrap();
            mesh.add_modifier_to_selected(Modifier::mirror()).unwrap();
            mesh.add_modifier_to_selected(Modifier::array()).unwrap();
            mesh.modifiers_of_selected()
        };
        // Cases 4 and 5 show the save-changes question and the path box, which
        // block everything else while open.
        for (case, (target, transform)) in [
            (Target::Nothing, false),
            (Target::Shape(Shape::Cube), true),
            (Target::Edited(Shape::Icosphere), false),
            (Target::Several, false),
            (Target::Nothing, false),
            (Target::Nothing, false),
            (Target::Nothing, false),
        ]
        .into_iter()
        .enumerate()
        {
            // Tall enough that the three docked panels show everything without
            // scrolling; the case with modifiers needs more.
            let height: f32 = if case == 6 { 1800.0 } else { 1100.0 };
            context.io_mut().display_size = [1280.0, height];
            let mut info = info(target, &warnings, transform);
            if case == 6 {
                info.parts_selected = 1;
                info.modifiers = modifier_list.clone();
            }
            if case == 4 {
                info.confirm = Some("Save changes to sword.mix first?");
            }
            // Some buttons only work in edge mode, and the loop cut hint must draw too.
            if target == Target::Several {
                info.mode = ModeChoice::Edge;
                info.loop_cut = Some(2);
            }
            if target == Target::Edited(Shape::Icosphere) {
                info.transform_help = Some((Kind::Bevel, None));
                info.bevel = Some((0.25, 3, WidthType::Percent));
            }
            // The bevel panel and a lit handle tool.
            if target == Target::Nothing && case == 0 {
                info.bevel_panel = true;
                info.gizmo_tool = Some(Kind::Rotate);
                info.knife = Some(true);
            }
            if case == 2 {
                info.bisect = true;
                info.view_only = true;
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
                    text: "/somewhere/model.mix".to_string(),
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
            for y in (0..height as i32).step_by(9) {
                for x in (0..1280).step_by(23) {
                    let (x, y) = (x as f32, y as f32);
                    // Folding the bevel panel would hide its controls from the rest of the sweep.
                    state.bevel_folded = false;
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
            // The View mode button. The File menu, opened earlier in each row of the sweep,
            // is still open when the sweep reaches it, so click it on its own.
            click(&mut context, &mut state, 389.0, 25.0);
            // The mirror tool: the seventh button, below one gap between groups.
            let size = tool_button_size(height);
            let mirror_button = [
                MARGIN + 8.0 + size / 2.0,
                TOP_BAR + MARGIN + 8.0 + 6.0 * (size + 4.0) + 14.0 + size / 2.0,
            ];
            // The export button (top right) and the mirror tool (in the tool strip).
            for (opener, xs, ys) in [
                ([1215.0, 25.0], 900..1280, 52..480),
                (mirror_button, 60..320, 300..520),
                // The File menu, beside the name at the top left.
                ([132.0, 25.0], 80..400, 40..260),
            ] {
                for y in ys.step_by(11) {
                    for x in xs.clone().step_by(13) {
                        click(&mut context, &mut state, opener[0], opener[1]);
                        click(&mut context, &mut state, x as f32, y as f32);
                    }
                }
                // Close whatever is still open.
                click(&mut context, &mut state, 640.0, height - 10.0);
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
            "AdjustBevel",
            "GizmoTool(Move)",
            "Knife",
            "Bisect",
            "Join",
            "Separate",
            "Boolean(Union)",
            "Boolean(Subtract)",
            "Boolean(Intersect)",
            "ToggleView",
            "StartWindow",
            "Settings",
            "Shade(0)",
            "Shade(180)",
            "Shade(30)",
            "MarkSharp(true)",
            "MarkSharp(false)",
            "New",
            "Open",
            "Save",
            "SaveAs",
            "ConfirmSave",
            "ConfirmDiscard",
            "ConfirmCancel",
            "PathChosen",
            "PathCancelled",
            "AddModifier(Mirror)",
            "AddModifier(Array)",
            "ChangeModifier",
            "ApplyModifiers",
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
        assert!(actions.iter().any(|a| a.starts_with("RemoveModifier(")));
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
        // The two docked panels' rectangles and the Properties panel's content height.
        let layout = |context: &mut imgui::Context, target, height: f32| {
            context.io_mut().display_size = [1280.0, height];
            let info = info(target, &warnings, false);
            let mut state = PanelState::default();
            let mut rects = Vec::new();
            for _ in 0..3 {
                let ui = context.new_frame();
                // The panels come after the two bars and the tool strip.
                rects = build(ui, &mut state, &info, &fonts).rects[3..5].to_vec();
                context.render();
            }
            (rects, state.props_content)
        };
        let (_, empty) = layout(&mut context, Target::Nothing, 800.0);
        let (rects, sphere) = layout(&mut context, Target::Shape(Shape::UvSphere), 800.0);
        assert!(empty < sphere, "{empty} vs {sphere}");
        // Stacked: Add Shapes above Properties, both inside the column.
        let bottom = 800.0 - STATUS_BAR - MARGIN;
        assert!(rects[0][1] + rects[0][3] < rects[1][1]);
        assert!(
            (rects[1][1] + rects[1][3] - bottom).abs() <= 2.0,
            "{rects:?}"
        );
        let (small, _) = layout(&mut context, Target::Shape(Shape::UvSphere), 400.0);
        assert!((small[1][1] + small[1][3] - (400.0 - STATUS_BAR - MARGIN)).abs() <= 2.0);
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
        let reserved = Dock::default().reserved();
        let [x, y, w, h] = viewport_area([1280.0, 800.0], reserved);
        assert!(x > tool_button_size(800.0) && y == TOP_BAR);
        assert!(x + w < 1280.0 - RIGHT_PANEL && y + h == 800.0 - STATUS_BAR);
        let tiny = viewport_area([100.0, 50.0], reserved);
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
