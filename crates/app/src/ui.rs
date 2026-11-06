//! The tool panel, built with Dear ImGui.

use imgui::{Condition, Drag, SliderFlags, Ui, WindowFlags};
use modeler_mesh::{MAX_SUBDIVISIONS, Primitive};

use crate::editor::Target;
use crate::picking::Region;
use crate::transform::Kind;

pub const PANEL_MARGIN: f32 = 8.0;
pub const PANEL_WIDTH: f32 = 300.0;

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

    fn name(self) -> &'static str {
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

/// What the document looks like, for showing in the panel.
pub struct PanelInfo<'a> {
    pub triangles: usize,
    pub vertices: usize,
    pub faces: usize,
    pub warnings: &'a [String],
    pub target: Target,
    pub mode: ModeChoice,
    pub selected_vertices: usize,
    pub transform_help: Option<(Kind, Option<usize>)>,
    pub undo_label: Option<&'static str>,
    pub redo_label: Option<&'static str>,
    pub export_folder: &'a str,
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
    #[cfg(windows)]
    OpenExportFolder,
}

/// Fills a row with buttons of equal width. Labels must be unique in the panel.
fn button_row(ui: &Ui, action: &mut Option<Action>, buttons: &[(&str, Action)], enabled: bool) {
    let gap = ui.clone_style().item_spacing[0];
    let count = buttons.len() as f32;
    let width = (PANEL_WIDTH - 28.0 - gap * (count - 1.0)) / count;
    let disabled = ui.begin_disabled(!enabled);
    for (i, (label, choice)) in buttons.iter().enumerate() {
        if i > 0 {
            ui.same_line();
        }
        if ui.button_with_size(label, [width, 0.0]) {
            *action = Some(*choice);
        }
    }
    disabled.end();
}

const AXIS_NAMES: [&str; 3] = ["X", "Y", "Z"];

/// Builds the panel. Returns the action, and the panel's height.
pub fn panel(ui: &Ui, state: &mut PanelState, info: &PanelInfo) -> (Option<Action>, f32) {
    let mut action = None;
    let mut height = 0.0;
    ui.window("Tools")
        .position([PANEL_MARGIN, PANEL_MARGIN], Condition::Always)
        .size(
            [PANEL_WIDTH, ui.io().display_size[1] - PANEL_MARGIN * 2.0],
            Condition::Always,
        )
        .flags(WindowFlags::NO_MOVE | WindowFlags::NO_RESIZE | WindowFlags::NO_COLLAPSE)
        .build(|| {
            let half = (PANEL_WIDTH - 24.0) / 2.0;

            if let Some((kind, axis)) = info.transform_help {
                ui.text_colored([1.0, 0.7, 0.3, 1.0], format!("{}...", kind.name()));
                ui.text_wrapped("Move the mouse. Left click confirms. Right click or Esc cancels.");
                ui.text_wrapped("X, Y or Z locks to an axis. Hold Ctrl to snap.");
                if let Some(axis) = axis {
                    ui.text(format!("Locked to {}", AXIS_NAMES[axis]));
                }
                ui.separator();
            }

            ui.text("Select");
            let mode_button = |label: &str, mode: ModeChoice, action: &mut Option<Action>| {
                let color = (info.mode == mode).then(|| {
                    ui.push_style_color(imgui::StyleColor::Button, [0.85, 0.5, 0.15, 1.0])
                });
                if ui.button_with_size(label, [(PANEL_WIDTH - 52.0) / 4.0, 0.0]) {
                    *action = Some(Action::SetMode(mode));
                }
                drop(color);
            };
            mode_button("Part 4", ModeChoice::Part, &mut action);
            ui.same_line();
            mode_button("Vertex 1", ModeChoice::Vertex, &mut action);
            ui.same_line();
            mode_button("Edge 2", ModeChoice::Edge, &mut action);
            ui.same_line();
            mode_button("Face 3", ModeChoice::Face, &mut action);
            ui.text_disabled("Click selects. Shift adds, Ctrl removes.");
            ui.text("Drag:");
            ui.same_line();
            ui.radio_button("Off", &mut state.tool, DragTool::ClickOnly);
            ui.same_line();
            ui.radio_button("Box", &mut state.tool, DragTool::Box);
            ui.same_line();
            ui.radio_button("Circle", &mut state.tool, DragTool::Circle);
            ui.same_line();
            ui.radio_button("Lasso", &mut state.tool, DragTool::Lasso);
            button_row(
                ui,
                &mut action,
                &[
                    ("All (A)", Action::SelectAll),
                    ("None", Action::SelectNone),
                    ("Invert", Action::SelectInvert),
                ],
                true,
            );
            button_row(
                ui,
                &mut action,
                &[
                    ("Linked (L)", Action::SelectLinked),
                    ("More", Action::SelectMore),
                    ("Less", Action::SelectLess),
                ],
                true,
            );
            ui.text_disabled(format!("Selected: {} vertices", info.selected_vertices));

            ui.separator();
            ui.text("Transform");
            let any = info.selected_vertices > 0;
            button_row(
                ui,
                &mut action,
                &[
                    ("Move (G)", Action::Transform(Kind::Move)),
                    ("Rotate (R)", Action::Transform(Kind::Rotate)),
                    ("Scale (S)", Action::Transform(Kind::Scale)),
                ],
                any,
            );
            button_row(
                ui,
                &mut action,
                &[("Duplicate (Shift+D)", Action::Duplicate)],
                any,
            );
            button_row(
                ui,
                &mut action,
                &[
                    ("Mirror X", Action::Mirror(0)),
                    ("Mirror Y", Action::Mirror(1)),
                    ("Mirror Z", Action::Mirror(2)),
                ],
                any,
            );

            ui.separator();
            ui.text("Edit");
            button_row(
                ui,
                &mut action,
                &[
                    ("Extrude (E)", Action::Extrude),
                    ("Inset (I)", Action::Inset),
                    ("Delete (X)", Action::Delete),
                ],
                any,
            );
            button_row(
                ui,
                &mut action,
                &[
                    ("Merge (M)", Action::Merge),
                    ("Fill (F)", Action::Fill),
                    ("Subdivide", Action::Subdivide),
                ],
                any,
            );
            button_row(
                ui,
                &mut action,
                &[("Merge close vertices", Action::MergeByDistance)],
                any,
            );

            ui.separator();
            ui.text("Add shape");
            for (i, shape) in Shape::ALL.into_iter().enumerate() {
                if ui.button_with_size(shape.name(), [half, 0.0]) {
                    action = Some(Action::Add(shape));
                }
                if i % 2 == 0 {
                    ui.same_line();
                }
            }
            ui.new_line();

            // Every label in a window must be different. Dear ImGui builds IDs from
            // them, and two widgets sharing an ID can crash it.
            ui.separator();
            let shape = match info.target {
                Target::Shape(shape) => {
                    ui.text(format!("Changing: {}", shape.name()));
                    Some(shape)
                }
                Target::Nothing => {
                    ui.text("Settings for the next shape");
                    None
                }
                Target::Edited(shape) => {
                    ui.text_wrapped(format!(
                        "This {} was reshaped, so these only apply to the next shape.",
                        shape.name()
                    ));
                    None
                }
                Target::Several => {
                    ui.text_wrapped(
                        "Several shapes are selected. Select one to change it. These apply to \
                         the next shape.",
                    );
                    None
                }
            };
            let width = PANEL_WIDTH - 126.0;
            let applies = |setting: Setting| shape.is_none_or(|s| s.uses(setting));

            ui.set_next_item_width(width);
            Drag::new("Size")
                .range(0.01, 1000.0)
                .speed(0.05)
                .flags(SliderFlags::ALWAYS_CLAMP)
                .build(ui, &mut state.size);
            let int_slider = |label: &str, setting, max, value: &mut i32, min| {
                let disabled = ui.begin_disabled(!applies(setting));
                ui.set_next_item_width(width);
                ui.slider_config(label, min, max)
                    .flags(SliderFlags::ALWAYS_CLAMP)
                    .build(value);
                disabled.end();
            };
            int_slider("Segments", Setting::Segments, 64, &mut state.segments, 3);
            int_slider("Rings", Setting::Rings, 32, &mut state.rings, 2);
            int_slider(
                "Subdivisions",
                Setting::Subdivisions,
                MAX_SUBDIVISIONS.min(4) as i32,
                &mut state.subdivisions,
                0,
            );
            int_slider("Grid cuts", Setting::Cuts, 64, &mut state.cuts, 1);
            ui.set_next_item_width(width);
            ui.input_float3("Location", &mut state.location).build();

            ui.separator();
            let disabled = ui.begin_disabled(info.undo_label.is_none());
            if ui.button_with_size("Undo", [half, 0.0]) {
                action = Some(Action::Undo);
            }
            disabled.end();
            ui.same_line();
            let disabled = ui.begin_disabled(info.redo_label.is_none());
            if ui.button_with_size("Redo", [half, 0.0]) {
                action = Some(Action::Redo);
            }
            disabled.end();
            ui.text_disabled(format!("Last: {}", info.undo_label.unwrap_or("nothing")));

            ui.separator();
            ui.text(format!("Triangles: {}", info.triangles));
            ui.text(format!("Vertices: {}", info.vertices));
            ui.text(format!("Faces: {}", info.faces));

            ui.separator();
            ui.text("Export OBJ");
            ui.set_next_item_width(width);
            ui.input_text("File name", &mut state.file_name).build();
            ui.set_next_item_width(width);
            Drag::new("Scale")
                .range(0.001, 1000.0)
                .speed(0.01)
                .build(ui, &mut state.scale);
            ui.text_disabled("Scale 1: 1 unit = 1 stud");
            for warning in info.warnings {
                ui.text_wrapped(warning);
            }
            let disabled = ui.begin_disabled(info.triangles == 0);
            if ui.button_with_size("Export", [PANEL_WIDTH - 16.0, 0.0]) {
                action = Some(Action::Export);
            }
            disabled.end();
            if !state.message.is_empty() {
                ui.text_wrapped(&state.message);
                #[cfg(windows)]
                if ui.button_with_size("Open folder", [PANEL_WIDTH - 16.0, 0.0]) {
                    action = Some(Action::OpenExportFolder);
                }
            }
            ui.text_wrapped(format!("Saved in: {}", info.export_folder));
            height = ui.window_size()[1];
        });
    (action, height)
}

/// Draws a selection shape being dragged.
pub fn draw_region(ui: &Ui, region: &Region) {
    let list = ui.get_foreground_draw_list();
    let color = [1.0, 0.75, 0.3, 1.0];
    match region {
        Region::Box { a, b } => {
            list.add_rect(*a, *b, color).build();
        }
        Region::Brush { path, radius } => {
            if let Some(last) = path.last() {
                list.add_circle(*last, *radius, color).build();
            }
            list.add_polyline(path.clone(), color).build();
        }
        Region::Lasso { points } => {
            list.add_polyline(points.clone(), color).build();
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

    /// Clicks and drags all over the panel, with no window or graphics card. Two widgets
    /// sharing an ID crashed the app when clicked, and this would crash the same way.
    #[test]
    fn clicking_and_dragging_everywhere_on_the_panel_is_safe() {
        let mut context = imgui::Context::create();
        context.set_ini_filename(None);
        context.fonts().build_rgba32_texture();
        context.io_mut().display_size = [1024.0, 1000.0];
        context.io_mut().delta_time = 1.0 / 60.0;
        let warnings = vec!["A warning".to_string()];

        let mut clicked_actions = 0;
        for target in [
            Target::Nothing,
            Target::Shape(Shape::Cube),
            Target::Edited(Shape::Icosphere),
            Target::Several,
        ] {
            let info = PanelInfo {
                triangles: 12,
                vertices: 8,
                faces: 6,
                warnings: &warnings,
                target,
                mode: ModeChoice::Part,
                selected_vertices: 8,
                transform_help: Some((Kind::Move, Some(0))),
                undo_label: Some("Add Cube"),
                redo_label: Some("Add Cube"),
                export_folder: "exports",
            };
            let mut state = PanelState {
                message: "Saved".to_string(),
                ..Default::default()
            };
            let mut frame = |context: &mut imgui::Context, state: &mut PanelState| {
                let ui = context.new_frame();
                let (action, _) = panel(ui, state, &info);
                context.render();
                clicked_actions += usize::from(action.is_some());
            };
            frame(&mut context, &mut state);
            frame(&mut context, &mut state);
            for y in (0..1000).step_by(6) {
                for x in (10..PANEL_WIDTH as i32).step_by(25) {
                    let (x, y) = (x as f32, y as f32);
                    let io = context.io_mut();
                    io.add_mouse_pos_event([x, y]);
                    frame(&mut context, &mut state);
                    context
                        .io_mut()
                        .add_mouse_button_event(imgui::MouseButton::Left, true);
                    frame(&mut context, &mut state);
                    context.io_mut().add_mouse_pos_event([x + 15.0, y]);
                    frame(&mut context, &mut state);
                    context
                        .io_mut()
                        .add_mouse_button_event(imgui::MouseButton::Left, false);
                    frame(&mut context, &mut state);
                    // Text fields would otherwise keep the keyboard between clicks.
                    frame(&mut context, &mut state);
                }
            }
            assert!(state.size.is_finite() && state.size > 0.0);
            assert!((3..=64).contains(&state.segments));
            assert!((2..=32).contains(&state.rings));
            assert!((0..=4).contains(&state.subdivisions));
            assert!((1..=64).contains(&state.cuts));
        }
        assert!(
            clicked_actions > 0,
            "the test must really reach the buttons"
        );
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
