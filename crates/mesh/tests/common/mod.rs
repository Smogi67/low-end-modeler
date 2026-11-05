//! Shapes and helpers shared by the integration tests. Primitives for users come
//! in Stage 3; these exist only to give the tests something to edit.
#![allow(dead_code)]

use modeler_mesh::{EdgeId, FaceId, Mesh, VertexId};

/// A cube with consistent outward winding.
pub fn cube() -> (Mesh, [VertexId; 8], [FaceId; 6]) {
    let mut mesh = Mesh::new();
    let positions = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
        [1.0, 0.0, 1.0],
        [1.0, 1.0, 1.0],
        [0.0, 1.0, 1.0],
    ];
    let v = positions.map(|p| mesh.add_vertex(p).unwrap());
    let indices = [
        [3, 2, 1, 0],
        [4, 5, 6, 7],
        [0, 1, 5, 4],
        [1, 2, 6, 5],
        [2, 3, 7, 6],
        [3, 0, 4, 7],
    ];
    let f = indices.map(|quad| mesh.add_face(&quad.map(|i| v[i])).unwrap());
    mesh.validate().unwrap();
    (mesh, v, f)
}

/// A flat grid of `columns` by `rows` quads. Vertex (x, y) is at index `y * (columns + 1) + x`.
pub fn grid(columns: usize, rows: usize) -> (Mesh, Vec<VertexId>) {
    let mut mesh = Mesh::new();
    let mut v = Vec::with_capacity((columns + 1) * (rows + 1));
    for y in 0..=rows {
        for x in 0..=columns {
            v.push(mesh.add_vertex([x as f32, y as f32, 0.0]).unwrap());
        }
    }
    let at = |x: usize, y: usize| y * (columns + 1) + x;
    for y in 0..rows {
        for x in 0..columns {
            let quad = [
                v[at(x, y)],
                v[at(x + 1, y)],
                v[at(x + 1, y + 1)],
                v[at(x, y + 1)],
            ];
            mesh.add_face(&quad).unwrap();
        }
    }
    mesh.validate().unwrap();
    (mesh, v)
}

/// Vertices of a face, in winding order, starting from the face's lowest-sorted
/// vertex, so two cycles can be compared regardless of where each one starts.
pub fn cycle(mesh: &Mesh, face: FaceId) -> Vec<VertexId> {
    normalize_cycle(&mesh.face_vertices(face).unwrap().collect::<Vec<_>>())
}

pub fn normalize_cycle(vertices: &[VertexId]) -> Vec<VertexId> {
    let start = vertices
        .iter()
        .enumerate()
        .min_by_key(|(_, v)| **v)
        .map(|(i, _)| i)
        .unwrap_or(0);
    let mut out = vertices.to_vec();
    out.rotate_left(start);
    out
}

pub fn euler_characteristic(mesh: &Mesh) -> i64 {
    mesh.vertex_count() as i64 - mesh.edge_count() as i64 + mesh.face_count() as i64
}

/// True if the mesh is a closed surface with neighbors winding consistently.
pub fn is_closed_and_consistent(mesh: &Mesh) -> bool {
    mesh.edges().all(|(id, _)| {
        let corners: Vec<_> = mesh
            .edge_corners(id)
            .unwrap()
            .map(|c| mesh.corner(c).unwrap().vertex())
            .collect();
        corners.len() == 2 && corners[0] != corners[1]
    })
}

pub fn edge(mesh: &Mesh, a: VertexId, b: VertexId) -> EdgeId {
    mesh.edge_between(a, b)
        .unwrap()
        .expect("expected an edge here")
}
