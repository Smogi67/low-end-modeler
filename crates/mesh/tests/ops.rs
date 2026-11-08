//! Each edit: its result, its refusals, and that the mesh stays valid.

mod common;
use common::{cube, cycle, edge, euler_characteristic, grid, is_closed_and_consistent};
use mixer_mesh::{Mesh, MeshError, VertexId};
use std::collections::HashSet;

fn verts(mesh: &mut Mesh, count: usize) -> Vec<VertexId> {
    (0..count)
        .map(|i| mesh.add_vertex([i as f32, (i * i) as f32, 0.0]).unwrap())
        .collect()
}

/// A refused edit must return this error and leave the mesh exactly as it was.
fn assert_refused<T: std::fmt::Debug>(
    mesh: &mut Mesh,
    expected: MeshError,
    edit: impl FnOnce(&mut Mesh) -> Result<T, MeshError>,
) {
    let before = mesh.clone();
    let result = edit(mesh);
    assert_eq!(result.err(), Some(expected));
    assert_eq!(*mesh, before, "a refused edit changed the mesh");
    mesh.validate().unwrap();
}

#[test]
fn add_vertex_and_move_it() {
    let mut mesh = Mesh::new();
    let v = mesh.add_vertex([1.0, 2.0, 3.0]).unwrap();
    assert_eq!(mesh.vertex(v).unwrap().position(), [1.0, 2.0, 3.0]);
    mesh.set_vertex_position(v, [4.0, 5.0, 6.0]).unwrap();
    assert_eq!(mesh.vertex(v).unwrap().position(), [4.0, 5.0, 6.0]);
    mesh.validate().unwrap();
}

#[test]
fn non_finite_positions_are_refused_in_every_component() {
    let mut mesh = Mesh::new();
    let v = mesh.add_vertex([0.0; 3]).unwrap();
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        for axis in 0..3 {
            let mut position = [0.0; 3];
            position[axis] = bad;
            assert_refused(&mut mesh, MeshError::NonFinitePosition, |m| {
                m.add_vertex(position)
            });
            assert_refused(&mut mesh, MeshError::NonFinitePosition, |m| {
                m.set_vertex_position(v, position)
            });
        }
    }
}

#[test]
fn add_edge_and_its_refusals() {
    let mut mesh = Mesh::new();
    let v = verts(&mut mesh, 3);
    let e = mesh.add_edge(v[0], v[1]).unwrap();
    mesh.validate().unwrap();
    assert_refused(&mut mesh, MeshError::SameVertex(v[0]), |m| {
        m.add_edge(v[0], v[0])
    });
    assert_refused(&mut mesh, MeshError::EdgeExists(e), |m| {
        m.add_edge(v[0], v[1])
    });
    assert_refused(&mut mesh, MeshError::EdgeExists(e), |m| {
        m.add_edge(v[1], v[0])
    });

    mesh.remove_vertex(v[2]).unwrap();
    assert_refused(&mut mesh, MeshError::StaleVertex(v[2]), |m| {
        m.add_edge(v[2], v[0])
    });
    assert_refused(&mut mesh, MeshError::StaleVertex(v[2]), |m| {
        m.add_edge(v[0], v[2])
    });
}

#[test]
fn many_edges_at_one_vertex() {
    let mut mesh = Mesh::new();
    let v = verts(&mut mesh, 10);
    for i in 1..10 {
        mesh.add_edge(v[0], v[i]).unwrap();
        mesh.validate().unwrap();
    }
    assert_eq!(mesh.vertex_edges(v[0]).unwrap().count(), 9);
    for i in (1..10).rev() {
        mesh.remove_edge(edge(&mesh, v[0], v[i])).unwrap();
        mesh.validate().unwrap();
    }
    assert_eq!(mesh.vertex(v[0]).unwrap().first_edge(), None);
}

#[test]
fn add_face_creates_missing_edges_and_reuses_existing_ones() {
    let mut mesh = Mesh::new();
    let v = verts(&mut mesh, 4);
    let existing = mesh.add_edge(v[1], v[0]).unwrap();
    let face = mesh.add_face(&[v[0], v[1], v[2], v[3]]).unwrap();
    mesh.validate().unwrap();
    assert_eq!(mesh.edge_count(), 4);
    assert_eq!(mesh.corner_count(), 4);
    assert_eq!(edge(&mesh, v[0], v[1]), existing);
    assert_eq!(cycle(&mesh, face), v);
    let edges: Vec<_> = mesh.face_edges(face).unwrap().collect();
    assert_eq!(edges[0], existing);
    assert_eq!(edges[1], edge(&mesh, v[1], v[2]));
    assert_eq!(edges[3], edge(&mesh, v[3], v[0]));
}

#[test]
fn add_face_refusals() {
    let mut mesh = Mesh::new();
    let v = verts(&mut mesh, 5);
    let face = mesh.add_face(&[v[0], v[1], v[2]]).unwrap();
    assert_refused(&mut mesh, MeshError::FaceTooSmall { count: 0 }, |m| {
        m.add_face(&[])
    });
    assert_refused(&mut mesh, MeshError::FaceTooSmall { count: 2 }, |m| {
        m.add_face(&[v[0], v[1]])
    });
    assert_refused(&mut mesh, MeshError::RepeatedVertex(v[3]), |m| {
        m.add_face(&[v[3], v[4], v[3]])
    });
    assert_refused(&mut mesh, MeshError::FaceExists(face), |m| {
        m.add_face(&[v[1], v[2], v[0]])
    });
    assert_refused(&mut mesh, MeshError::FaceExists(face), |m| {
        m.add_face(&[v[2], v[1], v[0]])
    });
    let dead = v[4];
    mesh.remove_vertex(dead).unwrap();
    assert_refused(&mut mesh, MeshError::StaleVertex(dead), |m| {
        m.add_face(&[v[0], v[1], dead])
    });
}

#[test]
fn faces_sharing_some_vertices_are_not_duplicates() {
    let mut mesh = Mesh::new();
    let v = verts(&mut mesh, 5);
    mesh.add_face(&[v[0], v[1], v[2], v[3]]).unwrap();
    mesh.add_face(&[v[0], v[1], v[2]]).unwrap();
    mesh.add_face(&[v[0], v[1], v[2], v[4]]).unwrap();
    mesh.validate().unwrap();
}

#[test]
fn remove_face_keeps_edges_and_vertices() {
    let (mut mesh, _, f) = cube();
    mesh.remove_face(f[0]).unwrap();
    mesh.validate().unwrap();
    assert_eq!((mesh.vertex_count(), mesh.edge_count()), (8, 12));
    assert_eq!((mesh.corner_count(), mesh.face_count()), (20, 5));
    assert_refused(&mut mesh, MeshError::StaleFace(f[0]), |m| {
        m.remove_face(f[0])
    });
}

#[test]
fn remove_edge_removes_its_faces() {
    let (mut mesh, v, f) = cube();
    let e = edge(&mesh, v[0], v[1]);
    mesh.remove_edge(e).unwrap();
    mesh.validate().unwrap();
    assert_eq!(mesh.vertex_count(), 8);
    assert_eq!(mesh.edge_count(), 11);
    assert_eq!(mesh.face_count(), 4);
    assert!(mesh.face(f[0]).is_none() && mesh.face(f[2]).is_none());
    assert!(mesh.face(f[1]).is_some());
    assert_refused(&mut mesh, MeshError::StaleEdge(e), |m| m.remove_edge(e));
}

#[test]
fn remove_edge_with_three_faces() {
    let mut mesh = Mesh::new();
    let v = verts(&mut mesh, 5);
    let f1 = mesh.add_face(&[v[0], v[1], v[2]]).unwrap();
    let f2 = mesh.add_face(&[v[0], v[1], v[3]]).unwrap();
    let f3 = mesh.add_face(&[v[0], v[1], v[4]]).unwrap();
    mesh.remove_edge(edge(&mesh, v[0], v[1])).unwrap();
    mesh.validate().unwrap();
    for f in [f1, f2, f3] {
        assert!(mesh.face(f).is_none());
    }
    assert_eq!(mesh.corner_count(), 0);
}

#[test]
fn remove_vertex_removes_everything_using_it() {
    let (mut mesh, v, _) = cube();
    mesh.remove_vertex(v[0]).unwrap();
    mesh.validate().unwrap();
    assert_eq!(mesh.vertex_count(), 7);
    assert_eq!(mesh.edge_count(), 9);
    assert_eq!(mesh.face_count(), 3);
    assert_refused(&mut mesh, MeshError::StaleVertex(v[0]), |m| {
        m.remove_vertex(v[0])
    });
}

#[test]
fn removing_every_vertex_leaves_an_empty_valid_mesh() {
    let (mut mesh, v, _) = cube();
    for vertex in v {
        mesh.remove_vertex(vertex).unwrap();
        mesh.validate().unwrap();
    }
    assert_eq!(mesh.vertices().count(), 0);
    assert_eq!(mesh.edges().count(), 0);
    assert_eq!(mesh.corners().count(), 0);
    assert_eq!(mesh.faces().count(), 0);
}

#[test]
fn split_edge_on_a_wire_edge() {
    let mut mesh = Mesh::new();
    let a = mesh.add_vertex([0.0, 0.0, 0.0]).unwrap();
    let b = mesh.add_vertex([10.0, 20.0, 40.0]).unwrap();
    let e = mesh.add_edge(a, b).unwrap();
    let (middle, new_edge) = mesh.split_edge(e, 0.25).unwrap();
    mesh.validate().unwrap();
    assert_eq!(mesh.vertex(middle).unwrap().position(), [2.5, 5.0, 10.0]);
    assert_eq!(mesh.edge(e).unwrap().vertices(), [a, middle]);
    assert_eq!(mesh.edge(new_edge).unwrap().vertices(), [middle, b]);
    assert_eq!(mesh.edge_between(a, b), Ok(None));
}

#[test]
fn split_edge_factor_is_exact_at_both_ends() {
    let mut mesh = Mesh::new();
    let a = mesh.add_vertex([0.1, 0.2, 0.3]).unwrap();
    let b = mesh.add_vertex([0.7, 0.9, 1.1]).unwrap();
    let e = mesh.add_edge(a, b).unwrap();
    let (at_start, _) = mesh.split_edge(e, 0.0).unwrap();
    assert_eq!(mesh.vertex(at_start).unwrap().position(), [0.1, 0.2, 0.3]);

    let mut mesh = Mesh::new();
    let a = mesh.add_vertex([0.1, 0.2, 0.3]).unwrap();
    let b = mesh.add_vertex([0.7, 0.9, 1.1]).unwrap();
    let e = mesh.add_edge(a, b).unwrap();
    let (at_end, _) = mesh.split_edge(e, 1.0).unwrap();
    assert_eq!(mesh.vertex(at_end).unwrap().position(), [0.7, 0.9, 1.1]);
}

#[test]
fn split_edge_refusals() {
    let mut mesh = Mesh::new();
    let v = verts(&mut mesh, 2);
    let e = mesh.add_edge(v[0], v[1]).unwrap();
    for bad in [-0.01, 1.01, f32::NAN, f32::INFINITY] {
        assert_refused(&mut mesh, MeshError::InvalidFactor, |m| {
            m.split_edge(e, bad)
        });
    }
    mesh.remove_edge(e).unwrap();
    assert_refused(&mut mesh, MeshError::StaleEdge(e), |m| m.split_edge(e, 0.5));
}

#[test]
fn split_edge_updates_both_faces_on_a_cube() {
    let (mut mesh, v, f) = cube();
    let e = edge(&mesh, v[0], v[1]);
    let (middle, _) = mesh.split_edge(e, 0.5).unwrap();
    mesh.validate().unwrap();
    assert_eq!(mesh.vertex_count(), 9);
    assert_eq!(mesh.edge_count(), 13);
    assert_eq!(mesh.face_count(), 6);
    assert_eq!(euler_characteristic(&mesh), 2);
    assert!(is_closed_and_consistent(&mesh));
    assert_eq!(cycle(&mesh, f[0]), vec![v[0], v[3], v[2], v[1], middle]);
    assert_eq!(cycle(&mesh, f[2]), vec![v[0], middle, v[1], v[5], v[4]]);
    assert_eq!(mesh.face(f[0]).unwrap().corner_count(), 5);
    assert_eq!(mesh.vertex_faces(middle).unwrap().count(), 2);
}

#[test]
fn split_edge_with_three_faces_and_mixed_winding() {
    let mut mesh = Mesh::new();
    let v = verts(&mut mesh, 5);
    let f1 = mesh.add_face(&[v[0], v[1], v[2]]).unwrap();
    let f2 = mesh.add_face(&[v[1], v[0], v[3]]).unwrap();
    let f3 = mesh.add_face(&[v[0], v[1], v[4]]).unwrap();
    let e = edge(&mesh, v[0], v[1]);
    let (middle, new_edge) = mesh.split_edge(e, 0.5).unwrap();
    mesh.validate().unwrap();
    assert_eq!(cycle(&mesh, f1), vec![v[0], middle, v[1], v[2]]);
    assert_eq!(cycle(&mesh, f2), vec![v[0], v[3], v[1], middle]);
    assert_eq!(cycle(&mesh, f3), vec![v[0], middle, v[1], v[4]]);
    assert_eq!(mesh.edge_faces(e).unwrap().count(), 3);
    assert_eq!(mesh.edge_faces(new_edge).unwrap().count(), 3);
}

#[test]
fn split_face_on_a_quad() {
    let (mut mesh, v, f) = cube();
    let top = f[1];
    let (new_face, e) = mesh.split_face(top, v[4], v[6]).unwrap();
    mesh.validate().unwrap();
    assert_eq!(cycle(&mesh, top), vec![v[4], v[5], v[6]]);
    assert_eq!(cycle(&mesh, new_face), vec![v[4], v[6], v[7]]);
    assert_eq!(mesh.edge(e).unwrap().vertices(), [v[4], v[6]]);
    assert_eq!(mesh.edge_faces(e).unwrap().count(), 2);
    assert_eq!(euler_characteristic(&mesh), 2);
    assert!(is_closed_and_consistent(&mesh));
}

#[test]
fn split_face_on_an_ngon_keeps_the_documented_parts() {
    let mut mesh = Mesh::new();
    let v = verts(&mut mesh, 7);
    let face = mesh.add_face(&v).unwrap();
    let (new_face, _) = mesh.split_face(face, v[5], v[1]).unwrap();
    mesh.validate().unwrap();
    assert_eq!(cycle(&mesh, face), vec![v[0], v[1], v[5], v[6]]);
    assert_eq!(cycle(&mesh, new_face), vec![v[1], v[2], v[3], v[4], v[5]]);
    assert_eq!(mesh.face(face).unwrap().corner_count(), 4);
    assert_eq!(mesh.face(new_face).unwrap().corner_count(), 5);
    for corner in mesh.face_corners(new_face).unwrap() {
        assert_eq!(mesh.corner(corner).unwrap().face(), new_face);
    }
    for corner in mesh.face_corners(face).unwrap() {
        assert_eq!(mesh.corner(corner).unwrap().face(), face);
    }
}

#[test]
fn split_face_reuses_a_wire_edge() {
    let mut mesh = Mesh::new();
    let v = verts(&mut mesh, 4);
    let face = mesh.add_face(&v).unwrap();
    let wire = mesh.add_edge(v[0], v[2]).unwrap();
    let (_, e) = mesh.split_face(face, v[0], v[2]).unwrap();
    mesh.validate().unwrap();
    assert_eq!(e, wire);
    assert_eq!(mesh.edge_count(), 5);
}

#[test]
fn split_face_refusals() {
    let mut mesh = Mesh::new();
    let v = verts(&mut mesh, 6);
    let quad = mesh.add_face(&[v[0], v[1], v[2], v[3]]).unwrap();
    let tri = mesh.add_face(&[v[0], v[1], v[4]]).unwrap();
    assert_refused(&mut mesh, MeshError::SameVertex(v[0]), |m| {
        m.split_face(quad, v[0], v[0])
    });
    for (a, b) in [(v[0], v[1]), (v[1], v[0]), (v[3], v[0]), (v[0], v[3])] {
        assert_refused(&mut mesh, MeshError::CornersAdjacent { face: quad }, |m| {
            m.split_face(quad, a, b)
        });
    }
    assert_refused(&mut mesh, MeshError::CornersAdjacent { face: tri }, |m| {
        m.split_face(tri, v[0], v[1])
    });
    assert_refused(
        &mut mesh,
        MeshError::VertexNotInFace {
            vertex: v[5],
            face: quad,
        },
        |m| m.split_face(quad, v[5], v[1]),
    );
    assert_refused(
        &mut mesh,
        MeshError::VertexNotInFace {
            vertex: v[5],
            face: quad,
        },
        |m| m.split_face(quad, v[1], v[5]),
    );
    let dead = v[5];
    mesh.remove_vertex(dead).unwrap();
    assert_refused(&mut mesh, MeshError::StaleVertex(dead), |m| {
        m.split_face(quad, dead, v[1])
    });
    assert_refused(&mut mesh, MeshError::StaleVertex(dead), |m| {
        m.split_face(quad, v[1], dead)
    });
    mesh.remove_face(tri).unwrap();
    assert_refused(&mut mesh, MeshError::StaleFace(tri), |m| {
        m.split_face(tri, v[0], v[1])
    });
}

#[test]
fn split_face_refuses_to_duplicate_either_part() {
    let mut mesh = Mesh::new();
    let v = verts(&mut mesh, 4);
    let quad = mesh.add_face(&v).unwrap();
    // The kept part would be 0-1-2.
    let kept = mesh.add_face(&[v[0], v[1], v[2]]).unwrap();
    assert_refused(&mut mesh, MeshError::FaceExists(kept), |m| {
        m.split_face(quad, v[0], v[2])
    });
    mesh.remove_face(kept).unwrap();
    // The moved part would be 2-3-0.
    let moved = mesh.add_face(&[v[2], v[3], v[0]]).unwrap();
    assert_refused(&mut mesh, MeshError::FaceExists(moved), |m| {
        m.split_face(quad, v[0], v[2])
    });
}

#[test]
fn join_undoes_split() {
    let (mut mesh, v, f) = cube();
    let top = f[1];
    let (new_face, e) = mesh.split_face(top, v[4], v[6]).unwrap();
    let kept = mesh.join_faces(e).unwrap();
    mesh.validate().unwrap();
    let removed = if kept == top { new_face } else { top };
    assert!(mesh.face(removed).is_none());
    assert!(mesh.edge(e).is_none());
    assert_eq!(cycle(&mesh, kept), vec![v[4], v[5], v[6], v[7]]);
    assert_eq!(mesh.face(kept).unwrap().corner_count(), 4);
    assert_eq!(
        (mesh.vertex_count(), mesh.edge_count(), mesh.face_count()),
        (8, 12, 6)
    );
    assert!(is_closed_and_consistent(&mesh));
    for corner in mesh.face_corners(kept).unwrap() {
        assert_eq!(mesh.corner(corner).unwrap().face(), kept);
    }
}

#[test]
fn join_two_ngons_gives_the_expected_outline() {
    let mut mesh = Mesh::new();
    let v = verts(&mut mesh, 7);
    let f1 = mesh.add_face(&[v[0], v[1], v[2], v[3]]).unwrap();
    let f2 = mesh.add_face(&[v[1], v[0], v[4], v[5], v[6]]).unwrap();
    let kept = mesh.join_faces(edge(&mesh, v[0], v[1])).unwrap();
    mesh.validate().unwrap();
    assert!(kept == f1 || kept == f2);
    assert_eq!(
        cycle(&mesh, kept),
        vec![v[0], v[4], v[5], v[6], v[1], v[2], v[3]]
    );
    assert_eq!(mesh.face_count(), 1);
    assert_eq!(mesh.corner_count(), 7);
}

#[test]
fn join_refusals() {
    let mut mesh = Mesh::new();
    let v = verts(&mut mesh, 9);

    let wire = mesh.add_edge(v[7], v[8]).unwrap();
    assert_refused(
        &mut mesh,
        MeshError::NotTwoFaces {
            edge: wire,
            faces: 0,
        },
        |m| m.join_faces(wire),
    );

    mesh.add_face(&[v[0], v[1], v[2]]).unwrap();
    let e = edge(&mesh, v[0], v[1]);
    assert_refused(
        &mut mesh,
        MeshError::NotTwoFaces { edge: e, faces: 1 },
        |m| m.join_faces(e),
    );

    let second = mesh.add_face(&[v[0], v[1], v[3]]).unwrap();
    assert_refused(&mut mesh, MeshError::InconsistentWinding(e), |m| {
        m.join_faces(e)
    });

    mesh.flip_face(second).unwrap();
    mesh.add_face(&[v[0], v[1], v[4]]).unwrap();
    assert_refused(
        &mut mesh,
        MeshError::NotTwoFaces { edge: e, faces: 3 },
        |m| m.join_faces(e),
    );

    mesh.remove_edge(wire).unwrap();
    assert_refused(&mut mesh, MeshError::StaleEdge(wire), |m| {
        m.join_faces(wire)
    });
}

#[test]
fn join_refuses_faces_that_touch_elsewhere() {
    // Two faces sharing edge 0-1 and also touching at vertex 2.
    let mut mesh = Mesh::new();
    let v = verts(&mut mesh, 5);
    mesh.add_face(&[v[0], v[1], v[2], v[3]]).unwrap();
    mesh.add_face(&[v[1], v[0], v[4], v[2]]).unwrap();
    let e = edge(&mesh, v[0], v[1]);
    assert_refused(&mut mesh, MeshError::JoinWouldRepeatVertex(v[2]), |m| {
        m.join_faces(e)
    });
}

#[test]
fn join_refuses_faces_sharing_two_edges() {
    let mut mesh = Mesh::new();
    let v = verts(&mut mesh, 5);
    mesh.add_face(&[v[0], v[1], v[2], v[3]]).unwrap();
    mesh.add_face(&[v[2], v[1], v[0], v[4]]).unwrap();
    let e = edge(&mesh, v[0], v[1]);
    assert_refused(&mut mesh, MeshError::JoinWouldRepeatVertex(v[2]), |m| {
        m.join_faces(e)
    });
}

#[test]
fn join_refuses_to_duplicate_a_face() {
    let mut mesh = Mesh::new();
    let v = verts(&mut mesh, 4);
    let existing = mesh.add_face(&[v[0], v[1], v[2], v[3]]).unwrap();
    mesh.add_face(&[v[0], v[2], v[1]]).unwrap();
    mesh.add_face(&[v[0], v[3], v[2]]).unwrap();
    let e = edge(&mesh, v[0], v[2]);
    assert_refused(&mut mesh, MeshError::FaceExists(existing), |m| {
        m.join_faces(e)
    });
}

/// A face using only some of the merged vertices is not a duplicate.
#[test]
fn join_allows_a_face_on_a_subset_of_the_merged_vertices() {
    let mut mesh = Mesh::new();
    let v = verts(&mut mesh, 4);
    let subset = mesh.add_face(&[v[1], v[2], v[3]]).unwrap();
    mesh.add_face(&[v[0], v[2], v[1]]).unwrap();
    mesh.add_face(&[v[0], v[3], v[2]]).unwrap();
    let kept = mesh.join_faces(edge(&mesh, v[0], v[2])).unwrap();
    mesh.validate().unwrap();
    assert_eq!(mesh.face(kept).unwrap().corner_count(), 4);
    assert!(mesh.face(subset).is_some());
}

#[test]
fn flip_face_reverses_and_keeps_edges() {
    let (mut mesh, v, f) = cube();
    let face = f[1];
    let edges_before: HashSet<_> = mesh.face_edges(face).unwrap().collect();
    mesh.flip_face(face).unwrap();
    mesh.validate().unwrap();
    assert_eq!(cycle(&mesh, face), vec![v[4], v[7], v[6], v[5]]);
    let edges_after: HashSet<_> = mesh.face_edges(face).unwrap().collect();
    assert_eq!(edges_before, edges_after);
    assert_eq!(mesh.edge_count(), 12);
    assert!(!is_closed_and_consistent(&mesh));

    mesh.flip_face(face).unwrap();
    mesh.validate().unwrap();
    assert_eq!(cycle(&mesh, face), vec![v[4], v[5], v[6], v[7]]);
    assert!(is_closed_and_consistent(&mesh));
    mesh.remove_face(face).unwrap();
    assert_refused(&mut mesh, MeshError::StaleFace(face), |m| m.flip_face(face));
}

#[test]
fn flip_makes_a_mismatched_pair_joinable() {
    let mut mesh = Mesh::new();
    let v = verts(&mut mesh, 4);
    mesh.add_face(&[v[0], v[1], v[2]]).unwrap();
    let second = mesh.add_face(&[v[0], v[1], v[3]]).unwrap();
    mesh.flip_face(second).unwrap();
    let kept = mesh.join_faces(edge(&mesh, v[0], v[1])).unwrap();
    mesh.validate().unwrap();
    assert_eq!(mesh.face(kept).unwrap().corner_count(), 4);
}

#[test]
fn local_edits_on_a_closed_surface_keep_it_closed() {
    let (mut mesh, v, _) = cube();
    let (m1, _) = mesh.split_edge(edge(&mesh, v[0], v[1]), 0.5).unwrap();
    let (m2, _) = mesh.split_edge(edge(&mesh, v[1], v[2]), 0.5).unwrap();
    let face = mesh
        .face_with_vertices(&[v[3], v[2], m2, v[1], m1, v[0]])
        .unwrap()
        .unwrap();
    let (_, cut) = mesh.split_face(face, m1, m2).unwrap();
    mesh.validate().unwrap();
    assert_eq!(euler_characteristic(&mesh), 2);
    assert!(is_closed_and_consistent(&mesh));
    mesh.join_faces(cut).unwrap();
    mesh.validate().unwrap();
    assert_eq!(euler_characteristic(&mesh), 2);
    assert!(is_closed_and_consistent(&mesh));
}

#[test]
fn edits_work_on_a_larger_grid() {
    let (mut mesh, v) = grid(10, 10);
    assert_eq!(mesh.face_count(), 100);
    assert_eq!(mesh.vertex_count(), 121);
    assert_eq!(mesh.edge_count(), 220);
    // Remove a middle vertex: its 4 faces and 4 edges go with it.
    mesh.remove_vertex(v[5 * 11 + 5]).unwrap();
    mesh.validate().unwrap();
    assert_eq!(mesh.face_count(), 96);
    assert_eq!(mesh.edge_count(), 216);
}
