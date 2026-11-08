//! Parts and groups. A part is a connected piece of the mesh. Joining parts gives
//! their faces one group number, so they select, move and export as one part
//! without being connected. Separating sets the number back to 0.

use std::collections::{HashMap, HashSet};

use crate::error::MeshError;
use crate::id::{FaceId, VertexId};
use crate::mesh::Mesh;
use crate::selection::Element;

/// Connected pieces, joined further by face groups. Keyed by vertex slot.
struct PartFinder {
    parent: Vec<usize>,
}

impl PartFinder {
    fn root(&mut self, mut i: usize) -> usize {
        while self.parent[i] != i {
            self.parent[i] = self.parent[self.parent[i]];
            i = self.parent[i];
        }
        i
    }

    fn join(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.root(a), self.root(b));
        self.parent[ra] = rb;
    }
}

impl Mesh {
    fn part_finder(&self) -> PartFinder {
        let slots = self
            .vertices()
            .map(|(id, _)| id.index() as usize + 1)
            .max()
            .unwrap_or(0);
        let mut finder = PartFinder {
            parent: (0..slots).collect(),
        };
        for (_, edge) in self.edges() {
            let [a, b] = edge.vertices.map(|v| v.index() as usize);
            finder.join(a, b);
        }
        let mut first_of_group: HashMap<u16, usize> = HashMap::new();
        for (_, f) in self.faces() {
            if f.group == 0 {
                continue;
            }
            let vertex = self.corners[f.first_corner].vertex.index() as usize;
            let first = *first_of_group.entry(f.group).or_insert(vertex);
            finder.join(first, vertex);
        }
        finder
    }

    fn face_root(&self, finder: &mut PartFinder, face: FaceId) -> usize {
        let vertex: VertexId = self.corners[self.faces[face].first_corner].vertex;
        finder.root(vertex.index() as usize)
    }

    /// The faces of each part, joined groups counting as one part, in a fixed order.
    pub fn parts(&self) -> Vec<Vec<FaceId>> {
        let mut finder = self.part_finder();
        let mut order: Vec<usize> = Vec::new();
        let mut parts: HashMap<usize, Vec<FaceId>> = HashMap::new();
        for (face, _) in self.faces() {
            let root = self.face_root(&mut finder, face);
            parts
                .entry(root)
                .or_insert_with(|| {
                    order.push(root);
                    Vec::new()
                })
                .push(face);
        }
        order
            .into_iter()
            .map(|root| parts.remove(&root).unwrap_or_default())
            .collect()
    }

    /// All faces of the parts that touch these elements, joined groups included.
    pub fn linked_faces(&self, elements: &[Element]) -> Vec<FaceId> {
        let mut finder = self.part_finder();
        let mut roots: HashSet<usize> = HashSet::new();
        for element in elements {
            let vertex = match *element {
                Element::Vertex(v) if self.vertices.contains(v) => Some(v),
                Element::Edge(e) if self.edges.contains(e) => Some(self.edges[e].vertices[0]),
                Element::Face(f) if self.faces.contains(f) => {
                    Some(self.corners[self.faces[f].first_corner].vertex)
                }
                _ => None,
            };
            if let Some(v) = vertex {
                roots.insert(finder.root(v.index() as usize));
            }
        }
        let faces: Vec<FaceId> = self.faces().map(|(id, _)| id).collect();
        faces
            .into_iter()
            .filter(|&face| roots.contains(&self.face_root(&mut finder, face)))
            .collect()
    }

    /// A group number no face uses yet.
    pub(crate) fn next_group(&self) -> Result<u16, MeshError> {
        let highest = self.faces().map(|(_, f)| f.group).max().unwrap_or(0);
        highest.checked_add(1).ok_or(MeshError::IdsExhausted)
    }

    /// The parts with a selected face, each as its list of faces.
    fn selected_parts(&self) -> Vec<Vec<FaceId>> {
        self.parts()
            .into_iter()
            .filter(|part| part.iter().any(|&f| self.faces[f].selected))
            .collect()
    }

    /// Joins the parts with a selected face into one group, so they act as one part.
    /// Returns how many parts were joined. Needs at least two.
    pub fn join_selected_parts(&mut self) -> Result<usize, MeshError> {
        let parts = self.selected_parts();
        if parts.len() < 2 {
            return Err(MeshError::NeedsTwoParts);
        }
        let group = self.next_group()?;
        for &face in parts.iter().flatten() {
            self.faces[face].group = group;
        }
        Ok(parts.len())
    }

    /// Undoes joining for the parts with a selected face: each connected piece is its
    /// own part again. Returns how many pieces there are now.
    pub fn separate_selected_parts(&mut self) -> Result<usize, MeshError> {
        let parts = self.selected_parts();
        let grouped: Vec<FaceId> = parts
            .iter()
            .flatten()
            .copied()
            .filter(|&f| self.faces[f].group != 0)
            .collect();
        if grouped.is_empty() {
            return Err(MeshError::NotJoined);
        }
        for &face in &grouped {
            self.faces[face].group = 0;
        }
        let pieces = self.selected_parts().len();
        Ok(pieces)
    }
}

impl Mesh {
    /// One vertex of each grouped face, with its group, to find the groups again after
    /// an edit with [`restore_groups`](Self::restore_groups).
    pub(crate) fn group_marks(&self) -> Vec<(FaceId, VertexId, u16)> {
        self.faces()
            .filter(|(_, f)| f.group != 0)
            .map(|(id, f)| (id, self.corners[f.first_corner].vertex, f.group))
            .collect()
    }

    /// Tools that rebuild faces make new ones with no group. When that leaves a
    /// connected piece of a joined part with no grouped face at all, this puts the
    /// piece back in the group of a removed face that had one of its vertices. Faces
    /// that still exist keep whatever group the edit gave them, so separating works.
    pub(crate) fn restore_groups(&mut self, marks: &[(FaceId, VertexId, u16)]) {
        if marks.is_empty() {
            return;
        }
        let slots = self
            .vertices()
            .map(|(id, _)| id.index() as usize + 1)
            .max()
            .unwrap_or(0);
        let mut finder = PartFinder {
            parent: (0..slots).collect(),
        };
        for (_, edge) in self.edges() {
            let [a, b] = edge.vertices.map(|v| v.index() as usize);
            finder.join(a, b);
        }
        let mut grouped: HashSet<usize> = HashSet::new();
        let mut piece_faces: HashMap<usize, Vec<FaceId>> = HashMap::new();
        let faces: Vec<(FaceId, u16, VertexId)> = self
            .faces()
            .map(|(id, f)| (id, f.group, self.corners[f.first_corner].vertex))
            .collect();
        for (face, group, vertex) in faces {
            let root = finder.root(vertex.index() as usize);
            if group != 0 {
                grouped.insert(root);
            }
            piece_faces.entry(root).or_default().push(face);
        }
        for &(old_face, vertex, group) in marks {
            if self.faces.contains(old_face) || !self.vertices.contains(vertex) {
                continue;
            }
            let root = finder.root(vertex.index() as usize);
            if !grouped.insert(root) {
                continue;
            }
            for &face in piece_faces.get(&root).map(Vec::as_slice).unwrap_or(&[]) {
                self.faces[face].group = group;
            }
        }
    }
}
