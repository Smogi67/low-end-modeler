//! The tool panel, built with Dear ImGui.

use imgui::{Condition, Drag, Ui, WindowFlags};
use modeler_mesh::{MAX_SUBDIVISIONS, Primitive};

pub const PANEL_MARGIN: f32 = 8.0;
pub const PANEL_WIDTH: f32 = 236.0;

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

/// What the document looks like, for showing in the panel.
pub struct PanelInfo<'a> {
    pub triangles: usize,
    pub vertices: usize,
    pub faces: usize,
    pub warnings: &'a [String],
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
    #[cfg(windows)]
    OpenExportFolder,
}

/// Builds the panel. Returns the action, and the panel's height.
pub fn panel(ui: &Ui, state: &mut PanelState, info: &PanelInfo) -> (Option<Action>, f32) {
    let mut action = None;
    let mut height = 0.0;
    ui.window("Tools")
        .position([PANEL_MARGIN, PANEL_MARGIN], Condition::Always)
        .size([PANEL_WIDTH, 0.0], Condition::Always)
        .flags(
            WindowFlags::NO_MOVE
                | WindowFlags::NO_RESIZE
                | WindowFlags::NO_COLLAPSE
                | WindowFlags::ALWAYS_AUTO_RESIZE,
        )
        .build(|| {
            let half = (PANEL_WIDTH - 24.0) / 2.0;

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

            let width = PANEL_WIDTH - 100.0;
            ui.set_next_item_width(width);
            Drag::new("Size")
                .range(0.01, 1000.0)
                .speed(0.05)
                .build(ui, &mut state.size);
            ui.set_next_item_width(width);
            ui.slider("Segments", 3, 64, &mut state.segments);
            ui.set_next_item_width(width);
            ui.slider("Rings", 2, 32, &mut state.rings);
            ui.set_next_item_width(width);
            ui.slider(
                "Icosphere",
                0,
                MAX_SUBDIVISIONS.min(5) as i32,
                &mut state.subdivisions,
            );
            ui.set_next_item_width(width);
            ui.slider("Grid cuts", 1, 64, &mut state.cuts);
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
        for (segments, rings, subdivisions, cuts) in [(3, 2, 0, 1), (64, 32, 2, 64)] {
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
