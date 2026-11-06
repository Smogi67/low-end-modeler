//! Selection, and edits that follow it: linked, more, less, duplicate and mirror.
//!
//! Selection is stored on the elements, so it is part of undo. The active mode decides
//! which flags are the source of truth, and `flush` updates the rest from them. In all
//! modes a selected face has all its edges selected, and a selected edge has both its
//! vertices selected.

use std::collections::{HashMap, HashSet};

use crate::error::MeshError;
use crate::id::{EdgeId, FaceId, VertexId};
use crate::mesh::Mesh;

/// Which kind of element the user is selecting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectMode {
    /// Vertices are selected directly.
    Vertex,
    /// Edges are selected directly.
    Edge,
    /// Faces are selected directly.
    Face,
}

/// How new elements combine with what is already selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectOp {
    /// Only these become selected.
    Replace,
    /// These are added.
    Add,
    /// These are removed.
    Subtract,
}

/// Any element that can be selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Element {
    /// A vertex.
    Vertex(VertexId),
    /// An edge.
    Edge(EdgeId),
    /// A face.
    Face(FaceId),
}

impl Mesh {
    // Each setter writes only on a real change, so that undo saves only what changed.
    fn set_vertex_flag(&mut self, id: VertexId, selected: bool) {
        if self.vertices[id].selected != selected {
            self.vertices[id].selected = selected;
        }
    }
    fn set_edge_flag(&mut self, id: EdgeId, selected: bool) {
        if self.edges[id].selected != selected {
            self.edges[id].selected = selected;
        }
    }
    fn set_face_flag(&mut self, id: FaceId, selected: bool) {
        if self.faces[id].selected != selected {
            self.faces[id].selected = selected;
        }
    }

    fn all_ids(&self) -> (Vec<VertexId>, Vec<EdgeId>, Vec<FaceId>) {
        (
            self.vertices.iter().map(|(id, _)| id).collect(),
            self.edges.iter().map(|(id, _)| id).collect(),
            self.faces.iter().map(|(id, _)| id).collect(),
        )
    }

    /// Updates the other flags from the ones the mode treats as the source of truth.
    pub fn flush_selection(&mut self, mode: SelectMode) {
        let (vertices, edges, faces) = self.all_ids();
        match mode {
            SelectMode::Vertex => {
                for edge in edges {
                    let [a, b] = self.edges[edge].vertices;
                    let on = self.vertices[a].selected && self.vertices[b].selected;
                    self.set_edge_flag(edge, on);
                }
                for face in faces {
                    let on = self
                        .face_walk(face)
                        .all(|c| self.vertices[self.corners[c].vertex].selected);
                    self.set_face_flag(face, on);
                }
            }
            SelectMode::Edge => {
                for vertex in vertices {
                    let on = self.disk_walk(vertex).any(|e| self.edges[e].selected);
                    self.set_vertex_flag(vertex, on);
                }
                for face in faces {
                    let on = self
                        .face_walk(face)
                        .all(|c| self.edges[self.corners[c].edge].selected);
                    self.set_face_flag(face, on);
                }
            }
            SelectMode::Face => {
                for edge in edges {
                    let on = self
                        .radial_walk(edge)
                        .any(|c| self.faces[self.corners[c].face].selected);
                    self.set_edge_flag(edge, on);
                }
                for vertex in vertices {
                    let on = self
                        .corners_at(vertex)
                        .any(|c| self.faces[self.corners[c].face].selected);
                    self.set_vertex_flag(vertex, on);
                }
            }
        }
    }

    /// Clears the whole selection.
    pub fn deselect_all(&mut self) {
        let (vertices, edges, faces) = self.all_ids();
        vertices
            .into_iter()
            .for_each(|id| self.set_vertex_flag(id, false));
        edges
            .into_iter()
            .for_each(|id| self.set_edge_flag(id, false));
        faces
            .into_iter()
            .for_each(|id| self.set_face_flag(id, false));
    }

    /// Selects everything.
    pub fn select_all(&mut self) {
        let (vertices, edges, faces) = self.all_ids();
        vertices
            .into_iter()
            .for_each(|id| self.set_vertex_flag(id, true));
        edges
            .into_iter()
            .for_each(|id| self.set_edge_flag(id, true));
        faces
            .into_iter()
            .for_each(|id| self.set_face_flag(id, true));
    }

    /// Swaps selected and unselected, for elements of the mode's kind.
    pub fn invert_selection(&mut self, mode: SelectMode) {
        let (vertices, edges, faces) = self.all_ids();
        match mode {
            SelectMode::Vertex => {
                for id in vertices {
                    let on = !self.vertices[id].selected;
                    self.set_vertex_flag(id, on);
                }
            }
            SelectMode::Edge => {
                for id in edges {
                    let on = !self.edges[id].selected;
                    self.set_edge_flag(id, on);
                }
            }
            SelectMode::Face => {
                for id in faces {
                    let on = !self.faces[id].selected;
                    self.set_face_flag(id, on);
                }
            }
        }
        self.flush_selection(mode);
    }

    /// Changes the selection by these elements. Elements of another kind than the mode,
    /// and stale IDs, are refused before anything changes.
    pub fn select_elements(
        &mut self,
        mode: SelectMode,
        elements: &[Element],
        op: SelectOp,
    ) -> Result<(), MeshError> {
        for &element in elements {
            match (element, mode) {
                (Element::Vertex(id), SelectMode::Vertex) => self.require_vertex(id)?,
                (Element::Edge(id), SelectMode::Edge) => self.require_edge(id)?,
                (Element::Face(id), SelectMode::Face) => self.require_face(id)?,
                _ => return Err(MeshError::WrongSelectMode),
            }
        }
        if op == SelectOp::Replace {
            self.deselect_all();
        }
        let on = op != SelectOp::Subtract;
        for &element in elements {
            match element {
                Element::Vertex(id) => self.set_vertex_flag(id, on),
                Element::Edge(id) => self.set_edge_flag(id, on),
                Element::Face(id) => self.set_face_flag(id, on),
            }
        }
        self.flush_selection(mode);
        Ok(())
    }

    /// Selected vertices.
    pub fn selected_vertices(&self) -> Vec<VertexId> {
        self.vertices
            .iter()
            .filter(|(_, v)| v.selected)
            .map(|(id, _)| id)
            .collect()
    }

    /// The average position of the selected vertices, or `None` if there are none.
    pub fn selection_center(&self) -> Option<[f32; 3]> {
        let mut sum = [0.0f64; 3];
        let mut count = 0u32;
        for (_, vertex) in self.vertices.iter().filter(|(_, v)| v.selected) {
            for (total, value) in sum.iter_mut().zip(vertex.position) {
                *total += value as f64;
            }
            count += 1;
        }
        (count > 0).then(|| sum.map(|s| (s / count as f64) as f32))
    }

    fn set_from_vertex_set(&mut self, mode: SelectMode, set: &HashSet<VertexId>) {
        let (vertices, edges, faces) = self.all_ids();
        match mode {
            SelectMode::Vertex => {
                for id in vertices {
                    self.set_vertex_flag(id, set.contains(&id));
                }
            }
            SelectMode::Edge => {
                for id in edges {
                    let on = self.edges[id].vertices.iter().all(|v| set.contains(v));
                    self.set_edge_flag(id, on);
                }
            }
            SelectMode::Face => {
                for id in faces {
                    let on = self
                        .face_walk(id)
                        .all(|c| set.contains(&self.corners[c].vertex));
                    self.set_face_flag(id, on);
                }
            }
        }
        self.flush_selection(mode);
    }

    /// Vertices sharing an edge or a face with `vertex`.
    fn neighbors(&self, vertex: VertexId) -> impl Iterator<Item = VertexId> + '_ {
        let by_edge = self
            .disk_walk(vertex)
            .filter_map(move |e| self.edges[e].other_vertex(vertex));
        let by_face = self.corners_at(vertex).flat_map(move |c| {
            self.face_walk(self.corners[c].face)
                .map(|c| self.corners[c].vertex)
        });
        by_edge.chain(by_face).filter(move |&v| v != vertex)
    }

    /// Adds everything connected to the selection through edges.
    pub fn select_linked(&mut self, mode: SelectMode) {
        let mut stack = self.selected_vertices();
        let mut set: HashSet<VertexId> = stack.iter().copied().collect();
        while let Some(vertex) = stack.pop() {
            let next: Vec<VertexId> = self
                .disk_walk(vertex)
                .filter_map(|e| self.edges[e].other_vertex(vertex))
                .collect();
            for other in next {
                if set.insert(other) {
                    stack.push(other);
                }
            }
        }
        self.set_from_vertex_set(mode, &set);
    }

    /// Grows the selection by one step.
    pub fn select_more(&mut self, mode: SelectMode) {
        let selected = self.selected_vertices();
        let mut set: HashSet<VertexId> = selected.iter().copied().collect();
        for vertex in selected {
            set.extend(self.neighbors(vertex));
        }
        self.set_from_vertex_set(mode, &set);
    }

    /// Shrinks the selection by one step.
    pub fn select_less(&mut self, mode: SelectMode) {
        let selected: HashSet<VertexId> = self.selected_vertices().into_iter().collect();
        let kept: HashSet<VertexId> = selected
            .iter()
            .copied()
            .filter(|&v| self.neighbors(v).all(|n| selected.contains(&n)))
            .collect();
        self.set_from_vertex_set(mode, &kept);
    }

    /// Copies the selected parts. The copies become the selection, and the originals
    /// are deselected. A copied edge or face uses copied vertices only.
    pub fn duplicate_selected(&mut self, mode: SelectMode) -> Result<(), MeshError> {
        let mut map: HashMap<VertexId, VertexId> = HashMap::new();
        for vertex in self.selected_vertices() {
            let position = self.vertices[vertex].position;
            map.insert(vertex, self.add_vertex(position)?);
        }
        let edges: Vec<[VertexId; 2]> = self
            .edges
            .iter()
            .filter(|(_, e)| e.selected)
            .map(|(_, e)| e.vertices)
            .collect();
        let faces: Vec<Vec<VertexId>> = self
            .faces
            .iter()
            .filter(|(_, f)| f.selected)
            .map(|(id, _)| self.face_walk(id).map(|c| self.corners[c].vertex).collect())
            .collect();

        self.deselect_all();
        for vertex in map.values().copied().collect::<Vec<_>>() {
            self.set_vertex_flag(vertex, true);
        }
        let mut new_edges = Vec::with_capacity(edges.len());
        for [a, b] in edges {
            new_edges.push(self.add_edge(map[&a], map[&b])?);
        }
        for edge in new_edges {
            self.set_edge_flag(edge, true);
        }
        for face in faces {
            let cycle: Vec<VertexId> = face.iter().map(|v| map[v]).collect();
            let new_face = self.add_face(&cycle)?;
            self.set_face_flag(new_face, true);
        }
        self.flush_selection(mode);
        Ok(())
    }

    /// Reflects the selection across a plane through `pivot`, facing along `axis`
    /// (0, 1 or 2 for X, Y or Z). A reflection turns faces inside out, so fully selected
    /// faces have their winding reversed to keep them facing outward.
    pub fn mirror_selected(&mut self, axis: usize, pivot: [f32; 3]) -> Result<(), MeshError> {
        if axis > 2 {
            return Err(MeshError::InvalidAxis);
        }
        for vertex in self.selected_vertices() {
            let mut position = self.vertices[vertex].position;
            position[axis] = 2.0 * pivot[axis] - position[axis];
            self.set_vertex_position(vertex, position)?;
        }
        let faces: Vec<FaceId> = self
            .faces
            .iter()
            .filter(|(_, f)| f.selected)
            .map(|(id, _)| id)
            .collect();
        for face in faces {
            self.flip_face(face)?;
        }
        Ok(())
    }
}
