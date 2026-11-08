//! Knife and bisect.

mod common;
use common::{cube, edge, grid, is_closed_and_consistent};
use modeler_mesh::{CutPoint, Document, Element, Mesh, MeshError, SelectMode, SelectOp};

fn volume(mesh: &Mesh) -> f64 {
    let mut total = 0.0;
    for (face, _) in mesh.faces() {
        for t in mesh.face_triangles(face).unwrap() {
            let [a, b, c] = t.map(|v| mesh.vertex(v).unwrap().position().map(f64::from));
            total += (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                + a[2] * (b[0] * c[1] - b[1] * c[0]))
                / 6.0;
        }
    }
    total
}

#[test]
fn a_knife_line_across_one_face_splits_it() {
    let (mut mesh, v, _) = cube();
    let (faces, edges) = (mesh.face_count(), mesh.edge_count());
    // From the middle of one edge of a face to the middle of the opposite edge.
    let e1 = edge(&mesh, v[0], v[1]);
    let e2 = edge(&mesh, v[3], v[2]);
    let path = [
        CutPoint::Edge {
            edge: e1,
            factor: 0.5,
        },
        CutPoint::Edge {
            edge: e2,
            factor: 0.5,
        },
    ];
    assert_eq!(mesh.knife(&path, SelectMode::Edge), Ok(1));
    mesh.validate().unwrap();
    assert!(is_closed_and_consistent(&mesh));
    assert_eq!(mesh.face_count(), faces + 1);
    assert_eq!(mesh.edge_count(), edges + 3);
    assert!(
        (volume(&mesh) - 1.0).abs() < 1e-6,
        "the shape does not change"
    );
    // The cut is selected.
    assert_eq!(mesh.edges().filter(|(_, e)| e.is_selected()).count(), 1);
}

#[test]
fn a_knife_path_around_a_corner_cuts_three_faces() {
    let (mut mesh, v, _) = cube();
    let faces = mesh.face_count();
    // Around corner v[6]: across the three faces that meet there.
    let path = [
        CutPoint::Edge {
            edge: edge(&mesh, v[2], v[6]),
            factor: 0.5,
        },
        CutPoint::Edge {
            edge: edge(&mesh, v[5], v[6]),
            factor: 0.5,
        },
        CutPoint::Edge {
            edge: edge(&mesh, v[7], v[6]),
            factor: 0.5,
        },
        CutPoint::Edge {
            edge: edge(&mesh, v[2], v[6]),
            factor: 0.5,
        },
    ];
    assert_eq!(mesh.knife(&path, SelectMode::Edge), Ok(3));
    mesh.validate().unwrap();
    assert!(is_closed_and_consistent(&mesh));
    assert_eq!(mesh.face_count(), faces + 3);
}

#[test]
fn a_knife_can_go_corner_to_corner_and_cross_an_edge_twice() {
    let (mut mesh, v, _) = cube();
    // A diagonal from corner to corner.
    let path = [CutPoint::Vertex(v[0]), CutPoint::Vertex(v[2])];
    assert_eq!(mesh.knife(&path, SelectMode::Vertex), Ok(1));
    mesh.validate().unwrap();

    // Two spots on one edge, given out of order, both get points.
    let (mut mesh, v, _) = cube();
    let e = edge(&mesh, v[0], v[1]);
    let far = edge(&mesh, v[3], v[2]);
    // How far along an edge, measured from a given end.
    let at = |mesh: &Mesh, e, from, amount: f32| {
        let first = mesh.edge(e).unwrap().vertices()[0];
        let factor = if first == from { amount } else { 1.0 - amount };
        CutPoint::Edge { edge: e, factor }
    };
    let path = [
        at(&mesh, e, v[0], 0.7),
        at(&mesh, far, v[3], 0.7),
        at(&mesh, far, v[3], 0.3),
        at(&mesh, e, v[0], 0.3),
    ];
    assert_eq!(mesh.knife(&path, SelectMode::Edge), Ok(2));
    mesh.validate().unwrap();
    assert!(is_closed_and_consistent(&mesh));
    assert_eq!(mesh.vertex_count(), 8 + 4);
}

#[test]
fn knife_refusals_change_nothing() {
    let (mut mesh, v, _) = cube();
    let before = mesh.clone();
    // Two neighbors along an edge: nothing to cut.
    let path = [CutPoint::Vertex(v[0]), CutPoint::Vertex(v[1])];
    assert_eq!(
        mesh.knife(&path, SelectMode::Edge),
        Err(MeshError::NothingToCut)
    );
    let e = edge(&mesh, v[0], v[1]);
    let path = [CutPoint::Edge {
        edge: e,
        factor: 1.5,
    }];
    assert_eq!(
        mesh.knife(&path, SelectMode::Edge),
        Err(MeshError::InvalidFactor)
    );
    assert_eq!(mesh, before);
}

#[test]
fn knife_undo_is_exact() {
    let (mesh, v, _) = cube();
    let e1 = edge(&mesh, v[0], v[1]);
    let e2 = edge(&mesh, v[3], v[2]);
    let mut document = Document::new(mesh.clone());
    document
        .edit("Knife", |m| {
            m.knife(
                &[
                    CutPoint::Edge {
                        edge: e1,
                        factor: 0.25,
                    },
                    CutPoint::Edge {
                        edge: e2,
                        factor: 0.75,
                    },
                ],
                SelectMode::Edge,
            )
            .map(|_| ())
        })
        .unwrap();
    document.undo();
    assert_eq!(document.mesh(), &mesh);
}

fn select_all_faces(mesh: &mut Mesh) {
    let faces: Vec<Element> = mesh.faces().map(|(f, _)| Element::Face(f)).collect();
    mesh.select_elements(SelectMode::Face, &faces, SelectOp::Replace)
        .unwrap();
}

#[test]
fn bisect_cuts_a_cube_all_the_way_round() {
    let (mut mesh, _, _) = cube();
    select_all_faces(&mut mesh);
    // A slanted plane through the middle crosses four faces.
    let cuts = mesh
        .bisect_selected([0.5, 0.5, 0.5], [0.0, 0.3, 1.0], SelectMode::Face)
        .unwrap();
    assert_eq!(cuts, 4);
    mesh.validate().unwrap();
    assert!(is_closed_and_consistent(&mesh));
    assert_eq!(mesh.face_count(), 10);
    assert!((volume(&mesh) - 1.0).abs() < 1e-6);
}

#[test]
fn bisect_through_corners_and_a_grid() {
    // A plane through two opposite edges of the cube cuts along diagonals.
    let (mut mesh, _, _) = cube();
    select_all_faces(&mut mesh);
    let cuts = mesh
        .bisect_selected([0.0, 0.0, 0.0], [1.0, -1.0, 0.0], SelectMode::Face)
        .unwrap();
    assert_eq!(cuts, 2);
    mesh.validate().unwrap();

    // Only the selected faces are cut.
    let (mut grid_mesh, _) = grid(4, 4);
    let first: Vec<Element> = grid_mesh
        .faces()
        .take(4)
        .map(|(f, _)| Element::Face(f))
        .collect();
    grid_mesh
        .select_elements(SelectMode::Face, &first, SelectOp::Replace)
        .unwrap();
    let before = grid_mesh.face_count();
    let cuts = grid_mesh
        .bisect_selected([0.5, 0.5, 0.0], [1.0, 0.2, 0.0], SelectMode::Face)
        .unwrap();
    grid_mesh.validate().unwrap();
    assert!((1..=4).contains(&cuts));
    assert_eq!(grid_mesh.face_count(), before + cuts);
}

#[test]
fn bisect_refuses_what_it_cannot_cut() {
    let (mut mesh, _, _) = cube();
    let before = mesh.clone();
    assert_eq!(
        mesh.bisect_selected([0.5; 3], [0.0, 0.0, 1.0], SelectMode::Face),
        Err(MeshError::NoFacesSelected)
    );
    select_all_faces(&mut mesh);
    let selected = mesh.clone();
    // A plane that misses the cube.
    assert_eq!(
        mesh.bisect_selected([0.0, 0.0, 5.0], [0.0, 0.0, 1.0], SelectMode::Face),
        Err(MeshError::NothingToCut)
    );
    assert_eq!(mesh, selected);
    assert_ne!(before, selected);
}
