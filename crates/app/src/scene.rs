//! Turns the mesh into plain number lists for the graphics card. No graphics code,
//! so it is tested without a window.

use modeler_mesh::Mesh;

/// Position then normal, for each corner of each triangle.
pub const FLOATS_PER_TRIANGLE_VERTEX: usize = 6;
/// Position, for each end of each edge.
pub const FLOATS_PER_EDGE_VERTEX: usize = 3;

/// All triangles, with each face's normal on its corners for flat shading.
pub fn triangle_vertices(mesh: &Mesh) -> Vec<f32> {
    let mut out = Vec::with_capacity(mesh.triangle_count() * 3 * FLOATS_PER_TRIANGLE_VERTEX);
    for (face, _) in mesh.faces() {
        // These IDs come straight from the mesh, so they cannot be stale.
        let normal = mesh
            .face_normal(face)
            .ok()
            .flatten()
            .unwrap_or([0.0, 0.0, 1.0]);
        for triangle in mesh.face_triangles(face).unwrap_or_default() {
            for vertex in triangle {
                if let Some(vertex) = mesh.vertex(vertex) {
                    out.extend_from_slice(&vertex.position());
                    out.extend_from_slice(&normal);
                }
            }
        }
    }
    out
}

/// Both ends of every edge, including wire edges.
pub fn edge_vertices(mesh: &Mesh) -> Vec<f32> {
    let mut out = Vec::with_capacity(mesh.edge_count() * 2 * FLOATS_PER_EDGE_VERTEX);
    for (_, edge) in mesh.edges() {
        for vertex in edge.vertices() {
            if let Some(vertex) = mesh.vertex(vertex) {
                out.extend_from_slice(&vertex.position());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use modeler_mesh::Primitive;

    #[test]
    fn an_empty_mesh_gives_nothing() {
        let mesh = Mesh::new();
        assert!(triangle_vertices(&mesh).is_empty());
        assert!(edge_vertices(&mesh).is_empty());
    }

    #[test]
    fn a_cube_gives_12_triangles_and_12_edges() {
        let mut mesh = Mesh::new();
        mesh.add_primitive(Primitive::Cube { size: 2.0 }, [0.0; 3])
            .unwrap();
        assert_eq!(
            triangle_vertices(&mesh).len(),
            12 * 3 * FLOATS_PER_TRIANGLE_VERTEX
        );
        assert_eq!(edge_vertices(&mesh).len(), 12 * 2 * FLOATS_PER_EDGE_VERTEX);
    }

    #[test]
    fn each_corner_carries_its_faces_normal() {
        let mut mesh = Mesh::new();
        mesh.add_primitive(
            Primitive::Circle {
                radius: 1.0,
                segments: 8,
            },
            [0.0, 0.0, 5.0],
        )
        .unwrap();
        let data = triangle_vertices(&mesh);
        assert_eq!(data.len(), 6 * 3 * FLOATS_PER_TRIANGLE_VERTEX);
        for vertex in data.chunks_exact(FLOATS_PER_TRIANGLE_VERTEX) {
            assert_eq!(vertex[2], 5.0);
            assert_eq!(vertex[3..], [0.0, 0.0, 1.0]);
        }
    }

    #[test]
    fn wire_edges_are_drawn_and_a_face_without_area_gets_a_usable_normal() {
        let mut mesh = Mesh::new();
        let v: Vec<_> = (0..3)
            .map(|i| mesh.add_vertex([i as f32, 0.0, 0.0]).unwrap())
            .collect();
        mesh.add_edge(v[0], v[1]).unwrap();
        assert_eq!(edge_vertices(&mesh).len(), 2 * FLOATS_PER_EDGE_VERTEX);
        assert!(triangle_vertices(&mesh).is_empty());
        mesh.add_face(&v).unwrap();
        let data = triangle_vertices(&mesh);
        assert_eq!(data[3..6], [0.0, 0.0, 1.0]);
    }
}
