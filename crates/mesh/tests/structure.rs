//! Neighbor lookups, and the unusual shapes that the structure must be able to hold.

mod common;
use common::{cube, cycle, edge, euler_characteristic, grid, is_closed_and_consistent};
use modeler_mesh::{EdgeId, Mesh, MeshError};
use std::collections::HashSet;

#[test]
fn empty_mesh_is_valid() {
    let mesh = Mesh::new();
    mesh.validate().unwrap();
    assert_eq!(mesh.vertex_count(), 0);
    assert_eq!(mesh.edge_count(), 0);
    assert_eq!(mesh.corner_count(), 0);
    assert_eq!(mesh.face_count(), 0);
    assert_eq!(mesh, Mesh::default());
}

#[test]
fn cube_counts_are_correct() {
    let (mesh, _, _) = cube();
    assert_eq!(mesh.vertex_count(), 8);
    assert_eq!(mesh.edge_count(), 12);
    assert_eq!(mesh.corner_count(), 24);
    assert_eq!(mesh.face_count(), 6);
    assert_eq!(euler_characteristic(&mesh), 2);
    assert!(is_closed_and_consistent(&mesh));
}

#[test]
fn neighbor_lookups_on_a_cube() {
    let (mesh, v, f) = cube();
    for &vertex in &v {
        let edges: HashSet<_> = mesh.vertex_edges(vertex).unwrap().collect();
        let faces: HashSet<_> = mesh.vertex_faces(vertex).unwrap().collect();
        let corners: Vec<_> = mesh.vertex_corners(vertex).unwrap().collect();
        assert_eq!(edges.len(), 3);
        assert_eq!(faces.len(), 3);
        assert_eq!(corners.len(), 3);
        for corner in corners {
            assert_eq!(mesh.corner(corner).unwrap().vertex(), vertex);
        }
        for e in edges {
            assert!(mesh.edge(e).unwrap().touches(vertex));
        }
    }
    for (id, data) in mesh.edges() {
        assert_eq!(data.vertices().len(), 2);
        assert_eq!(mesh.edge_faces(id).unwrap().count(), 2);
        assert!(data.first_corner().is_some());
    }
    for &face in &f {
        assert_eq!(mesh.face(face).unwrap().corner_count(), 4);
        assert_eq!(mesh.face_vertices(face).unwrap().count(), 4);
        assert_eq!(mesh.face_edges(face).unwrap().count(), 4);
        assert_eq!(mesh.face_corners(face).unwrap().count(), 4);
    }
    assert_eq!(cycle(&mesh, f[1]), vec![v[4], v[5], v[6], v[7]]);
    assert_eq!(mesh.vertex(v[6]).unwrap().position(), [1.0, 1.0, 1.0]);
    assert!(mesh.vertex(v[0]).unwrap().first_edge().is_some());
}

#[test]
fn corner_links_lead_where_they_claim_to() {
    let (mesh, _, _) = cube();
    for (id, corner) in mesh.corners() {
        let next = mesh.corner(corner.next()).unwrap();
        let prev = mesh.corner(corner.prev()).unwrap();
        assert_eq!(next.prev(), id);
        assert_eq!(prev.next(), id);
        assert_eq!(next.face(), corner.face());
        let edge = mesh.edge(corner.edge()).unwrap();
        assert_eq!(edge.other_vertex(corner.vertex()), Some(next.vertex()));
        let radial = mesh.corner(corner.radial_next()).unwrap();
        assert_eq!(radial.radial_prev(), id);
        assert_eq!(radial.edge(), corner.edge());
        assert_ne!(radial.face(), corner.face());
    }
}

#[test]
fn edge_helpers() {
    let mut mesh = Mesh::new();
    let a = mesh.add_vertex([0.0; 3]).unwrap();
    let b = mesh.add_vertex([1.0; 3]).unwrap();
    let c = mesh.add_vertex([2.0; 3]).unwrap();
    let e = mesh.add_edge(a, b).unwrap();
    let data = mesh.edge(e).unwrap();
    assert_eq!(data.vertices(), [a, b]);
    assert!(data.touches(a) && data.touches(b) && !data.touches(c));
    assert_eq!(data.other_vertex(a), Some(b));
    assert_eq!(data.other_vertex(b), Some(a));
    assert_eq!(data.other_vertex(c), None);
    assert_eq!(mesh.edge_between(a, b), Ok(Some(e)));
    assert_eq!(mesh.edge_between(b, a), Ok(Some(e)));
    assert_eq!(mesh.edge_between(a, c), Ok(None));
}

#[test]
fn face_with_vertices_ignores_order_only_for_the_exact_set() {
    let (mesh, v, f) = cube();
    assert_eq!(
        mesh.face_with_vertices(&[v[6], v[4], v[7], v[5]]),
        Ok(Some(f[1]))
    );
    assert_eq!(mesh.face_with_vertices(&[v[4], v[5], v[6]]), Ok(None));
    assert_eq!(mesh.face_with_vertices(&[v[0], v[1], v[6], v[7]]), Ok(None));
    assert_eq!(mesh.face_with_vertices(&[]), Ok(None));
}

#[test]
fn queries_report_stale_ids() {
    let (mut mesh, v, f) = cube();
    let e = edge(&mesh, v[0], v[1]);
    let c = mesh.vertex_corners(v[0]).unwrap().next().unwrap();
    mesh.remove_vertex(v[0]).unwrap();

    assert!(mesh.vertex(v[0]).is_none());
    assert!(mesh.edge(e).is_none());
    assert!(mesh.corner(c).is_none());
    assert!(mesh.face(f[0]).is_none());
    assert_eq!(
        mesh.vertex_edges(v[0]).err(),
        Some(MeshError::StaleVertex(v[0]))
    );
    assert_eq!(
        mesh.vertex_corners(v[0]).err(),
        Some(MeshError::StaleVertex(v[0]))
    );
    assert_eq!(
        mesh.vertex_faces(v[0]).err(),
        Some(MeshError::StaleVertex(v[0]))
    );
    assert_eq!(mesh.edge_corners(e).err(), Some(MeshError::StaleEdge(e)));
    assert_eq!(mesh.edge_faces(e).err(), Some(MeshError::StaleEdge(e)));
    assert_eq!(
        mesh.face_corners(f[0]).err(),
        Some(MeshError::StaleFace(f[0]))
    );
    assert_eq!(
        mesh.face_vertices(f[0]).err(),
        Some(MeshError::StaleFace(f[0]))
    );
    assert_eq!(
        mesh.face_edges(f[0]).err(),
        Some(MeshError::StaleFace(f[0]))
    );
    assert_eq!(
        mesh.edge_between(v[0], v[1]),
        Err(MeshError::StaleVertex(v[0]))
    );
    assert_eq!(
        mesh.edge_between(v[1], v[0]),
        Err(MeshError::StaleVertex(v[0]))
    );
    assert_eq!(
        mesh.face_with_vertices(&[v[1], v[0]]),
        Err(MeshError::StaleVertex(v[0]))
    );
}

#[test]
fn a_new_element_in_a_reused_slot_does_not_answer_to_the_old_id() {
    let mut mesh = Mesh::new();
    let old = mesh.add_vertex([1.0, 2.0, 3.0]).unwrap();
    mesh.remove_vertex(old).unwrap();
    let new = mesh.add_vertex([4.0, 5.0, 6.0]).unwrap();
    assert_eq!(
        old.index(),
        new.index(),
        "this test needs the slot to be reused"
    );
    assert_ne!(old, new);
    assert!(mesh.vertex(old).is_none());
    assert_eq!(
        mesh.set_vertex_position(old, [0.0; 3]),
        Err(MeshError::StaleVertex(old))
    );
    assert_eq!(mesh.vertex(new).unwrap().position(), [4.0, 5.0, 6.0]);
}

#[test]
fn holds_loose_vertices_and_wire_edges() {
    let mut mesh = Mesh::new();
    let a = mesh.add_vertex([0.0; 3]).unwrap();
    let b = mesh.add_vertex([1.0; 3]).unwrap();
    let c = mesh.add_vertex([2.0; 3]).unwrap();
    let e = mesh.add_edge(a, b).unwrap();
    mesh.validate().unwrap();
    assert_eq!(mesh.vertex(c).unwrap().first_edge(), None);
    assert_eq!(mesh.vertex_edges(c).unwrap().count(), 0);
    assert_eq!(mesh.vertex_faces(c).unwrap().count(), 0);
    assert_eq!(mesh.edge(e).unwrap().first_corner(), None);
    assert_eq!(mesh.edge_faces(e).unwrap().count(), 0);
}

#[test]
fn holds_an_edge_shared_by_three_faces() {
    let mut mesh = Mesh::new();
    let v: Vec<_> = (0..5)
        .map(|i| mesh.add_vertex([i as f32, 0.0, 0.0]).unwrap())
        .collect();
    mesh.add_face(&[v[0], v[1], v[2]]).unwrap();
    mesh.add_face(&[v[1], v[0], v[3]]).unwrap();
    mesh.add_face(&[v[0], v[1], v[4]]).unwrap();
    mesh.validate().unwrap();
    assert_eq!(mesh.edge_faces(edge(&mesh, v[0], v[1])).unwrap().count(), 3);
}

#[test]
fn holds_two_fans_meeting_at_one_vertex() {
    let mut mesh = Mesh::new();
    let v: Vec<_> = (0..5)
        .map(|i| mesh.add_vertex([i as f32, 0.0, 0.0]).unwrap())
        .collect();
    mesh.add_face(&[v[0], v[1], v[2]]).unwrap();
    mesh.add_face(&[v[0], v[3], v[4]]).unwrap();
    mesh.validate().unwrap();
    assert_eq!(mesh.vertex_faces(v[0]).unwrap().count(), 2);
    assert_eq!(mesh.vertex_edges(v[0]).unwrap().count(), 4);
}

#[test]
fn holds_neighbors_with_opposite_winding() {
    let mut mesh = Mesh::new();
    let v: Vec<_> = (0..4)
        .map(|i| mesh.add_vertex([i as f32, 0.0, 0.0]).unwrap())
        .collect();
    mesh.add_face(&[v[0], v[1], v[2]]).unwrap();
    mesh.add_face(&[v[0], v[1], v[3]]).unwrap();
    mesh.validate().unwrap();
}

#[test]
fn holds_large_ngons() {
    let mut mesh = Mesh::new();
    let v: Vec<_> = (0..500)
        .map(|i| mesh.add_vertex([i as f32, 0.0, 0.0]).unwrap())
        .collect();
    let face = mesh.add_face(&v).unwrap();
    mesh.validate().unwrap();
    assert_eq!(mesh.face(face).unwrap().corner_count(), 500);
    assert_eq!(cycle(&mesh, face), v);
}

/// Stage 6's loop tools need this walk: straight across each quad corner to the
/// next edge. Proves the stored links are enough, without building the tool early.
#[test]
fn links_support_walking_edge_rings_and_edge_loops() {
    let (mesh, v) = grid(5, 3);
    let at = |x: usize, y: usize| v[y * 6 + x];

    // Ring: from a bottom edge across the quads to the opposite edges.
    let start = edge(&mesh, at(2, 0), at(3, 0));
    let mut ring: Vec<EdgeId> = vec![start];
    let mut corner = mesh.edge(start).unwrap().first_corner().unwrap();
    loop {
        let data = mesh.corner(corner).unwrap();
        let opposite = mesh
            .corner(mesh.corner(data.next()).unwrap().next())
            .unwrap();
        ring.push(opposite.edge());
        let across = opposite.radial_next();
        if mesh.corner(across).unwrap().face() == opposite.face() {
            break;
        }
        corner = across;
    }
    let expected: Vec<EdgeId> = (0..=3).map(|y| edge(&mesh, at(2, y), at(3, y))).collect();
    assert_eq!(ring, expected);

    // Loop: along a horizontal line. Continue through the edge at the far side of
    // the vertex, where four edges meet.
    let mut walked = vec![edge(&mesh, at(0, 1), at(1, 1))];
    let mut from = at(0, 1);
    while let Some(next) = next_in_loop(&mesh, *walked.last().unwrap(), from) {
        from = mesh
            .edge(*walked.last().unwrap())
            .unwrap()
            .other_vertex(from)
            .unwrap();
        walked.push(next);
    }
    let expected: Vec<EdgeId> = (0..5)
        .map(|x| edge(&mesh, at(x, 1), at(x + 1, 1)))
        .collect();
    assert_eq!(walked, expected);
}

/// The edge continuing straight on from `current`, leaving through its far vertex.
fn next_in_loop(mesh: &Mesh, current: EdgeId, from: modeler_mesh::VertexId) -> Option<EdgeId> {
    let far = mesh.edge(current).unwrap().other_vertex(from)?;
    if mesh.vertex_edges(far).unwrap().count() != 4 {
        return None;
    }
    let side_faces: HashSet<_> = mesh.edge_faces(current).unwrap().collect();
    mesh.vertex_edges(far)
        .unwrap()
        .filter(|&candidate| candidate != current)
        .find(|&candidate| {
            mesh.edge_faces(candidate)
                .unwrap()
                .all(|f| !side_faces.contains(&f))
        })
}

#[test]
fn memory_use_per_element_stays_small() {
    use std::mem::size_of;
    // These limits catch accidental growth, which matters on a 4 GB machine.
    assert_eq!(size_of::<modeler_mesh::VertexId>(), 8);
    assert_eq!(size_of::<Option<modeler_mesh::VertexId>>(), 8);
    assert!(size_of::<modeler_mesh::Vertex>() <= 24);
    assert!(size_of::<modeler_mesh::Edge>() <= 60);
    assert!(size_of::<modeler_mesh::Corner>() <= 56);
    assert!(size_of::<modeler_mesh::Face>() <= 16);
}

#[test]
fn memory_bytes_grows_with_the_mesh() {
    let (small, _) = grid(2, 2);
    let (large, _) = grid(20, 20);
    assert!(small.memory_bytes() > 0);
    assert!(large.memory_bytes() > small.memory_bytes() * 10);
}
