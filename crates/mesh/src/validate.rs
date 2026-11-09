//! Mesh consistency checking.
//!
//! [`Mesh::validate`] confirms the following. It never panics, even on badly
//! damaged data, because a checker that crashes on the damage it exists to find
//! is no use.
//!
//! **Storage:** each array's free list and live count are correct, and every
//! generation number is unique and has been issued.
//!
//! **Vertices:** positions are finite. Each vertex's disk cycle closes, its
//! forward and backward links agree, and it contains only edges touching that
//! vertex. Every edge appears in the disk cycle at both of its ends.
//!
//! **Edges:** an edge joins two different live vertices, and at most one edge
//! joins any pair. Each radial cycle closes, its links agree, and it contains
//! only corners on that edge. Every corner is in exactly one radial cycle.
//!
//! **Faces:** a face has at least 3 corners and its stored count is correct. Each
//! face cycle closes and its links agree. Each corner names its face, and its edge
//! joins its vertex to the next corner's vertex. No vertex repeats within a face.
//! No two faces use the same set of vertices. Every corner is in exactly one face.
//!
//! **Selection:** a selected face has all its edges selected, and a selected edge has
//! both its vertices selected.
//!
//! Winding may differ between neighbors, and it is deliberately not checked.

use std::collections::HashSet;
use std::fmt;

use crate::mesh::Mesh;

/// What [`Mesh::validate`] found wrong. Any such error means a bug in the mesh code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    message: String,
}

impl ValidationError {
    /// A description of the first problem found.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid mesh: {}", self.message)
    }
}

impl std::error::Error for ValidationError {}

impl From<String> for ValidationError {
    fn from(message: String) -> Self {
        Self { message }
    }
}

macro_rules! bail {
    ($($arg:tt)*) => {
        return Err(ValidationError { message: format!($($arg)*) })
    };
}

impl Mesh {
    /// Checks all the rules listed in the [module documentation](self).
    ///
    /// This visits every element, so it is meant for tests and debug builds. Edits
    /// in debug builds run it automatically when they finish.
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.vertices.check_integrity("vertex")?;
        self.edges.check_integrity("edge")?;
        self.corners.check_integrity("corner")?;
        self.faces.check_integrity("face")?;
        self.check_edge_endpoints()?;
        self.check_vertices_and_disk_cycles()?;
        self.check_radial_cycles()?;
        self.check_faces()?;
        self.check_selection()?;
        self.materials.check_integrity("material")?;
        for (id, face) in self.faces.iter() {
            if let Some(material) = face.material
                && !self.materials.contains(material)
            {
                bail!("face {id} uses removed material {material}");
            }
        }
        Ok(())
    }

    fn check_selection(&self) -> Result<(), ValidationError> {
        for (id, edge) in self.edges.iter() {
            if edge.selected && !edge.vertices.iter().all(|&v| self.vertices[v].selected) {
                bail!("selected edge {id} has an unselected vertex");
            }
        }
        for (id, face) in self.faces.iter() {
            if face.selected
                && !self
                    .face_walk(id)
                    .all(|c| self.edges[self.corners[c].edge].selected)
            {
                bail!("selected face {id} has an unselected edge");
            }
        }
        Ok(())
    }

    fn check_edge_endpoints(&self) -> Result<(), ValidationError> {
        let mut pairs = HashSet::with_capacity(self.edges.live_count());
        for (id, edge) in self.edges.iter() {
            let [a, b] = edge.vertices;
            if a == b {
                bail!("edge {id} joins vertex {a} to itself");
            }
            for vertex in [a, b] {
                if !self.vertices.contains(vertex) {
                    bail!("edge {id} uses stale vertex {vertex}");
                }
            }
            if !pairs.insert((a.min(b), a.max(b))) {
                bail!("edge {id} duplicates another edge between {a} and {b}");
            }
        }
        Ok(())
    }

    fn check_vertices_and_disk_cycles(&self) -> Result<(), ValidationError> {
        let edge_total = self.edges.live_count();
        let mut disk_entries = 0usize;
        for (vertex_id, vertex) in self.vertices.iter() {
            if !vertex.position.iter().all(|c| c.is_finite()) {
                bail!("vertex {vertex_id} has a non-finite position");
            }
            let Some(first) = vertex.first_edge else {
                continue;
            };
            let mut current = first;
            let mut steps = 0usize;
            loop {
                let Some(edge) = self.edges.get(current) else {
                    bail!("disk cycle of vertex {vertex_id} reaches stale edge {current}");
                };
                let Some(side) = edge.side(vertex_id) else {
                    bail!(
                        "disk cycle of vertex {vertex_id} reaches edge {current}, which does not touch it"
                    );
                };
                let next = edge.disk[side].next;
                let Some(next_edge) = self.edges.get(next) else {
                    bail!("disk cycle of vertex {vertex_id} reaches stale edge {next}");
                };
                let Some(next_side) = next_edge.side(vertex_id) else {
                    bail!(
                        "disk cycle of vertex {vertex_id} reaches edge {next}, which does not touch it"
                    );
                };
                if next_edge.disk[next_side].prev != current {
                    bail!("disk links of vertex {vertex_id} disagree between {current} and {next}");
                }
                steps += 1;
                if steps > edge_total {
                    bail!("disk cycle of vertex {vertex_id} does not close");
                }
                current = next;
                if current == first {
                    break;
                }
            }
            disk_entries += steps;
        }
        if disk_entries != edge_total * 2 {
            bail!(
                "disk cycles hold {disk_entries} entries, but {edge_total} edges need {}",
                edge_total * 2
            );
        }
        Ok(())
    }

    fn check_radial_cycles(&self) -> Result<(), ValidationError> {
        let corner_total = self.corners.live_count();
        let mut radial_entries = 0usize;
        for (edge_id, edge) in self.edges.iter() {
            let Some(first) = edge.first_corner else {
                continue;
            };
            let mut current = first;
            let mut steps = 0usize;
            loop {
                let Some(corner) = self.corners.get(current) else {
                    bail!("radial cycle of edge {edge_id} reaches stale corner {current}");
                };
                if corner.edge != edge_id {
                    bail!(
                        "radial cycle of edge {edge_id} reaches corner {current}, which names edge {}",
                        corner.edge
                    );
                }
                let next = corner.radial_next;
                let Some(next_corner) = self.corners.get(next) else {
                    bail!("radial cycle of edge {edge_id} reaches stale corner {next}");
                };
                if next_corner.radial_prev != current {
                    bail!("radial links of edge {edge_id} disagree between {current} and {next}");
                }
                steps += 1;
                if steps > corner_total {
                    bail!("radial cycle of edge {edge_id} does not close");
                }
                current = next;
                if current == first {
                    break;
                }
            }
            radial_entries += steps;
        }
        if radial_entries != corner_total {
            bail!(
                "radial cycles hold {radial_entries} entries, but there are {corner_total} corners"
            );
        }
        Ok(())
    }

    fn check_faces(&self) -> Result<(), ValidationError> {
        let corner_total = self.corners.live_count();
        let mut face_entries = 0usize;
        let mut vertex_sets = HashSet::with_capacity(self.faces.live_count());
        for (face_id, face) in self.faces.iter() {
            let count = face.corner_count();
            if count < 3 {
                bail!("face {face_id} has only {count} corners");
            }
            let mut vertices = Vec::with_capacity(count.min(corner_total));
            let mut current = face.first_corner;
            for step in 0..count {
                let Some(corner) = self.corners.get(current) else {
                    bail!("face {face_id} reaches stale corner {current}");
                };
                if corner.face != face_id {
                    bail!(
                        "face {face_id} reaches corner {current}, which names face {}",
                        corner.face
                    );
                }
                if !self.vertices.contains(corner.vertex) {
                    bail!("corner {current} uses stale vertex {}", corner.vertex);
                }
                let Some(edge) = self.edges.get(corner.edge) else {
                    bail!("corner {current} uses stale edge {}", corner.edge);
                };
                let Some(next) = self.corners.get(corner.next) else {
                    bail!("face {face_id} reaches stale corner {}", corner.next);
                };
                if next.prev != current {
                    bail!(
                        "face links disagree between corners {current} and {}",
                        corner.next
                    );
                }
                if edge.other_vertex(corner.vertex) != Some(next.vertex) {
                    bail!(
                        "corner {current} says edge {} joins {} and {}, but it does not",
                        corner.edge,
                        corner.vertex,
                        next.vertex
                    );
                }
                vertices.push(corner.vertex);
                current = corner.next;
                let closed = current == face.first_corner;
                if closed != (step + 1 == count) {
                    bail!("face {face_id} says it has {count} corners, but its cycle disagrees");
                }
            }
            vertices.sort_unstable();
            if let Some(pair) = vertices.windows(2).find(|pair| pair[0] == pair[1]) {
                bail!("face {face_id} repeats vertex {}", pair[0]);
            }
            if !vertex_sets.insert(vertices) {
                bail!("face {face_id} uses the same vertices as another face");
            }
            face_entries += count;
        }
        if face_entries != corner_total {
            bail!("face cycles hold {face_entries} entries, but there are {corner_total} corners");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::id::{CornerId, EdgeId, FaceId, VertexId};
    use crate::mesh::{DiskLink, Face, Mesh};

    /// Everything the corruption tests need: three faces on edge `shared`, a wire
    /// edge, a loose vertex, and IDs of deleted elements.
    struct Fixture {
        mesh: Mesh,
        v: [VertexId; 6],
        shared: EdgeId,
        wire: EdgeId,
        face: FaceId,
        corner: CornerId,
        dead_vertex: VertexId,
        dead_edge: EdgeId,
        dead_corner: CornerId,
    }

    fn fixture() -> Fixture {
        let mut mesh = Mesh::new();
        let v = [
            mesh.add_vertex([0.0, 0.0, 0.0]).unwrap(),
            mesh.add_vertex([1.0, 0.0, 0.0]).unwrap(),
            mesh.add_vertex([0.0, 1.0, 0.0]).unwrap(),
            mesh.add_vertex([1.0, 1.0, 0.0]).unwrap(),
            mesh.add_vertex([0.0, 0.0, 1.0]).unwrap(),
            mesh.add_vertex([5.0, 5.0, 5.0]).unwrap(),
        ];
        let face = mesh.add_face(&[v[0], v[1], v[2]]).unwrap();
        mesh.add_face(&[v[1], v[0], v[3]]).unwrap();
        mesh.add_face(&[v[0], v[1], v[4]]).unwrap();
        let shared = mesh.find_edge(v[0], v[1]).unwrap();
        let wire = mesh.add_edge(v[2], v[3]).unwrap();
        let corner = mesh.faces[face].first_corner;

        let dead_vertex = mesh.add_vertex([9.0, 9.0, 9.0]).unwrap();
        let other = mesh.add_vertex([9.0, 9.0, 8.0]).unwrap();
        let third = mesh.add_vertex([9.0, 8.0, 8.0]).unwrap();
        let dead_face = mesh.add_face(&[dead_vertex, other, third]).unwrap();
        let dead_corner = mesh.faces[dead_face].first_corner;
        let dead_edge = mesh.find_edge(dead_vertex, other).unwrap();
        mesh.remove_vertex(other).unwrap();
        mesh.remove_vertex(third).unwrap();
        mesh.remove_vertex(dead_vertex).unwrap();

        mesh.validate().expect("the fixture must start valid");
        Fixture {
            mesh,
            v,
            shared,
            wire,
            face,
            corner,
            dead_vertex,
            dead_edge,
            dead_corner,
        }
    }

    fn assert_detected(expected: &str, corrupt: impl FnOnce(&mut Fixture)) {
        let mut fixture = fixture();
        corrupt(&mut fixture);
        assert_message(&fixture.mesh, expected);
    }

    fn assert_message(mesh: &Mesh, expected: &str) {
        match mesh.validate() {
            Ok(()) => panic!("corruption not detected, wanted: {expected}"),
            Err(error) => assert!(
                error.message().contains(expected),
                "wanted {expected:?}, got {:?}",
                error.message()
            ),
        }
    }

    /// Creates a face straight from corner data, skipping `add_face` and its checks.
    fn face_by_hand(mesh: &mut Mesh, corners: &[(VertexId, EdgeId)]) -> FaceId {
        let face = mesh.faces.insert_with(|_| Face {
            first_corner: CornerId::dangling(),
            corner_count: corners.len() as u32,
            selected: false,
            group: 0,
            smooth: 0,
            material: None,
        });
        let ids: Vec<CornerId> = corners
            .iter()
            .map(|&(vertex, edge)| mesh.new_corner(vertex, edge, face))
            .collect();
        for i in 0..ids.len() {
            mesh.link_corners(ids[i], ids[(i + 1) % ids.len()]);
        }
        mesh.faces[face].first_corner = ids[0];
        face
    }

    #[test]
    fn detects_non_finite_positions() {
        assert_detected("non-finite", |f| {
            f.mesh.vertices[f.v[0]].position[1] = f32::NAN
        });
        assert_detected("non-finite", |f| {
            f.mesh.vertices[f.v[0]].position[2] = f32::INFINITY;
        });
    }

    #[test]
    fn detects_bad_edge_endpoints() {
        assert_detected("to itself", |f| f.mesh.edges[f.wire].vertices[1] = f.v[2]);
        assert_detected("stale vertex", |f| {
            f.mesh.edges[f.wire].vertices[1] = f.dead_vertex;
        });
        assert_detected("duplicates", |f| {
            f.mesh.edges[f.wire].vertices = [f.v[0], f.v[1]];
        });
        // Same pair written in the other order must count as a duplicate too.
        assert_detected("duplicates", |f| {
            f.mesh.edges[f.wire].vertices = [f.v[1], f.v[0]];
        });
    }

    #[test]
    fn detects_bad_disk_cycles() {
        assert_detected("stale edge", |f| {
            f.mesh.vertices[f.v[5]].first_edge = Some(f.dead_edge);
        });
        assert_detected("does not touch", |f| {
            f.mesh.vertices[f.v[5]].first_edge = Some(f.shared);
        });
        assert_detected("stale edge", |f| {
            let side = f.mesh.edges[f.wire].side_of(f.v[2]);
            f.mesh.edges[f.wire].disk[side].next = f.dead_edge;
        });
        assert_detected("does not touch", |f| {
            let side = f.mesh.edges[f.shared].side_of(f.v[0]);
            f.mesh.edges[f.shared].disk[side].next = f.wire;
        });
        assert_detected("disk links", |f| {
            let side = f.mesh.edges[f.shared].side_of(f.v[0]);
            let next = f.mesh.edges[f.shared].disk[side].next;
            let next_side = f.mesh.edges[next].side_of(f.v[0]);
            f.mesh.edges[next].disk[next_side].prev = next;
        });
        // Edges missing from a vertex's disk cycle.
        assert_detected("disk cycles hold", |f| {
            f.mesh.vertices[f.v[2]].first_edge = None;
        });
    }

    #[test]
    fn detects_bad_radial_cycles() {
        assert_detected("stale corner", |f| {
            f.mesh.edges[f.wire].first_corner = Some(f.dead_corner);
        });
        assert_detected("names edge", |f| {
            f.mesh.edges[f.wire].first_corner = Some(f.corner);
        });
        assert_detected("stale corner", |f| {
            let first = f.mesh.edges[f.shared].first_corner.unwrap();
            f.mesh.corners[first].radial_next = f.dead_corner;
        });
        assert_detected("radial links", |f| {
            let first = f.mesh.edges[f.shared].first_corner.unwrap();
            let next = f.mesh.corners[first].radial_next;
            f.mesh.corners[next].radial_prev = next;
        });
        // A corner that says it uses a dead edge is still found through the radial
        // cycle of the live edge it is really on.
        assert_detected("names edge", |f| {
            f.mesh.corners[f.corner].edge = f.dead_edge
        });
        // Corners missing from their edge's radial cycle.
        assert_detected("radial cycles hold", |f| {
            f.mesh.edges[f.shared].first_corner = None;
        });
    }

    #[test]
    fn detects_bad_faces() {
        assert_detected("only 2 corners", |f| f.mesh.faces[f.face].corner_count = 2);
        assert_detected("cycle disagrees", |f| f.mesh.faces[f.face].corner_count = 4);
        assert_detected("stale corner", |f| {
            f.mesh.faces[f.face].first_corner = f.dead_corner;
        });
        assert_detected("names face", |f| {
            let other = f
                .mesh
                .faces
                .iter()
                .map(|(id, _)| id)
                .find(|&id| id != f.face)
                .unwrap();
            f.mesh.corners[f.corner].face = other;
        });
        assert_detected("stale vertex", |f| {
            f.mesh.corners[f.corner].vertex = f.dead_vertex;
        });
        assert_detected("stale corner", |f| {
            f.mesh.corners[f.corner].next = f.dead_corner;
        });
        assert_detected("face links", |f| {
            let next = f.mesh.corners[f.corner].next;
            f.mesh.corners[next].prev = next;
        });
        assert_detected("but it does not", |f| {
            f.mesh.corners[f.corner].vertex = f.v[5];
        });
        // A corner correctly filed under an edge that does not lead to the next corner.
        assert_detected("but it does not", |f| {
            f.mesh.radial_remove(f.corner);
            f.mesh.corners[f.corner].edge = f.wire;
            f.mesh.radial_insert(f.corner);
        });
    }

    #[test]
    fn detects_selection_that_is_not_carried_down() {
        assert_detected("unselected vertex", |f| {
            f.mesh.edges[f.wire].selected = true
        });
        assert_detected("unselected edge", |f| f.mesh.faces[f.face].selected = true);
    }

    #[test]
    fn detects_a_repeated_vertex_in_a_face() {
        let mut mesh = Mesh::new();
        let a = mesh.add_vertex([0.0, 0.0, 0.0]).unwrap();
        let b = mesh.add_vertex([1.0, 0.0, 0.0]).unwrap();
        let ab = mesh.add_edge(a, b).unwrap();
        // Goes a, b, a, b over one edge: every link agrees, but vertices repeat.
        face_by_hand(&mut mesh, &[(a, ab), (b, ab), (a, ab), (b, ab)]);
        assert_message(&mesh, "repeats vertex");
    }

    #[test]
    fn detects_faces_with_the_same_vertices() {
        let mut mesh = Mesh::new();
        let a = mesh.add_vertex([0.0, 0.0, 0.0]).unwrap();
        let b = mesh.add_vertex([1.0, 0.0, 0.0]).unwrap();
        let c = mesh.add_vertex([0.0, 1.0, 0.0]).unwrap();
        mesh.add_face(&[a, b, c]).unwrap();
        let ab = mesh.find_edge(a, b).unwrap();
        let bc = mesh.find_edge(b, c).unwrap();
        let ca = mesh.find_edge(c, a).unwrap();
        // Opposite winding, same vertex set: this still counts as a duplicate.
        face_by_hand(&mut mesh, &[(a, ca), (c, bc), (b, ab)]);
        assert_message(&mesh, "same vertices");
    }

    #[test]
    fn detects_a_corner_outside_every_face() {
        let mut fixture = fixture();
        fixture
            .mesh
            .new_corner(fixture.v[2], fixture.wire, fixture.face);
        assert_message(&fixture.mesh, "face cycles hold");
    }

    #[test]
    fn checks_the_storage_of_all_four_arrays() {
        let mut f = fixture();
        f.mesh.vertices.test_set_next_generation(1);
        assert_message(&f.mesh, "vertex slot");
        let mut f = fixture();
        f.mesh.edges.test_set_next_generation(1);
        assert_message(&f.mesh, "edge slot");
        let mut f = fixture();
        f.mesh.corners.test_set_next_generation(1);
        assert_message(&f.mesh, "corner slot");
        let mut f = fixture();
        f.mesh.faces.test_set_next_generation(1);
        assert_message(&f.mesh, "face slot");
    }

    #[test]
    fn damaged_cycles_end_with_an_error_and_do_not_hang() {
        let mut mesh = Mesh::new();
        let a = mesh.add_vertex([0.0, 0.0, 0.0]).unwrap();
        let b = mesh.add_vertex([1.0, 0.0, 0.0]).unwrap();
        let c = mesh.add_vertex([2.0, 0.0, 0.0]).unwrap();
        let ab = mesh.add_edge(a, b).unwrap();
        let ac = mesh.add_edge(a, c).unwrap();
        let side = mesh.edges[ac].side_of(a);
        mesh.edges[ac].disk[side] = DiskLink { next: ac, prev: ac };
        let side = mesh.edges[ab].side_of(a);
        mesh.edges[ab].disk[side].next = ac;
        assert!(mesh.validate().is_err());

        let mut mesh = Mesh::new();
        let v: Vec<_> = (0..4)
            .map(|i| mesh.add_vertex([i as f32, 0.0, 0.0]).unwrap())
            .collect();
        mesh.add_face(&[v[0], v[1], v[2]]).unwrap();
        mesh.add_face(&[v[1], v[0], v[3]]).unwrap();
        let edge = mesh.find_edge(v[0], v[1]).unwrap();
        let first = mesh.edges[edge].first_corner.unwrap();
        let second = mesh.corners[first].radial_next;
        mesh.corners[second].radial_next = second;
        mesh.corners[second].radial_prev = second;
        assert!(mesh.validate().is_err());
    }

    #[test]
    fn error_displays_its_message() {
        let mut mesh = Mesh::new();
        let v = mesh.add_vertex([0.0; 3]).unwrap();
        mesh.vertices[v].position[0] = f32::NAN;
        let error = mesh.validate().unwrap_err();
        assert!(error.to_string().starts_with("invalid mesh: "));
        assert!(error.to_string().contains(error.message()));
    }
}
