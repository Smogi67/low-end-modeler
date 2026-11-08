//! The mesh elements and read-only queries. Edits are in `ops.rs`.

use crate::error::MeshError;
use crate::id::{CornerId, EdgeId, FaceId, ModifierId, VertexId};
use crate::modifiers::ModifierEntry;
use crate::pool::{Pool, PoolDelta};

/// A point in space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vertex {
    pub(crate) position: [f32; 3],
    /// Any one edge from the disk cycle, or `None` for a loose vertex.
    pub(crate) first_edge: Option<EdgeId>,
    pub(crate) selected: bool,
}

impl Vertex {
    /// Position in model space.
    pub fn position(&self) -> [f32; 3] {
        self.position
    }

    /// One edge touching this vertex, or `None` for a loose vertex.
    pub fn first_edge(&self) -> Option<EdgeId> {
        self.first_edge
    }

    /// True if the vertex is selected.
    pub fn is_selected(&self) -> bool {
        self.selected
    }
}

/// Neighbors of an edge in the disk cycle around one of its vertices.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct DiskLink {
    pub(crate) next: EdgeId,
    pub(crate) prev: EdgeId,
}

/// A connection between two different vertices.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Edge {
    pub(crate) vertices: [VertexId; 2],
    /// `disk[i]` holds the links around `vertices[i]`.
    pub(crate) disk: [DiskLink; 2],
    /// Any one corner from the radial cycle, or `None` for a wire edge.
    pub(crate) first_corner: Option<CornerId>,
    pub(crate) selected: bool,
    /// Smooth shading stops at this edge, as at Blender's sharp edges.
    pub(crate) sharp: bool,
}

impl Edge {
    /// The two end vertices. Their order has no geometric meaning.
    pub fn vertices(&self) -> [VertexId; 2] {
        self.vertices
    }

    /// One face corner on this edge, or `None` for a wire edge.
    pub fn first_corner(&self) -> Option<CornerId> {
        self.first_corner
    }

    /// True if the edge is selected.
    pub fn is_selected(&self) -> bool {
        self.selected
    }

    /// True if smooth shading stops at this edge.
    pub fn is_sharp(&self) -> bool {
        self.sharp
    }

    /// True if `vertex` is one of the two ends.
    pub fn touches(&self, vertex: VertexId) -> bool {
        self.side(vertex).is_some()
    }

    /// The end that is not `vertex`, or `None` if `vertex` is not an end.
    pub fn other_vertex(&self, vertex: VertexId) -> Option<VertexId> {
        self.side(vertex).map(|side| self.vertices[1 - side])
    }

    pub(crate) fn side(&self, vertex: VertexId) -> Option<usize> {
        if self.vertices[0] == vertex {
            Some(0)
        } else if self.vertices[1] == vertex {
            Some(1)
        } else {
            None
        }
    }

    /// Internal version of [`Self::side`] for when the caller knows the answer exists.
    pub(crate) fn side_of(&self, vertex: VertexId) -> usize {
        self.side(vertex)
            .expect("internal mesh bug: edge does not touch vertex")
    }
}

/// One corner of one face (a half-edge). It runs from [`vertex`](Self::vertex)
/// along [`edge`](Self::edge) to the vertex of the [`next`](Self::next) corner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Corner {
    pub(crate) vertex: VertexId,
    pub(crate) edge: EdgeId,
    pub(crate) face: FaceId,
    pub(crate) next: CornerId,
    pub(crate) prev: CornerId,
    pub(crate) radial_next: CornerId,
    pub(crate) radial_prev: CornerId,
}

impl Corner {
    /// The vertex at this corner.
    pub fn vertex(&self) -> VertexId {
        self.vertex
    }
    /// The edge from this corner's vertex to the next corner's vertex.
    pub fn edge(&self) -> EdgeId {
        self.edge
    }
    /// The face this corner belongs to.
    pub fn face(&self) -> FaceId {
        self.face
    }
    /// The next corner of the same face, in winding order.
    pub fn next(&self) -> CornerId {
        self.next
    }
    /// The previous corner of the same face, in winding order.
    pub fn prev(&self) -> CornerId {
        self.prev
    }
    /// The next corner on the same edge. Equals this corner if only one face uses the edge.
    pub fn radial_next(&self) -> CornerId {
        self.radial_next
    }
    /// The previous corner on the same edge.
    pub fn radial_prev(&self) -> CornerId {
        self.radial_prev
    }
}

/// A polygon with three or more corners.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Face {
    pub(crate) first_corner: CornerId,
    pub(crate) corner_count: u32,
    pub(crate) selected: bool,
    /// The group of joined parts this face belongs to, or 0 for none.
    pub(crate) group: u16,
    /// Shading: 0 is flat; otherwise smooth across edges up to this many degrees.
    /// See [`Mesh::shading_normals`].
    pub(crate) smooth: u8,
}

impl Face {
    /// Any one corner of the face.
    pub fn first_corner(&self) -> CornerId {
        self.first_corner
    }
    /// True if the face is selected.
    pub fn is_selected(&self) -> bool {
        self.selected
    }
    /// The group of joined parts this face belongs to, or 0 for none. See
    /// [`Mesh::join_selected_parts`].
    pub fn group(&self) -> u16 {
        self.group
    }
    /// 0 for flat shading; otherwise how sharp an edge, in degrees, is still shaded
    /// smooth. 180 smooths everything.
    pub fn smooth(&self) -> u8 {
        self.smooth
    }
    /// How many corners, and therefore vertices and edges, the face has.
    pub fn corner_count(&self) -> usize {
        self.corner_count as usize
    }
}

/// An editable polygon mesh. See the [crate documentation](crate) for the design.
///
/// Comparing two meshes compares their contents and slot layout, and ignores
/// private bookkeeping that undo does not restore.
#[derive(Debug, Clone, PartialEq)]
pub struct Mesh {
    pub(crate) vertices: Pool<Vertex, VertexId>,
    pub(crate) edges: Pool<Edge, EdgeId>,
    pub(crate) corners: Pool<Corner, CornerId>,
    pub(crate) faces: Pool<Face, FaceId>,
    /// Live modifiers on parts, kept here so that undo covers them.
    pub(crate) modifiers: Pool<ModifierEntry, ModifierId>,
}

impl Default for Mesh {
    fn default() -> Self {
        Self::new()
    }
}

/// Saved difference between two states of a mesh.
#[derive(Debug)]
pub(crate) struct MeshDelta {
    vertices: PoolDelta<Vertex>,
    edges: PoolDelta<Edge>,
    corners: PoolDelta<Corner>,
    faces: PoolDelta<Face>,
    modifiers: PoolDelta<ModifierEntry>,
}

impl MeshDelta {
    pub(crate) fn is_empty(&self) -> bool {
        self.vertices.is_empty()
            && self.edges.is_empty()
            && self.corners.is_empty()
            && self.faces.is_empty()
            && self.modifiers.is_empty()
    }

    pub(crate) fn memory_bytes(&self) -> usize {
        self.vertices.memory_bytes()
            + self.edges.memory_bytes()
            + self.corners.memory_bytes()
            + self.faces.memory_bytes()
            + self.modifiers.memory_bytes()
    }
}

/// Walks a cycle from `start` until it returns to `start`. Assumes a valid mesh.
fn walk_cycle<I: Copy + Eq>(
    start: Option<I>,
    mut step: impl FnMut(I) -> I,
) -> impl Iterator<Item = I> {
    let mut current = start;
    std::iter::from_fn(move || {
        let item = current?;
        let next = step(item);
        current = if Some(next) == start {
            None
        } else {
            Some(next)
        };
        Some(item)
    })
}

impl Mesh {
    /// Creates an empty mesh.
    pub fn new() -> Self {
        Self {
            vertices: Pool::new(),
            edges: Pool::new(),
            corners: Pool::new(),
            faces: Pool::new(),
            modifiers: Pool::new(),
        }
    }

    /// Looks up a vertex. Returns `None` if the ID is stale.
    pub fn vertex(&self, id: VertexId) -> Option<&Vertex> {
        self.vertices.get(id)
    }
    /// Looks up an edge. Returns `None` if the ID is stale.
    pub fn edge(&self, id: EdgeId) -> Option<&Edge> {
        self.edges.get(id)
    }
    /// Looks up a corner. Returns `None` if the ID is stale.
    pub fn corner(&self, id: CornerId) -> Option<&Corner> {
        self.corners.get(id)
    }
    /// Looks up a face. Returns `None` if the ID is stale.
    pub fn face(&self, id: FaceId) -> Option<&Face> {
        self.faces.get(id)
    }

    /// Number of vertices.
    pub fn vertex_count(&self) -> usize {
        self.vertices.live_count()
    }
    /// Number of edges.
    pub fn edge_count(&self) -> usize {
        self.edges.live_count()
    }
    /// Number of corners.
    pub fn corner_count(&self) -> usize {
        self.corners.live_count()
    }
    /// Number of faces.
    pub fn face_count(&self) -> usize {
        self.faces.live_count()
    }

    /// All vertices.
    pub fn vertices(&self) -> impl Iterator<Item = (VertexId, &Vertex)> {
        self.vertices.iter()
    }
    /// All edges.
    pub fn edges(&self) -> impl Iterator<Item = (EdgeId, &Edge)> {
        self.edges.iter()
    }
    /// All corners.
    pub fn corners(&self) -> impl Iterator<Item = (CornerId, &Corner)> {
        self.corners.iter()
    }
    /// All faces.
    pub fn faces(&self) -> impl Iterator<Item = (FaceId, &Face)> {
        self.faces.iter()
    }

    /// The edges touching a vertex.
    pub fn vertex_edges(
        &self,
        vertex: VertexId,
    ) -> Result<impl Iterator<Item = EdgeId> + '_, MeshError> {
        self.require_vertex(vertex)?;
        Ok(self.disk_walk(vertex))
    }

    /// The face corners at a vertex, one for each face using it.
    pub fn vertex_corners(
        &self,
        vertex: VertexId,
    ) -> Result<impl Iterator<Item = CornerId> + '_, MeshError> {
        self.require_vertex(vertex)?;
        Ok(self.corners_at(vertex))
    }

    /// The faces using a vertex, each once.
    pub fn vertex_faces(
        &self,
        vertex: VertexId,
    ) -> Result<impl Iterator<Item = FaceId> + '_, MeshError> {
        self.require_vertex(vertex)?;
        Ok(self
            .corners_at(vertex)
            .map(|corner| self.corners[corner].face))
    }

    /// The face corners on an edge, one for each face using it.
    pub fn edge_corners(
        &self,
        edge: EdgeId,
    ) -> Result<impl Iterator<Item = CornerId> + '_, MeshError> {
        self.require_edge(edge)?;
        Ok(self.radial_walk(edge))
    }

    /// The faces using an edge, each once.
    pub fn edge_faces(&self, edge: EdgeId) -> Result<impl Iterator<Item = FaceId> + '_, MeshError> {
        self.require_edge(edge)?;
        Ok(self
            .radial_walk(edge)
            .map(|corner| self.corners[corner].face))
    }

    /// The corners of a face, in winding order.
    pub fn face_corners(
        &self,
        face: FaceId,
    ) -> Result<impl Iterator<Item = CornerId> + '_, MeshError> {
        self.require_face(face)?;
        Ok(self.face_walk(face))
    }

    /// The vertices of a face, in winding order.
    pub fn face_vertices(
        &self,
        face: FaceId,
    ) -> Result<impl Iterator<Item = VertexId> + '_, MeshError> {
        self.require_face(face)?;
        Ok(self
            .face_walk(face)
            .map(|corner| self.corners[corner].vertex))
    }

    /// The edges of a face, in winding order.
    pub fn face_edges(&self, face: FaceId) -> Result<impl Iterator<Item = EdgeId> + '_, MeshError> {
        self.require_face(face)?;
        Ok(self.face_walk(face).map(|corner| self.corners[corner].edge))
    }

    /// The edge joining two vertices, if there is one.
    pub fn edge_between(&self, a: VertexId, b: VertexId) -> Result<Option<EdgeId>, MeshError> {
        self.require_vertex(a)?;
        self.require_vertex(b)?;
        Ok(self.find_edge(a, b))
    }

    /// The face that uses exactly this set of vertices, in any order, if there is one.
    pub fn face_with_vertices(&self, vertices: &[VertexId]) -> Result<Option<FaceId>, MeshError> {
        for &vertex in vertices {
            self.require_vertex(vertex)?;
        }
        Ok(self.find_face(vertices))
    }

    /// Bytes reserved for this mesh's elements, not counting undo history.
    pub fn memory_bytes(&self) -> usize {
        self.vertices.memory_bytes()
            + self.edges.memory_bytes()
            + self.corners.memory_bytes()
            + self.faces.memory_bytes()
            + self.modifiers.memory_bytes()
    }

    pub(crate) fn require_vertex(&self, id: VertexId) -> Result<(), MeshError> {
        if self.vertices.contains(id) {
            Ok(())
        } else {
            Err(MeshError::StaleVertex(id))
        }
    }

    pub(crate) fn require_edge(&self, id: EdgeId) -> Result<(), MeshError> {
        if self.edges.contains(id) {
            Ok(())
        } else {
            Err(MeshError::StaleEdge(id))
        }
    }

    pub(crate) fn require_face(&self, id: FaceId) -> Result<(), MeshError> {
        if self.faces.contains(id) {
            Ok(())
        } else {
            Err(MeshError::StaleFace(id))
        }
    }

    pub(crate) fn disk_walk(&self, vertex: VertexId) -> impl Iterator<Item = EdgeId> + '_ {
        walk_cycle(self.vertices[vertex].first_edge, move |edge| {
            let edge = &self.edges[edge];
            edge.disk[edge.side_of(vertex)].next
        })
    }

    pub(crate) fn radial_walk(&self, edge: EdgeId) -> impl Iterator<Item = CornerId> + '_ {
        walk_cycle(self.edges[edge].first_corner, |corner| {
            self.corners[corner].radial_next
        })
    }

    pub(crate) fn face_walk(&self, face: FaceId) -> impl Iterator<Item = CornerId> + '_ {
        walk_cycle(Some(self.faces[face].first_corner), |corner| {
            self.corners[corner].next
        })
    }

    /// Each face corner at a vertex has exactly one edge leaving it, so visiting
    /// the corners on each edge that start at this vertex finds each face once.
    pub(crate) fn corners_at(&self, vertex: VertexId) -> impl Iterator<Item = CornerId> + '_ {
        self.disk_walk(vertex).flat_map(move |edge| {
            self.radial_walk(edge)
                .filter(move |&corner| self.corners[corner].vertex == vertex)
        })
    }

    pub(crate) fn find_edge(&self, a: VertexId, b: VertexId) -> Option<EdgeId> {
        self.disk_walk(a)
            .find(|&edge| self.edges[edge].other_vertex(a) == Some(b))
    }

    /// See [`Self::face_with_vertices`]. Assumes all IDs are live.
    pub(crate) fn find_face(&self, vertices: &[VertexId]) -> Option<FaceId> {
        let &first = vertices.first()?;
        let mut sorted = vertices.to_vec();
        sorted.sort_unstable();
        self.corners_at(first)
            .map(|corner| self.corners[corner].face)
            .find(|&face| {
                self.faces[face].corner_count() == vertices.len()
                    && self
                        .face_walk(face)
                        .all(|corner| sorted.binary_search(&self.corners[corner].vertex).is_ok())
            })
    }

    /// Starts saving old values of everything that changes.
    pub(crate) fn begin_journal(&mut self) {
        self.vertices.begin_journal();
        self.edges.begin_journal();
        self.corners.begin_journal();
        self.faces.begin_journal();
        self.modifiers.begin_journal();
    }

    /// Stops saving and returns what changed.
    pub(crate) fn finish_journal(&mut self) -> MeshDelta {
        MeshDelta {
            vertices: self.vertices.finish_journal(),
            edges: self.edges.finish_journal(),
            corners: self.corners.finish_journal(),
            faces: self.faces.finish_journal(),
            modifiers: self.modifiers.finish_journal(),
        }
    }

    /// Stops saving and restores the state from before the journal started.
    pub(crate) fn rollback_journal(&mut self) {
        self.vertices.rollback_journal();
        self.edges.rollback_journal();
        self.corners.rollback_journal();
        self.faces.rollback_journal();
        self.modifiers.rollback_journal();
    }

    /// Swaps the mesh with the other state stored in the delta.
    pub(crate) fn swap_delta(&mut self, delta: &mut MeshDelta) {
        self.vertices.swap_delta(&mut delta.vertices);
        self.edges.swap_delta(&mut delta.edges);
        self.corners.swap_delta(&mut delta.corners);
        self.faces.swap_delta(&mut delta.faces);
        self.modifiers.swap_delta(&mut delta.modifiers);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A change touching every array, so each one's cost must be counted.
    #[test]
    fn memory_totals_add_up_all_arrays() {
        let mut mesh = Mesh::new();
        let v: Vec<VertexId> = (0..3)
            .map(|i| mesh.add_vertex([i as f32; 3]).unwrap())
            .collect();
        mesh.add_face(&v).unwrap();
        assert_eq!(
            mesh.memory_bytes(),
            mesh.vertices.memory_bytes()
                + mesh.edges.memory_bytes()
                + mesh.corners.memory_bytes()
                + mesh.faces.memory_bytes()
                + mesh.modifiers.memory_bytes()
        );

        mesh.begin_journal();
        let w: Vec<VertexId> = (0..3)
            .map(|i| mesh.add_vertex([9.0 + i as f32; 3]).unwrap())
            .collect();
        mesh.add_face(&w).unwrap();
        mesh.add_modifier_unchecked(1, crate::Modifier::mirror())
            .unwrap();
        let delta = mesh.finish_journal();
        assert_eq!(
            delta.memory_bytes(),
            delta.vertices.memory_bytes()
                + delta.edges.memory_bytes()
                + delta.corners.memory_bytes()
                + delta.faces.memory_bytes()
                + delta.modifiers.memory_bytes()
        );
    }
}
