//! Low-level edits. Later stages build user-facing tools such as extrude and bevel
//! from these.
//!
//! Every public edit checks all of its preconditions before changing anything, so
//! an `Err` always means the mesh was not touched, with or without an open edit.

use crate::error::MeshError;
use crate::id::{CornerId, EdgeId, FaceId, VertexId};
use crate::mesh::{Corner, DiskLink, Edge, Face, Mesh, Vertex};

fn check_position(position: [f32; 3]) -> Result<(), MeshError> {
    if position.iter().all(|component| component.is_finite()) {
        Ok(())
    } else {
        Err(MeshError::NonFinitePosition)
    }
}

impl Mesh {
    /// Adds a loose vertex.
    pub fn add_vertex(&mut self, position: [f32; 3]) -> Result<VertexId, MeshError> {
        check_position(position)?;
        self.reserve_ids(1, 0, 0, 0)?;
        Ok(self.vertices.insert_with(|_| Vertex {
            position,
            first_edge: None,
            selected: false,
        }))
    }

    /// Moves a vertex.
    pub fn set_vertex_position(
        &mut self,
        vertex: VertexId,
        position: [f32; 3],
    ) -> Result<(), MeshError> {
        self.require_vertex(vertex)?;
        check_position(position)?;
        self.vertices[vertex].position = position;
        Ok(())
    }

    /// Adds a wire edge between two vertices.
    pub fn add_edge(&mut self, a: VertexId, b: VertexId) -> Result<EdgeId, MeshError> {
        self.require_vertex(a)?;
        self.require_vertex(b)?;
        if a == b {
            return Err(MeshError::SameVertex(a));
        }
        if let Some(existing) = self.find_edge(a, b) {
            return Err(MeshError::EdgeExists(existing));
        }
        self.reserve_ids(0, 1, 0, 0)?;
        Ok(self.link_new_edge(a, b))
    }

    /// Adds a face through `vertices`, in winding order. Existing edges are reused
    /// and missing ones are created.
    pub fn add_face(&mut self, vertices: &[VertexId]) -> Result<FaceId, MeshError> {
        for &vertex in vertices {
            self.require_vertex(vertex)?;
        }
        if vertices.len() < 3 {
            return Err(MeshError::FaceTooSmall {
                count: vertices.len(),
            });
        }
        let mut sorted = vertices.to_vec();
        sorted.sort_unstable();
        if let Some(pair) = sorted.windows(2).find(|pair| pair[0] == pair[1]) {
            return Err(MeshError::RepeatedVertex(pair[0]));
        }
        if let Some(existing) = self.find_face(vertices) {
            return Err(MeshError::FaceExists(existing));
        }
        let count = vertices.len();
        self.reserve_ids(0, count, count, 1)?;

        let face = self.faces.insert_with(|_| Face {
            first_corner: CornerId::dangling(),
            corner_count: count as u32,
            selected: false,
            group: 0,
        });
        let mut corners = Vec::with_capacity(count);
        for (i, &vertex) in vertices.iter().enumerate() {
            let next_vertex = vertices[(i + 1) % count];
            let edge = match self.find_edge(vertex, next_vertex) {
                Some(edge) => edge,
                None => self.link_new_edge(vertex, next_vertex),
            };
            corners.push(self.new_corner(vertex, edge, face));
        }
        for i in 0..count {
            self.link_corners(corners[i], corners[(i + 1) % count]);
        }
        self.faces[face].first_corner = corners[0];
        Ok(face)
    }

    /// Removes a face. Its edges and vertices stay.
    pub fn remove_face(&mut self, face: FaceId) -> Result<(), MeshError> {
        self.require_face(face)?;
        let corners: Vec<CornerId> = self.face_walk(face).collect();
        for corner in corners {
            self.radial_remove(corner);
            self.corners.remove(corner);
        }
        self.faces.remove(face);
        Ok(())
    }

    /// Removes an edge and every face using it. Its vertices stay.
    pub fn remove_edge(&mut self, edge: EdgeId) -> Result<(), MeshError> {
        self.require_edge(edge)?;
        let faces: Vec<FaceId> = self
            .radial_walk(edge)
            .map(|corner| self.corners[corner].face)
            .collect();
        for face in faces {
            self.remove_face(face)?;
        }
        let [a, b] = self.edges[edge].vertices;
        self.disk_remove(edge, a);
        self.disk_remove(edge, b);
        self.edges.remove(edge);
        Ok(())
    }

    /// Removes a vertex and every edge and face using it.
    pub fn remove_vertex(&mut self, vertex: VertexId) -> Result<(), MeshError> {
        self.require_vertex(vertex)?;
        let edges: Vec<EdgeId> = self.disk_walk(vertex).collect();
        for edge in edges {
            self.remove_edge(edge)?;
        }
        self.vertices.remove(vertex);
        Ok(())
    }

    /// Inserts a new vertex on an edge, at `factor` of the way from
    /// `edge.vertices()[0]` to `edge.vertices()[1]`. Every face on the edge gains
    /// a corner. Returns the new vertex and the new edge. The original edge keeps
    /// its ID and now ends at the new vertex.
    pub fn split_edge(
        &mut self,
        edge: EdgeId,
        factor: f32,
    ) -> Result<(VertexId, EdgeId), MeshError> {
        self.require_edge(edge)?;
        if !(0.0..=1.0).contains(&factor) {
            return Err(MeshError::InvalidFactor);
        }
        let [a, b] = self.edges[edge].vertices;
        let start = self.vertices[a].position;
        let end = self.vertices[b].position;
        let position = std::array::from_fn(|i| start[i] * (1.0 - factor) + end[i] * factor);
        check_position(position)?;
        let corners: Vec<CornerId> = self.radial_walk(edge).collect();
        self.reserve_ids(1, 1, corners.len(), 0)?;

        // New parts of a selected edge stay selected, so that a selected face keeps
        // all of its parts selected.
        let selected = self.edges[edge].selected;
        let middle = self.vertices.insert_with(|_| Vertex {
            position,
            first_edge: None,
            selected,
        });

        // The original edge becomes a-middle. A new edge covers middle-b.
        self.disk_remove(edge, b);
        self.edges[edge].vertices[1] = middle;
        self.disk_insert(edge, middle);
        let new_edge = self.link_new_edge(middle, b);
        self.edges[new_edge].selected = selected;

        for corner in corners {
            let Corner {
                vertex, next, face, ..
            } = self.corners[corner];
            let inserted_edge = if vertex == a {
                // a -> middle stays on the original edge.
                new_edge
            } else {
                // b -> middle moves to the new edge.
                self.radial_remove(corner);
                self.corners[corner].edge = new_edge;
                self.radial_insert(corner);
                edge
            };
            let inserted = self.new_corner(middle, inserted_edge, face);
            self.link_corners(corner, inserted);
            self.link_corners(inserted, next);
            self.faces[face].corner_count += 1;
        }
        Ok((middle, new_edge))
    }

    /// Splits a face in two by connecting two of its corners that are not
    /// neighbors. An existing edge between them is reused. Returns the new face
    /// and the connecting edge.
    ///
    /// The original face keeps the part running from `a` to `b` in winding order.
    /// The new face takes the part running from `b` to `a`.
    pub fn split_face(
        &mut self,
        face: FaceId,
        a: VertexId,
        b: VertexId,
    ) -> Result<(FaceId, EdgeId), MeshError> {
        self.require_face(face)?;
        self.require_vertex(a)?;
        self.require_vertex(b)?;
        if a == b {
            return Err(MeshError::SameVertex(a));
        }
        let corner_a = self.corner_in_face(face, a)?;
        let corner_b = self.corner_in_face(face, b)?;
        if self.corners[corner_a].next == corner_b || self.corners[corner_b].next == corner_a {
            return Err(MeshError::CornersAdjacent { face });
        }

        let kept_part = self.corners_between(corner_a, corner_b);
        let moved_part = self.corners_between(corner_b, corner_a);
        for (part, closing_vertex) in [(&kept_part, b), (&moved_part, a)] {
            let mut vertices: Vec<VertexId> = part
                .iter()
                .map(|&corner| self.corners[corner].vertex)
                .collect();
            vertices.push(closing_vertex);
            if let Some(existing) = self.find_face(&vertices) {
                return Err(MeshError::FaceExists(existing));
            }
        }
        self.reserve_ids(0, 1, 2, 1)?;

        let edge = match self.find_edge(a, b) {
            Some(edge) => edge,
            None => self.link_new_edge(a, b),
        };
        let selected = self.faces[face].selected;
        let group = self.faces[face].group;
        if selected {
            self.edges[edge].selected = true;
        }
        let new_face = self.faces.insert_with(|_| Face {
            first_corner: corner_b,
            corner_count: moved_part.len() as u32 + 1,
            selected,
            group,
        });

        let before_a = self.corners[corner_a].prev;
        let before_b = self.corners[corner_b].prev;
        let closes_kept = self.new_corner(b, edge, face);
        let closes_moved = self.new_corner(a, edge, new_face);
        self.link_corners(before_b, closes_kept);
        self.link_corners(closes_kept, corner_a);
        self.link_corners(before_a, closes_moved);
        self.link_corners(closes_moved, corner_b);

        for &corner in &moved_part {
            self.corners[corner].face = new_face;
        }
        self.faces[face].first_corner = corner_a;
        self.faces[face].corner_count = kept_part.len() as u32 + 1;
        Ok((new_face, edge))
    }

    /// Joins the two faces on an edge into one and removes the edge. Returns the
    /// face that remains; the other one no longer exists.
    ///
    /// The faces must share only this edge and must wind in opposite directions
    /// along it, which is what neighbors with consistent winding do.
    pub fn join_faces(&mut self, edge: EdgeId) -> Result<FaceId, MeshError> {
        self.require_edge(edge)?;
        let corners: Vec<CornerId> = self.radial_walk(edge).collect();
        let &[corner_1, corner_2] = corners.as_slice() else {
            return Err(MeshError::NotTwoFaces {
                edge,
                faces: corners.len(),
            });
        };
        let first = self.corners[corner_1];
        let second = self.corners[corner_2];
        if first.vertex == second.vertex {
            return Err(MeshError::InconsistentWinding(edge));
        }
        let (kept, removed) = (first.face, second.face);

        // The merged face's vertices: all of the kept face's, plus the removed
        // face's other than the edge's two ends, which the kept face already has.
        let endpoints = self.edges[edge].vertices;
        let mut outline: Vec<VertexId> = self
            .face_walk(kept)
            .map(|corner| self.corners[corner].vertex)
            .collect();
        for corner in self.face_walk(removed) {
            let vertex = self.corners[corner].vertex;
            if endpoints.contains(&vertex) {
                continue;
            }
            if self
                .corners_at(vertex)
                .any(|c| self.corners[c].face == kept)
            {
                return Err(MeshError::JoinWouldRepeatVertex(vertex));
            }
            outline.push(vertex);
        }
        if let Some(existing) = self.find_face(&outline) {
            return Err(MeshError::FaceExists(existing));
        }

        let moved: Vec<CornerId> = self.face_walk(removed).collect();
        for corner in moved {
            if corner != corner_2 {
                self.corners[corner].face = kept;
            }
        }
        self.radial_remove(corner_1);
        self.radial_remove(corner_2);
        self.link_corners(first.prev, second.next);
        self.link_corners(second.prev, first.next);
        self.corners.remove(corner_1);
        self.corners.remove(corner_2);

        let merged_count = self.faces[kept].corner_count + self.faces[removed].corner_count - 2;
        self.faces[kept].first_corner = first.next;
        self.faces[kept].corner_count = merged_count;
        self.faces[kept].selected &= self.faces[removed].selected;
        self.faces.remove(removed);

        let [a, b] = endpoints;
        self.disk_remove(edge, a);
        self.disk_remove(edge, b);
        self.edges.remove(edge);
        Ok(kept)
    }

    /// Reverses a face's winding, which flips its normal.
    pub fn flip_face(&mut self, face: FaceId) -> Result<(), MeshError> {
        self.require_face(face)?;
        let corners: Vec<CornerId> = self.face_walk(face).collect();
        // Once reversed, each corner runs along the edge its old previous corner used.
        let new_edges: Vec<EdgeId> = corners
            .iter()
            .map(|&corner| self.corners[self.corners[corner].prev].edge)
            .collect();
        for &corner in &corners {
            self.radial_remove(corner);
        }
        for (&corner, &edge) in corners.iter().zip(&new_edges) {
            let data = &mut self.corners[corner];
            std::mem::swap(&mut data.next, &mut data.prev);
            data.edge = edge;
            self.radial_insert(corner);
        }
        Ok(())
    }

    /// Refuses an edit up front if it could run out of IDs part-way through.
    fn reserve_ids(
        &self,
        vertices: usize,
        edges: usize,
        corners: usize,
        faces: usize,
    ) -> Result<(), MeshError> {
        if self.vertices.can_allocate(vertices)
            && self.edges.can_allocate(edges)
            && self.corners.can_allocate(corners)
            && self.faces.can_allocate(faces)
        {
            Ok(())
        } else {
            Err(MeshError::IdsExhausted)
        }
    }

    fn corner_in_face(&self, face: FaceId, vertex: VertexId) -> Result<CornerId, MeshError> {
        self.face_walk(face)
            .find(|&corner| self.corners[corner].vertex == vertex)
            .ok_or(MeshError::VertexNotInFace { vertex, face })
    }

    /// Corners from `from` up to but not including `to`, following the face.
    fn corners_between(&self, from: CornerId, to: CornerId) -> Vec<CornerId> {
        let mut part = Vec::new();
        let mut corner = from;
        while corner != to {
            part.push(corner);
            corner = self.corners[corner].next;
        }
        part
    }

    fn link_new_edge(&mut self, a: VertexId, b: VertexId) -> EdgeId {
        let edge = self.edges.insert_with(|id| Edge {
            vertices: [a, b],
            disk: [DiskLink { next: id, prev: id }; 2],
            first_corner: None,
            selected: false,
        });
        self.disk_insert(edge, a);
        self.disk_insert(edge, b);
        edge
    }

    fn disk_insert(&mut self, edge: EdgeId, vertex: VertexId) {
        let side = self.edges[edge].side_of(vertex);
        let Some(first) = self.vertices[vertex].first_edge else {
            self.edges[edge].disk[side] = DiskLink {
                next: edge,
                prev: edge,
            };
            self.vertices[vertex].first_edge = Some(edge);
            return;
        };
        let first_side = self.edges[first].side_of(vertex);
        let last = self.edges[first].disk[first_side].prev;
        let last_side = self.edges[last].side_of(vertex);
        self.edges[edge].disk[side] = DiskLink {
            next: first,
            prev: last,
        };
        self.edges[last].disk[last_side].next = edge;
        self.edges[first].disk[first_side].prev = edge;
    }

    fn disk_remove(&mut self, edge: EdgeId, vertex: VertexId) {
        let side = self.edges[edge].side_of(vertex);
        let DiskLink { next, prev } = self.edges[edge].disk[side];
        if next == edge {
            self.vertices[vertex].first_edge = None;
            return;
        }
        let next_side = self.edges[next].side_of(vertex);
        let prev_side = self.edges[prev].side_of(vertex);
        self.edges[next].disk[next_side].prev = prev;
        self.edges[prev].disk[prev_side].next = next;
        if self.vertices[vertex].first_edge == Some(edge) {
            self.vertices[vertex].first_edge = Some(next);
        }
    }

    /// Creates a corner and adds it to its edge's radial cycle. The caller then
    /// links it into its face cycle.
    pub(crate) fn new_corner(&mut self, vertex: VertexId, edge: EdgeId, face: FaceId) -> CornerId {
        let corner = self.corners.insert_with(|id| Corner {
            vertex,
            edge,
            face,
            next: id,
            prev: id,
            radial_next: id,
            radial_prev: id,
        });
        self.radial_insert(corner);
        corner
    }

    pub(crate) fn link_corners(&mut self, from: CornerId, to: CornerId) {
        self.corners[from].next = to;
        self.corners[to].prev = from;
    }

    pub(crate) fn radial_insert(&mut self, corner: CornerId) {
        let edge = self.corners[corner].edge;
        let Some(first) = self.edges[edge].first_corner else {
            self.corners[corner].radial_next = corner;
            self.corners[corner].radial_prev = corner;
            self.edges[edge].first_corner = Some(corner);
            return;
        };
        let last = self.corners[first].radial_prev;
        self.corners[corner].radial_next = first;
        self.corners[corner].radial_prev = last;
        self.corners[last].radial_next = corner;
        self.corners[first].radial_prev = corner;
    }

    pub(crate) fn radial_remove(&mut self, corner: CornerId) {
        let Corner {
            edge,
            radial_next,
            radial_prev,
            ..
        } = self.corners[corner];
        if radial_next == corner {
            self.edges[edge].first_corner = None;
            return;
        }
        self.corners[radial_next].radial_prev = radial_prev;
        self.corners[radial_prev].radial_next = radial_next;
        if self.edges[edge].first_corner == Some(corner) {
            self.edges[edge].first_corner = Some(radial_next);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A quad plus two spare vertices. Each array's counter is then set so it has
    /// room for exactly `room` more elements of that type.
    fn setup() -> (Mesh, FaceId, EdgeId, [VertexId; 6]) {
        let mut mesh = Mesh::new();
        let v = [
            mesh.add_vertex([0.0, 0.0, 0.0]).unwrap(),
            mesh.add_vertex([1.0, 0.0, 0.0]).unwrap(),
            mesh.add_vertex([1.0, 1.0, 0.0]).unwrap(),
            mesh.add_vertex([0.0, 1.0, 0.0]).unwrap(),
            mesh.add_vertex([2.0, 0.0, 0.0]).unwrap(),
            mesh.add_vertex([2.0, 1.0, 0.0]).unwrap(),
        ];
        let quad = mesh.add_face(&v[..4]).unwrap();
        let edge = mesh.find_edge(v[0], v[1]).unwrap();
        (mesh, quad, edge, v)
    }

    fn assert_refused_cleanly<T: std::fmt::Debug>(
        mesh: &mut Mesh,
        edit: impl FnOnce(&mut Mesh) -> Result<T, MeshError>,
    ) {
        let before = mesh.clone();
        assert_eq!(edit(mesh).err(), Some(MeshError::IdsExhausted));
        assert_eq!(*mesh, before);
        mesh.validate().unwrap();
    }

    #[test]
    fn running_out_of_vertex_ids_refuses_cleanly() {
        let (mut mesh, _, edge, _) = setup();
        mesh.vertices.test_set_next_generation(0);
        assert_refused_cleanly(&mut mesh, |m| m.add_vertex([0.0; 3]));
        assert_refused_cleanly(&mut mesh, |m| m.split_edge(edge, 0.5));
    }

    #[test]
    fn running_out_of_edge_ids_refuses_cleanly() {
        let (mut mesh, quad, edge, v) = setup();
        mesh.edges.test_set_next_generation(0);
        assert_refused_cleanly(&mut mesh, |m| m.add_edge(v[4], v[5]));
        assert_refused_cleanly(&mut mesh, |m| m.split_edge(edge, 0.5));
        assert_refused_cleanly(&mut mesh, |m| m.split_face(quad, v[0], v[2]));
        assert_refused_cleanly(&mut mesh, |m| m.add_face(&[v[0], v[4], v[5]]));
    }

    #[test]
    fn running_out_of_corner_ids_refuses_cleanly() {
        let (mut mesh, quad, edge, v) = setup();
        mesh.corners.test_set_next_generation(0);
        assert_refused_cleanly(&mut mesh, |m| m.split_edge(edge, 0.5));
        assert_refused_cleanly(&mut mesh, |m| m.split_face(quad, v[0], v[2]));
        assert_refused_cleanly(&mut mesh, |m| m.add_face(&[v[0], v[4], v[5]]));
    }

    #[test]
    fn running_out_of_face_ids_refuses_cleanly() {
        let (mut mesh, quad, _, v) = setup();
        mesh.faces.test_set_next_generation(0);
        assert_refused_cleanly(&mut mesh, |m| m.split_face(quad, v[0], v[2]));
        assert_refused_cleanly(&mut mesh, |m| m.add_face(&[v[0], v[4], v[5]]));
    }

    #[test]
    fn edits_that_create_nothing_still_work_when_ids_run_out() {
        let (mut mesh, quad, edge, v) = setup();
        mesh.vertices.test_set_next_generation(0);
        mesh.edges.test_set_next_generation(0);
        mesh.corners.test_set_next_generation(0);
        mesh.faces.test_set_next_generation(0);
        mesh.set_vertex_position(v[4], [5.0; 3]).unwrap();
        mesh.flip_face(quad).unwrap();
        mesh.remove_edge(edge).unwrap();
        mesh.remove_vertex(v[5]).unwrap();
        mesh.validate().unwrap();
    }

    #[test]
    fn a_partly_used_id_counter_allows_an_edit_only_if_all_of_it_fits() {
        let (mut mesh, _, _, v) = setup();
        // Room for 2 corners; a triangle needs 3.
        mesh.corners.test_set_next_generation(u32::MAX - 1);
        assert_refused_cleanly(&mut mesh, |m| m.add_face(&[v[0], v[4], v[5]]));
    }
}
