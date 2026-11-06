//! The document, plus what is needed to keep adjusting the shape just added, as with
//! Blender's Adjust Last Operation panel. No window or graphics code, so it is tested
//! directly.

use modeler_mesh::{Document, Element, Mesh, MeshError, Primitive, SelectMode, SelectOp};

use crate::ui::{PanelState, Shape};

/// The newest shape, which the panel's values currently control.
#[derive(Debug, Clone, Copy, PartialEq)]
struct LastAdd {
    shape: Shape,
    primitive: Primitive,
    center: [f32; 3],
}

pub struct Editor {
    document: Document,
    last_add: Option<LastAdd>,
    mode: SelectMode,
}

impl Editor {
    pub fn new() -> Self {
        Self {
            document: Document::new(Mesh::new()),
            last_add: None,
            mode: SelectMode::Vertex,
        }
    }

    pub fn document(&self) -> &Document {
        &self.document
    }

    /// The shape that changing the panel's values would adjust, if any.
    pub fn adjusting(&self) -> Option<Shape> {
        self.last_add.map(|last| last.shape)
    }

    fn insert(
        &mut self,
        shape: Shape,
        primitive: Primitive,
        center: [f32; 3],
    ) -> Result<(), MeshError> {
        // The new shape becomes the selection, ready to move.
        let mode = self.mode;
        self.document.edit(shape.edit_label(), |mesh| {
            let faces = mesh.add_primitive(primitive, center)?;
            let picked: Vec<Element> = faces.into_iter().map(Element::Face).collect();
            mesh.deselect_all();
            mesh.select_elements(SelectMode::Face, &picked, SelectOp::Add)?;
            mesh.flush_selection(mode);
            Ok(())
        })?;
        self.last_add = Some(LastAdd {
            shape,
            primitive,
            center,
        });
        Ok(())
    }

    /// The active selection mode.
    pub fn mode(&self) -> SelectMode {
        self.mode
    }

    /// Switches modes, keeping what the new mode can hold. Returns whether it changed.
    pub fn set_mode(&mut self, mode: SelectMode) -> bool {
        if self.mode == mode {
            return false;
        }
        self.mode = mode;
        self.last_add = None;
        let _ = self.document.edit("Select mode", |mesh| {
            mesh.flush_selection(mode);
            Ok(())
        });
        true
    }

    /// Runs one edit as an undo step. Ends adjusting of the newest shape.
    pub fn run(
        &mut self,
        label: &'static str,
        edit: impl FnOnce(&mut Mesh, SelectMode) -> Result<(), MeshError>,
    ) -> Result<(), MeshError> {
        self.last_add = None;
        let mode = self.mode;
        self.document.edit(label, |mesh| edit(mesh, mode))
    }

    /// Starts an edit that stays open during a drag.
    pub fn begin(&mut self, label: &'static str) -> Result<(), MeshError> {
        self.last_add = None;
        self.document.begin_edit(label)
    }

    /// The mesh during an open edit.
    pub fn active_mesh(&mut self) -> Option<&mut Mesh> {
        self.document.active_mesh()
    }

    pub fn commit(&mut self) -> Result<bool, MeshError> {
        self.document.commit_edit()
    }

    pub fn cancel(&mut self) -> Result<(), MeshError> {
        self.document.cancel_edit()
    }

    pub fn add(&mut self, shape: Shape, state: &PanelState) -> Result<(), MeshError> {
        self.last_add = None;
        self.insert(shape, state.primitive(shape), state.location)
    }

    /// Rebuilds the newest shape if the panel's values for it changed. Returns whether
    /// the mesh changed. This replaces the shape's undo step, so history does not grow.
    pub fn adjust(&mut self, state: &PanelState) -> Result<bool, MeshError> {
        let Some(last) = self.last_add else {
            return Ok(false);
        };
        let primitive = state.primitive(last.shape);
        let center = state.location;
        if primitive == last.primitive && center == last.center {
            return Ok(false);
        }
        // Checked first, so that bad values leave the current shape in place.
        Mesh::new().add_primitive(primitive, center)?;
        self.document.undo();
        self.insert(last.shape, primitive, center)?;
        Ok(true)
    }

    pub fn undo(&mut self) -> bool {
        self.last_add = None;
        self.document.undo().is_some()
    }

    pub fn redo(&mut self) -> bool {
        self.last_add = None;
        self.document.redo().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vertices(editor: &Editor) -> usize {
        editor.document().mesh().vertex_count()
    }

    #[test]
    fn adjusting_rebuilds_the_newest_shape_without_new_undo_steps() {
        let mut editor = Editor::new();
        let mut state = PanelState {
            segments: 8,
            ..Default::default()
        };
        editor.add(Shape::Cylinder, &state).unwrap();
        assert_eq!(vertices(&editor), 16);
        assert_eq!(editor.adjusting(), Some(Shape::Cylinder));

        state.segments = 12;
        assert_eq!(editor.adjust(&state), Ok(true));
        assert_eq!(vertices(&editor), 24);
        assert_eq!(editor.document().undo_count(), 1);

        state.segments = 3;
        assert_eq!(editor.adjust(&state), Ok(true));
        assert_eq!(vertices(&editor), 6);
        editor.document().mesh().validate().unwrap();
        assert_eq!(editor.document().undo_count(), 1);
        assert_eq!(editor.document().redo_count(), 0);

        assert!(editor.undo());
        assert_eq!(vertices(&editor), 0);
    }

    #[test]
    fn location_and_size_move_and_resize_the_shape() {
        let mut editor = Editor::new();
        let mut state = PanelState::default();
        editor.add(Shape::Cube, &state).unwrap();

        state.location = [10.0, 0.0, 0.0];
        assert_eq!(editor.adjust(&state), Ok(true));
        let mesh = editor.document().mesh();
        assert!(mesh.vertices().all(|(_, v)| v.position()[0] >= 9.0));

        state.size = 4.0;
        state.location = [0.0; 3];
        assert_eq!(editor.adjust(&state), Ok(true));
        let mesh = editor.document().mesh();
        assert!(mesh.vertices().all(|(_, v)| v.position()[0].abs() == 2.0));
    }

    #[test]
    fn only_the_newest_shape_changes() {
        let mut editor = Editor::new();
        let mut state = PanelState {
            segments: 8,
            ..Default::default()
        };
        editor.add(Shape::Cylinder, &state).unwrap();
        editor.add(Shape::Cone, &state).unwrap();
        assert_eq!(vertices(&editor), 16 + 9);
        state.segments = 10;
        assert_eq!(editor.adjust(&state), Ok(true));
        assert_eq!(vertices(&editor), 16 + 11);
        assert_eq!(editor.document().undo_count(), 2);
        assert_eq!(editor.document().undo_label(), Some("Add Cone"));
    }

    #[test]
    fn values_the_shape_does_not_use_change_nothing() {
        let mut editor = Editor::new();
        let mut state = PanelState::default();
        editor.add(Shape::Cube, &state).unwrap();
        let before = editor.document().mesh().clone();
        state.segments = 30;
        state.rings = 20;
        state.subdivisions = 4;
        state.cuts = 9;
        assert_eq!(editor.adjust(&state), Ok(false));
        assert_eq!(*editor.document().mesh(), before);
    }

    #[test]
    fn bad_values_keep_the_current_shape() {
        let mut editor = Editor::new();
        let mut state = PanelState::default();
        editor.add(Shape::Cube, &state).unwrap();
        let before = editor.document().mesh().clone();
        state.size = 0.0;
        assert_eq!(editor.adjust(&state), Err(MeshError::InvalidPrimitive));
        assert_eq!(*editor.document().mesh(), before);
        assert_eq!(editor.adjusting(), Some(Shape::Cube));
        state.size = 2.0;
        state.location[0] = f32::NAN;
        assert_eq!(editor.adjust(&state), Err(MeshError::NonFinitePosition));
        assert_eq!(*editor.document().mesh(), before);
        assert_eq!(editor.document().undo_count(), 1);
    }

    #[test]
    fn nothing_is_adjusted_after_undo_or_redo_or_with_nothing_added() {
        let mut editor = Editor::new();
        let mut state = PanelState::default();
        assert_eq!(editor.adjust(&state), Ok(false));
        assert_eq!(editor.adjusting(), None);

        editor.add(Shape::Cube, &state).unwrap();
        assert!(editor.undo());
        assert_eq!(editor.adjusting(), None);
        state.size = 5.0;
        assert_eq!(editor.adjust(&state), Ok(false));
        assert_eq!(vertices(&editor), 0);

        assert!(editor.redo());
        assert_eq!(editor.adjusting(), None);
        assert_eq!(editor.adjust(&state), Ok(false));
        assert!(!editor.redo());
    }

    #[test]
    fn a_new_shape_uses_the_current_values_and_a_refused_one_adjusts_nothing() {
        let mut editor = Editor::new();
        let mut state = PanelState {
            size: 0.0,
            ..Default::default()
        };
        assert!(editor.add(Shape::Cube, &state).is_err());
        assert_eq!(editor.adjusting(), None);
        state.size = 4.0;
        editor.add(Shape::Cube, &state).unwrap();
        let mesh = editor.document().mesh();
        assert!(mesh.vertices().all(|(_, v)| v.position()[0].abs() == 2.0));
    }

    #[test]
    fn a_new_shape_becomes_the_selection_and_stays_so_when_adjusted() {
        let mut editor = Editor::new();
        let mut state = PanelState {
            segments: 8,
            ..Default::default()
        };
        editor.add(Shape::Cube, &state).unwrap();
        editor.add(Shape::Cylinder, &state).unwrap();
        let selected = |e: &Editor| e.document().mesh().selected_vertices().len();
        assert_eq!(selected(&editor), 16, "only the newest shape");
        state.segments = 12;
        editor.adjust(&state).unwrap();
        assert_eq!(selected(&editor), 24);
        editor.document().mesh().validate().unwrap();
    }

    #[test]
    fn other_edits_end_adjusting() {
        let mut editor = Editor::new();
        let mut state = PanelState::default();
        editor.add(Shape::Cube, &state).unwrap();
        editor
            .run("Select all", |mesh, _| {
                mesh.select_all();
                Ok(())
            })
            .unwrap();
        assert_eq!(editor.adjusting(), None);
        state.size = 5.0;
        assert_eq!(editor.adjust(&state), Ok(false));

        editor.add(Shape::Cube, &state).unwrap();
        assert!(editor.set_mode(SelectMode::Face));
        assert_eq!(editor.adjusting(), None);
        assert!(!editor.set_mode(SelectMode::Face));

        editor.add(Shape::Cube, &state).unwrap();
        editor.begin("Move").unwrap();
        assert_eq!(editor.adjusting(), None);
        assert_eq!(editor.adjust(&state), Ok(false));
        editor.cancel().unwrap();
    }

    #[test]
    fn switching_modes_keeps_a_whole_shape_selected_in_each_mode() {
        let mut editor = Editor::new();
        let state = PanelState::default();
        for mode in [SelectMode::Face, SelectMode::Edge, SelectMode::Vertex] {
            editor.set_mode(mode);
            editor.add(Shape::Cube, &state).unwrap();
            let mesh = editor.document().mesh();
            assert_eq!(mesh.selected_vertices().len(), 8, "{mode:?}");
            assert_eq!(
                mesh.faces().filter(|(_, f)| f.is_selected()).count(),
                6,
                "{mode:?}"
            );
            mesh.validate().unwrap();
        }
    }

    #[test]
    fn a_held_edit_can_commit_or_cancel() {
        let mut editor = Editor::new();
        let state = PanelState::default();
        editor.add(Shape::Cube, &state).unwrap();
        let before = editor.document().mesh().clone();
        let vertex = before.selected_vertices()[0];

        editor.begin("Move").unwrap();
        editor
            .active_mesh()
            .unwrap()
            .set_vertex_position(vertex, [9.0; 3])
            .unwrap();
        editor.cancel().unwrap();
        assert_eq!(*editor.document().mesh(), before);

        editor.begin("Move").unwrap();
        editor
            .active_mesh()
            .unwrap()
            .set_vertex_position(vertex, [9.0; 3])
            .unwrap();
        assert_eq!(editor.commit(), Ok(true));
        assert_ne!(*editor.document().mesh(), before);
        assert!(editor.undo());
        assert_eq!(*editor.document().mesh(), before);
    }

    /// Dragging a slider rebuilds a shape on each change, so memory must stay level.
    #[test]
    fn many_adjustments_do_not_grow_history() {
        let mut editor = Editor::new();
        let mut state = PanelState {
            segments: 8,
            ..Default::default()
        };
        editor.add(Shape::Cylinder, &state).unwrap();
        let mut largest = 0;
        for i in 0..500 {
            state.segments = 8 + (i % 2);
            editor.adjust(&state).unwrap();
            if i > 2 {
                largest = largest.max(editor.document().history_bytes());
            }
        }
        assert!(largest < 64 * 1024, "{largest}");
        assert_eq!(editor.document().undo_count(), 1);
    }
}
