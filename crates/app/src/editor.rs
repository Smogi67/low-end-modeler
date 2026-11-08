//! The document, plus a memory of the shapes that were added, so the panel's sliders can
//! keep changing a shape for as long as it is still as it was made. No window or graphics
//! code, so it is tested directly.

use mixer_mesh::{
    Document, Element, FaceId, Mesh, MeshError, Primitive, SelectMode, SelectOp, VertexId,
};

use crate::ui::{ModeChoice, PanelState, Shape};

/// A shape as it was made. IDs from an undone step go stale, and come back on redo, so a
/// record simply applies whenever all of its IDs are live.
#[derive(Debug, Clone, PartialEq)]
struct Record {
    shape: Shape,
    primitive: Primitive,
    center: [f32; 3],
    vertices: Vec<VertexId>,
    faces: Vec<FaceId>,
}

/// A run of slider changes that shares one undo step.
#[derive(Debug, Clone)]
struct Session {
    /// The shape before the first change, or `None` if the step also added the shape.
    base: Option<Record>,
    /// Where the current version is in the list of records.
    current: usize,
}

/// What the panel's sliders apply to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// Nothing that was made as a shape is selected.
    Nothing,
    /// One shape is selected, and can be changed.
    Shape(Shape),
    /// One shape is selected, but its geometry has been edited.
    Edited(Shape),
    /// Parts of several shapes are selected.
    Several,
}

pub struct Editor {
    document: Document,
    records: Vec<Record>,
    session: Option<Session>,
    mode: SelectMode,
    whole_parts: bool,
    /// Counts every change, so that the target is only worked out when needed.
    revision: u64,
    cached_revision: u64,
    target: Target,
    target_record: Option<usize>,
}

impl Editor {
    pub fn new() -> Self {
        Self {
            document: Document::new(Mesh::new()),
            records: Vec::new(),
            session: None,
            mode: SelectMode::Face,
            whole_parts: true,
            revision: 1,
            cached_revision: 0,
            target: Target::Nothing,
            target_record: None,
        }
    }

    pub fn document(&self) -> &Document {
        &self.document
    }

    /// Starts over with this mesh, as after opening a file. History and shape records
    /// are cleared; the selection mode stays.
    pub fn replace(&mut self, mesh: Mesh) {
        self.document = Document::new(mesh);
        self.records.clear();
        self.session = None;
        self.target_record = None;
        self.changed();
    }

    fn changed(&mut self) {
        self.revision += 1;
    }

    fn end_session(&mut self) {
        self.session = None;
        self.changed();
    }

    /// The panel's mode button that is lit.
    pub fn mode_choice(&self) -> ModeChoice {
        match (self.whole_parts, self.mode) {
            (true, _) => ModeChoice::Part,
            (false, SelectMode::Vertex) => ModeChoice::Vertex,
            (false, SelectMode::Edge) => ModeChoice::Edge,
            (false, SelectMode::Face) => ModeChoice::Face,
        }
    }

    /// The active selection mode. Part mode selects faces, a whole connected part at once.
    pub fn mode(&self) -> SelectMode {
        self.mode
    }

    /// True if a pick should grow to the whole connected part.
    pub fn whole_parts(&self) -> bool {
        self.whole_parts
    }

    /// The mode that decides which dots to draw. Part mode draws none, as edge mode does.
    pub fn drawing_mode(&self) -> SelectMode {
        if self.whole_parts {
            SelectMode::Edge
        } else {
            self.mode
        }
    }

    /// Switches modes, keeping what the new mode can hold. Returns whether it changed.
    pub fn set_mode(&mut self, choice: ModeChoice) -> bool {
        if self.mode_choice() == choice {
            return false;
        }
        let (mode, whole) = match choice {
            ModeChoice::Part => (SelectMode::Face, true),
            ModeChoice::Vertex => (SelectMode::Vertex, false),
            ModeChoice::Edge => (SelectMode::Edge, false),
            ModeChoice::Face => (SelectMode::Face, false),
        };
        self.mode = mode;
        self.whole_parts = whole;
        let _ = self.document.edit("Select mode", |mesh| {
            mesh.flush_selection(mode);
            if whole {
                let picked: Vec<Element> = mesh
                    .faces()
                    .filter(|(_, f)| f.is_selected())
                    .map(|(id, _)| Element::Face(id))
                    .collect();
                let faces = mesh.linked_faces(&picked);
                let all: Vec<Element> = faces.into_iter().map(Element::Face).collect();
                mesh.select_elements(mode, &all, SelectOp::Replace)?;
            }
            Ok(())
        });
        self.end_session();
        true
    }

    fn is_live(&self, record: &Record) -> bool {
        let mesh = self.document.mesh();
        record.vertices.iter().all(|&v| mesh.vertex(v).is_some())
            && record.faces.iter().all(|&f| mesh.face(f).is_some())
    }

    /// If the record's geometry is still as made, returns where its center is now. A shape
    /// that has only been moved as a whole counts, so that it keeps its sliders.
    fn unedited_center(&self, record: &Record) -> Option<[f32; 3]> {
        let mesh = self.document.mesh();
        let mut template = Mesh::new();
        let (made, _) = template
            .add_primitive_parts(record.primitive, record.center)
            .ok()?;
        if made.len() != record.vertices.len() || made.is_empty() {
            return None;
        }
        let position = |m: &Mesh, v: VertexId| m.vertex(v).map(|d| d.position());
        let first_now = position(mesh, record.vertices[0])?;
        let first_made = position(&template, made[0])?;
        let offset: [f32; 3] = std::array::from_fn(|i| first_now[i] - first_made[i]);
        let tolerance = 1e-3 * (1.0 + offset.iter().map(|c| c.abs()).sum::<f32>());
        for (&now, &then) in record.vertices.iter().zip(&made) {
            let (now_id, then_id) = (now, then);
            let now = position(mesh, now_id)?;
            let then = position(&template, then_id)?;
            if (0..3).any(|i| (now[i] - then[i] - offset[i]).abs() > tolerance) {
                return None;
            }
            // Anything attached by a tool since then would be lost in a rebuild.
            if mesh.vertex_edges(now_id).ok()?.count()
                != template.vertex_edges(then_id).ok()?.count()
                || mesh.vertex_faces(now_id).ok()?.count()
                    != template.vertex_faces(then_id).ok()?.count()
            {
                return None;
            }
        }
        Some(std::array::from_fn(|i| record.center[i] + offset[i]))
    }

    /// Works out what the sliders apply to, and shows that shape's values on them.
    pub fn refresh_target(&mut self, state: &mut PanelState) -> Target {
        if self.cached_revision == self.revision {
            return self.target;
        }
        self.cached_revision = self.revision;
        let mesh = self.document.mesh();
        let touched: Vec<usize> = (0..self.records.len())
            .filter(|&i| {
                let record = &self.records[i];
                self.is_live(record)
                    && record
                        .vertices
                        .iter()
                        .any(|&v| mesh.vertex(v).is_some_and(|d| d.is_selected()))
            })
            .collect();
        (self.target, self.target_record) = match touched.as_slice() {
            [] => (Target::Nothing, None),
            [only] => {
                let shape = self.records[*only].shape;
                match self.unedited_center(&self.records[*only]) {
                    Some(center) => {
                        self.records[*only].center = center;
                        state.show_values(self.records[*only].primitive, center);
                        (Target::Shape(shape), Some(*only))
                    }
                    None => (Target::Edited(shape), None),
                }
            }
            _ => (Target::Several, None),
        };
        self.target
    }

    /// Builds a shape in the open-ended edit, and selects it.
    fn build(
        document: &mut Document,
        label: &'static str,
        replacing: Option<&Record>,
        shape: Shape,
        primitive: Primitive,
        center: [f32; 3],
        mode: SelectMode,
    ) -> Result<Record, MeshError> {
        let (vertices, faces) = document.edit(label, |mesh| {
            if let Some(old) = replacing {
                for &vertex in &old.vertices {
                    mesh.remove_vertex(vertex)?;
                }
            }
            let (vertices, faces) = mesh.add_primitive_parts(primitive, center)?;
            let picked: Vec<Element> = faces.iter().map(|&f| Element::Face(f)).collect();
            mesh.deselect_all();
            mesh.select_elements(SelectMode::Face, &picked, SelectOp::Add)?;
            mesh.flush_selection(mode);
            Ok((vertices, faces))
        })?;
        Ok(Record {
            shape,
            primitive,
            center,
            vertices,
            faces,
        })
    }

    pub fn add(&mut self, shape: Shape, state: &PanelState) -> Result<(), MeshError> {
        self.end_session();
        // Records can only come back through redo, so without any they can go.
        if self.document.redo_count() == 0 {
            let live: Vec<Record> = self
                .records
                .iter()
                .filter(|record| self.is_live(record))
                .cloned()
                .collect();
            self.records = live;
        }
        let record = Self::build(
            &mut self.document,
            shape.edit_label(),
            None,
            shape,
            state.primitive(shape),
            state.location,
            self.mode,
        )?;
        self.records.push(record);
        self.session = Some(Session {
            base: None,
            current: self.records.len() - 1,
        });
        Ok(())
    }

    /// Rebuilds the selected shape if the panel's values for it changed. Returns whether
    /// the mesh changed. Changes in a row share one undo step, so history does not grow.
    pub fn adjust(&mut self, state: &PanelState) -> Result<bool, MeshError> {
        let Some(index) = self.target_record else {
            return Ok(false);
        };
        if self.cached_revision != self.revision {
            return Ok(false);
        }
        let record = self.records[index].clone();
        let primitive = state.primitive(record.shape);
        let center = state.location;
        if primitive == record.primitive && center == record.center {
            return Ok(false);
        }
        // Checked first, so that bad values leave the current shape in place.
        Mesh::new().add_primitive(primitive, center)?;

        let continuing = self
            .session
            .as_ref()
            .filter(|s| s.current == index)
            .cloned();
        let base = match &continuing {
            Some(session) => {
                self.document.undo();
                session.base.clone()
            }
            None => Some(record.clone()),
        };
        let label = if base.is_some() {
            "Change shape"
        } else {
            record.shape.edit_label()
        };
        let new_record = Self::build(
            &mut self.document,
            label,
            base.as_ref(),
            record.shape,
            primitive,
            center,
            self.mode,
        )?;
        let current = match continuing {
            Some(session) => {
                self.records[session.current] = new_record;
                session.current
            }
            None => {
                self.records.push(new_record);
                self.records.len() - 1
            }
        };
        self.session = Some(Session { base, current });
        self.target_record = Some(current);
        self.changed();
        // The panel already shows these values, and the next refresh confirms them.
        Ok(true)
    }

    /// Runs one edit as an undo step.
    pub fn run(
        &mut self,
        label: &'static str,
        edit: impl FnOnce(&mut Mesh, SelectMode) -> Result<(), MeshError>,
    ) -> Result<(), MeshError> {
        self.end_session();
        let mode = self.mode;
        self.document.edit(label, |mesh| edit(mesh, mode))
    }

    /// Starts an edit that stays open during a drag.
    pub fn begin(&mut self, label: &'static str) -> Result<(), MeshError> {
        self.end_session();
        self.target_record = None;
        self.document.begin_edit(label)
    }

    /// The mesh during an open edit.
    pub fn active_mesh(&mut self) -> Option<&mut Mesh> {
        self.document.active_mesh()
    }

    pub fn commit(&mut self) -> Result<bool, MeshError> {
        self.changed();
        self.document.commit_edit()
    }

    pub fn cancel(&mut self) -> Result<(), MeshError> {
        self.changed();
        self.document.cancel_edit()
    }

    pub fn undo(&mut self) -> bool {
        self.end_session();
        self.document.undo().is_some()
    }

    pub fn redo(&mut self) -> bool {
        self.end_session();
        self.document.redo().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vertices(editor: &Editor) -> usize {
        editor.document().mesh().vertex_count()
    }

    fn selected(editor: &Editor) -> usize {
        editor.document().mesh().selected_vertices().len()
    }

    /// What the app does each frame: find the target and show its values, let the user
    /// change them, then apply them. Here `state` already holds what the user typed.
    fn frame(editor: &mut Editor, state: &mut PanelState) -> Result<bool, MeshError> {
        let typed = state.clone();
        editor.refresh_target(state);
        *state = typed;
        editor.adjust(state)
    }

    fn select_all(editor: &mut Editor) {
        editor
            .run("Select all", |mesh, _| {
                mesh.select_all();
                Ok(())
            })
            .unwrap();
    }

    fn deselect(editor: &mut Editor) {
        editor
            .run("Select none", |mesh, _| {
                mesh.deselect_all();
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn sliders_change_a_new_shape_in_one_undo_step() {
        let mut editor = Editor::new();
        let mut state = PanelState {
            segments: 8,
            ..Default::default()
        };
        editor.add(Shape::Cylinder, &state).unwrap();
        assert_eq!(vertices(&editor), 16);
        assert_eq!(
            editor.refresh_target(&mut state),
            Target::Shape(Shape::Cylinder)
        );

        state.segments = 12;
        assert_eq!(frame(&mut editor, &mut state), Ok(true));
        assert_eq!(vertices(&editor), 24);
        state.segments = 3;
        assert_eq!(frame(&mut editor, &mut state), Ok(true));
        assert_eq!(vertices(&editor), 6);
        editor.document().mesh().validate().unwrap();
        assert_eq!(editor.document().undo_count(), 1);
        assert_eq!(editor.document().undo_label(), Some("Add Cylinder"));
        assert_eq!(editor.document().redo_count(), 0);

        assert!(editor.undo());
        assert_eq!(vertices(&editor), 0);
    }

    /// What the owner reported: selecting used to stop the sliders working.
    #[test]
    fn sliders_keep_working_after_select_all() {
        let mut editor = Editor::new();
        let mut state = PanelState {
            segments: 8,
            ..Default::default()
        };
        editor.add(Shape::Cylinder, &state).unwrap();
        frame(&mut editor, &mut state).unwrap();
        select_all(&mut editor);
        assert_eq!(
            editor.refresh_target(&mut state),
            Target::Shape(Shape::Cylinder)
        );
        state.segments = 12;
        assert_eq!(frame(&mut editor, &mut state), Ok(true));
        assert_eq!(vertices(&editor), 24);
        assert_eq!(selected(&editor), 24);
    }

    #[test]
    fn selecting_an_older_shape_brings_back_its_sliders_and_values() {
        let mut editor = Editor::new();
        let mut state = PanelState {
            size: 4.0,
            segments: 8,
            location: [10.0, 0.0, 0.0],
            ..Default::default()
        };
        editor.add(Shape::Cylinder, &state).unwrap();
        let older = editor.document().mesh().selected_vertices()[0];
        state.size = 2.0;
        state.segments = 5;
        state.location = [0.0; 3];
        editor.add(Shape::Cone, &state).unwrap();
        frame(&mut editor, &mut state).unwrap();
        let cone_vertices = 6;
        assert_eq!(vertices(&editor), 16 + cone_vertices);

        editor
            .run("Select", |mesh, mode| {
                mesh.deselect_all();
                mesh.select_elements(SelectMode::Vertex, &[Element::Vertex(older)], SelectOp::Add)?;
                mesh.flush_selection(SelectMode::Vertex);
                mesh.select_linked(mode);
                Ok(())
            })
            .unwrap();
        assert_eq!(
            editor.refresh_target(&mut state),
            Target::Shape(Shape::Cylinder)
        );
        assert_eq!(state.size, 4.0, "the sliders show this shape's values");
        assert_eq!(state.segments, 8);
        assert_eq!(state.location, [10.0, 0.0, 0.0]);

        state.segments = 10;
        assert_eq!(frame(&mut editor, &mut state), Ok(true));
        assert_eq!(
            vertices(&editor),
            20 + cone_vertices,
            "only that shape changed"
        );
        editor.document().mesh().validate().unwrap();
    }

    #[test]
    fn a_change_to_an_older_shape_can_be_undone_and_redone() {
        let mut editor = Editor::new();
        let mut state = PanelState {
            segments: 8,
            ..Default::default()
        };
        editor.add(Shape::Cylinder, &state).unwrap();
        frame(&mut editor, &mut state).unwrap();
        select_all(&mut editor);
        frame(&mut editor, &mut state).unwrap();
        let before = editor.document().mesh().clone();

        for segments in [10, 12, 14] {
            state.segments = segments;
            assert_eq!(frame(&mut editor, &mut state), Ok(true));
        }
        assert_eq!(editor.document().undo_label(), Some("Change shape"));
        let changed = editor.document().mesh().clone();
        let steps = editor.document().undo_count();

        assert!(editor.undo());
        assert_eq!(
            *editor.document().mesh(),
            before,
            "one undo reverses the whole drag"
        );
        assert_eq!(editor.document().undo_count(), steps - 1);
        assert_eq!(
            editor.refresh_target(&mut state),
            Target::Shape(Shape::Cylinder)
        );
        assert_eq!(state.segments, 8);
        assert_eq!(frame(&mut editor, &mut state), Ok(false));

        assert!(editor.redo());
        assert_eq!(*editor.document().mesh(), changed);
        assert_eq!(
            editor.refresh_target(&mut state),
            Target::Shape(Shape::Cylinder)
        );
        assert_eq!(state.segments, 14);
        state.segments = 6;
        assert_eq!(frame(&mut editor, &mut state), Ok(true));
        assert_eq!(vertices(&editor), 12);
    }

    #[test]
    fn location_and_size_move_and_resize_the_shape() {
        let mut editor = Editor::new();
        let mut state = PanelState::default();
        editor.add(Shape::Cube, &state).unwrap();
        frame(&mut editor, &mut state).unwrap();

        state.location = [10.0, 0.0, 0.0];
        assert_eq!(frame(&mut editor, &mut state), Ok(true));
        let mesh = editor.document().mesh();
        assert!(mesh.vertices().all(|(_, v)| v.position()[0] >= 9.0));

        state.size = 4.0;
        state.location = [0.0; 3];
        assert_eq!(frame(&mut editor, &mut state), Ok(true));
        let mesh = editor.document().mesh();
        assert!(mesh.vertices().all(|(_, v)| v.position()[0].abs() == 2.0));
    }

    #[test]
    fn values_the_shape_does_not_use_change_nothing() {
        let mut editor = Editor::new();
        let mut state = PanelState::default();
        editor.add(Shape::Cube, &state).unwrap();
        frame(&mut editor, &mut state).unwrap();
        let before = editor.document().mesh().clone();
        state.segments = 30;
        state.rings = 20;
        state.subdivisions = 4;
        state.cuts = 9;
        assert_eq!(frame(&mut editor, &mut state), Ok(false));
        assert_eq!(*editor.document().mesh(), before);
    }

    #[test]
    fn bad_values_keep_the_current_shape() {
        let mut editor = Editor::new();
        let mut state = PanelState::default();
        editor.add(Shape::Cube, &state).unwrap();
        frame(&mut editor, &mut state).unwrap();
        let before = editor.document().mesh().clone();
        state.size = 0.0;
        assert_eq!(
            frame(&mut editor, &mut state),
            Err(MeshError::InvalidPrimitive)
        );
        assert_eq!(*editor.document().mesh(), before);
        state.size = 2.0;
        state.location[0] = f32::NAN;
        assert_eq!(
            frame(&mut editor, &mut state),
            Err(MeshError::NonFinitePosition)
        );
        assert_eq!(*editor.document().mesh(), before);
        assert_eq!(editor.document().undo_count(), 1);
    }

    #[test]
    fn with_nothing_selected_the_values_are_for_the_next_shape() {
        let mut editor = Editor::new();
        let mut state = PanelState::default();
        assert_eq!(frame(&mut editor, &mut state), Ok(false));
        assert_eq!(editor.refresh_target(&mut state), Target::Nothing);

        editor.add(Shape::Cube, &state).unwrap();
        frame(&mut editor, &mut state).unwrap();
        deselect(&mut editor);
        state.size = 5.0;
        assert_eq!(frame(&mut editor, &mut state), Ok(false));
        assert_eq!(editor.refresh_target(&mut state), Target::Nothing);
        editor.add(Shape::Cube, &state).unwrap();
        let mesh = editor.document().mesh();
        assert_eq!(
            mesh.vertices()
                .filter(|(_, v)| v.position()[0] == 2.5)
                .count(),
            4
        );
    }

    #[test]
    fn several_shapes_selected_change_none_of_them() {
        let mut editor = Editor::new();
        let mut state = PanelState::default();
        editor.add(Shape::Cube, &state).unwrap();
        editor.add(Shape::Cone, &state).unwrap();
        select_all(&mut editor);
        assert_eq!(editor.refresh_target(&mut state), Target::Several);
        let before = editor.document().mesh().clone();
        state.size = 7.0;
        assert_eq!(frame(&mut editor, &mut state), Ok(false));
        assert_eq!(*editor.document().mesh(), before);
    }

    #[test]
    fn a_moved_shape_keeps_its_sliders_and_a_reshaped_one_loses_them() {
        let mut editor = Editor::new();
        let mut state = PanelState::default();
        editor.add(Shape::Cube, &state).unwrap();
        frame(&mut editor, &mut state).unwrap();
        let all = editor.document().mesh().selected_vertices();

        editor.begin("Move").unwrap();
        let mesh = editor.active_mesh().unwrap();
        for &vertex in &all {
            let p = mesh.vertex(vertex).unwrap().position();
            mesh.set_vertex_position(vertex, [p[0] + 3.0, p[1], p[2]])
                .unwrap();
        }
        editor.commit().unwrap();
        assert_eq!(
            editor.refresh_target(&mut state),
            Target::Shape(Shape::Cube)
        );
        assert_eq!(state.location, [3.0, 0.0, 0.0]);
        assert_eq!(
            frame(&mut editor, &mut state),
            Ok(false),
            "no rebuild from just moving"
        );

        editor.begin("Move").unwrap();
        let mesh = editor.active_mesh().unwrap();
        let p = mesh.vertex(all[0]).unwrap().position();
        mesh.set_vertex_position(all[0], [p[0], p[1], p[2] + 1.0])
            .unwrap();
        editor.commit().unwrap();
        assert_eq!(
            editor.refresh_target(&mut state),
            Target::Edited(Shape::Cube)
        );
        let before = editor.document().mesh().clone();
        state.size = 9.0;
        assert_eq!(frame(&mut editor, &mut state), Ok(false));
        assert_eq!(
            *editor.document().mesh(),
            before,
            "an edited shape must not be rebuilt"
        );
    }

    #[test]
    fn a_shape_with_something_attached_is_not_rebuilt() {
        let mut editor = Editor::new();
        let mut state = PanelState::default();
        editor.add(Shape::Cube, &state).unwrap();
        frame(&mut editor, &mut state).unwrap();
        let first = editor.document().mesh().selected_vertices()[0];
        editor
            .run("Attach", |mesh, _| {
                let loose = mesh.add_vertex([9.0; 3])?;
                mesh.add_edge(first, loose)?;
                Ok(())
            })
            .unwrap();
        assert_eq!(
            editor.refresh_target(&mut state),
            Target::Edited(Shape::Cube)
        );
    }

    #[test]
    fn modes_are_reported_and_part_mode_grows_to_the_whole_part() {
        let mut editor = Editor::new();
        let state = PanelState::default();
        assert_eq!(editor.mode_choice(), ModeChoice::Part);
        assert!(editor.whole_parts());
        assert_eq!(editor.drawing_mode(), SelectMode::Edge);
        assert!(!editor.set_mode(ModeChoice::Part));

        assert!(editor.set_mode(ModeChoice::Face));
        assert_eq!(editor.mode_choice(), ModeChoice::Face);
        assert_eq!(editor.drawing_mode(), SelectMode::Face);
        editor.add(Shape::Cube, &state).unwrap();
        let face = editor.document().mesh().faces().next().unwrap().0;
        editor
            .run("Select", |mesh, mode| {
                mesh.select_elements(mode, &[Element::Face(face)], SelectOp::Replace)
            })
            .unwrap();
        assert_eq!(selected(&editor), 4);
        assert!(editor.set_mode(ModeChoice::Part));
        assert_eq!(selected(&editor), 8);

        for (choice, mode) in [
            (ModeChoice::Vertex, SelectMode::Vertex),
            (ModeChoice::Edge, SelectMode::Edge),
        ] {
            assert!(editor.set_mode(choice));
            assert_eq!(editor.mode(), mode);
            assert!(!editor.whole_parts());
            assert_eq!(editor.mode_choice(), choice);
        }
    }

    #[test]
    fn switching_modes_keeps_a_whole_shape_selected() {
        let mut editor = Editor::new();
        let state = PanelState::default();
        for choice in [
            ModeChoice::Face,
            ModeChoice::Edge,
            ModeChoice::Vertex,
            ModeChoice::Part,
        ] {
            editor.set_mode(choice);
            editor.add(Shape::Cube, &state).unwrap();
            let mesh = editor.document().mesh();
            assert_eq!(mesh.selected_vertices().len(), 8, "{choice:?}");
            assert_eq!(
                mesh.faces().filter(|(_, f)| f.is_selected()).count(),
                6,
                "{choice:?}"
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
    fn many_changes_do_not_grow_history_or_records() {
        let mut editor = Editor::new();
        let mut state = PanelState {
            segments: 8,
            ..Default::default()
        };
        editor.add(Shape::Cylinder, &state).unwrap();
        frame(&mut editor, &mut state).unwrap();
        select_all(&mut editor);
        frame(&mut editor, &mut state).unwrap();
        let mut largest = 0;
        for i in 0..500 {
            state.segments = 8 + (i % 2);
            frame(&mut editor, &mut state).unwrap();
            if i > 2 {
                largest = largest.max(editor.document().history_bytes());
            }
        }
        assert!(largest < 64 * 1024, "{largest}");
        assert_eq!(
            editor.document().undo_count(),
            2,
            "selecting all again changed nothing"
        );
        assert_eq!(
            editor.records.len(),
            2,
            "the original and the current version"
        );
    }
}
