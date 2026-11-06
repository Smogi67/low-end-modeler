//! Selection modes, growing and shrinking, duplicate, mirror, and undo of all of them.

mod common;
use common::{cube, edge, grid};
use modeler_mesh::{Document, Element, Mesh, MeshError, Primitive, SelectMode, SelectOp};
use std::collections::HashSet;

fn counts(mesh: &Mesh) -> (usize, usize, usize) {
    (
        mesh.vertices().filter(|(_, v)| v.is_selected()).count(),
        mesh.edges().filter(|(_, e)| e.is_selected()).count(),
        mesh.faces().filter(|(_, f)| f.is_selected()).count(),
    )
}

#[test]
fn nothing_starts_selected() {
    let (mesh, _, _) = cube();
    assert_eq!(counts(&mesh), (0, 0, 0));
    assert_eq!(mesh.selection_center(), None);
    assert!(mesh.selected_vertices().is_empty());
}

#[test]
fn vertex_mode_selects_edges_and_faces_only_when_complete() {
    let (mut mesh, v, f) = cube();
    let top = [v[4], v[5], v[6], v[7]];
    let pick =
        |vs: &[modeler_mesh::VertexId]| vs.iter().map(|&x| Element::Vertex(x)).collect::<Vec<_>>();
    mesh.select_elements(SelectMode::Vertex, &pick(&top[..1]), SelectOp::Replace)
        .unwrap();
    assert_eq!(counts(&mesh), (1, 0, 0));
    mesh.select_elements(SelectMode::Vertex, &pick(&top[1..2]), SelectOp::Add)
        .unwrap();
    assert_eq!(counts(&mesh), (2, 1, 0));
    assert!(mesh.edge(edge(&mesh, v[4], v[5])).unwrap().is_selected());
    mesh.select_elements(SelectMode::Vertex, &pick(&top[2..]), SelectOp::Add)
        .unwrap();
    assert_eq!(counts(&mesh), (4, 4, 1));
    assert!(mesh.face(f[1]).unwrap().is_selected());
    mesh.validate().unwrap();

    mesh.select_elements(SelectMode::Vertex, &pick(&top[..1]), SelectOp::Subtract)
        .unwrap();
    assert_eq!(counts(&mesh), (3, 2, 0));
    mesh.validate().unwrap();

    mesh.select_elements(SelectMode::Vertex, &pick(&v[..1]), SelectOp::Replace)
        .unwrap();
    assert_eq!(counts(&mesh), (1, 0, 0));
}

#[test]
fn edge_mode_selects_vertices_and_completes_faces() {
    let (mut mesh, v, f) = cube();
    let ring =
        [(v[4], v[5]), (v[5], v[6]), (v[6], v[7]), (v[7], v[4])].map(|(a, b)| edge(&mesh, a, b));
    mesh.select_elements(
        SelectMode::Edge,
        &[Element::Edge(ring[0])],
        SelectOp::Replace,
    )
    .unwrap();
    assert_eq!(counts(&mesh), (2, 1, 0));
    // Two opposite edges have all four vertices, but in edge mode the face needs its edges.
    mesh.select_elements(SelectMode::Edge, &[Element::Edge(ring[2])], SelectOp::Add)
        .unwrap();
    assert_eq!(counts(&mesh), (4, 2, 0));
    mesh.select_elements(
        SelectMode::Edge,
        &[Element::Edge(ring[1]), Element::Edge(ring[3])],
        SelectOp::Add,
    )
    .unwrap();
    assert_eq!(counts(&mesh), (4, 4, 1));
    assert!(mesh.face(f[1]).unwrap().is_selected());
    mesh.validate().unwrap();
}

#[test]
fn face_mode_selects_their_parts_and_shared_parts_stay() {
    let (mut mesh, _, f) = cube();
    mesh.select_elements(SelectMode::Face, &[Element::Face(f[1])], SelectOp::Replace)
        .unwrap();
    assert_eq!(counts(&mesh), (4, 4, 1));
    mesh.select_elements(SelectMode::Face, &[Element::Face(f[2])], SelectOp::Add)
        .unwrap();
    assert_eq!(counts(&mesh), (6, 7, 2));
    mesh.select_elements(SelectMode::Face, &[Element::Face(f[1])], SelectOp::Subtract)
        .unwrap();
    assert_eq!(counts(&mesh), (4, 4, 1));
    mesh.validate().unwrap();
}

#[test]
fn switching_modes_keeps_only_what_the_new_mode_can_hold() {
    let (mut mesh, v, _) = cube();
    mesh.select_elements(
        SelectMode::Vertex,
        &[
            Element::Vertex(v[0]),
            Element::Vertex(v[1]),
            Element::Vertex(v[6]),
        ],
        SelectOp::Replace,
    )
    .unwrap();
    assert_eq!(counts(&mesh), (3, 1, 0));
    mesh.flush_selection(SelectMode::Edge);
    assert_eq!(
        counts(&mesh),
        (2, 1, 0),
        "the lone vertex has no selected edge"
    );
    mesh.flush_selection(SelectMode::Face);
    assert_eq!(counts(&mesh), (0, 0, 0));
    mesh.validate().unwrap();
}

#[test]
fn all_none_and_invert() {
    let (mut mesh, v, f) = cube();
    mesh.select_all();
    assert_eq!(counts(&mesh), (8, 12, 6));
    mesh.validate().unwrap();
    mesh.deselect_all();
    assert_eq!(counts(&mesh), (0, 0, 0));

    mesh.select_elements(
        SelectMode::Vertex,
        &[Element::Vertex(v[0])],
        SelectOp::Replace,
    )
    .unwrap();
    mesh.invert_selection(SelectMode::Vertex);
    assert_eq!(counts(&mesh).0, 7);
    assert!(!mesh.vertex(v[0]).unwrap().is_selected());
    mesh.validate().unwrap();

    mesh.select_elements(SelectMode::Face, &[Element::Face(f[0])], SelectOp::Replace)
        .unwrap();
    mesh.invert_selection(SelectMode::Face);
    assert_eq!(counts(&mesh).2, 5);
    assert!(!mesh.face(f[0]).unwrap().is_selected());
    mesh.validate().unwrap();

    let e = edge(&mesh, v[0], v[1]);
    mesh.select_elements(SelectMode::Edge, &[Element::Edge(e)], SelectOp::Replace)
        .unwrap();
    mesh.invert_selection(SelectMode::Edge);
    assert_eq!(counts(&mesh).1, 11);
    mesh.validate().unwrap();
}

#[test]
fn bad_selections_are_refused_without_changing_anything() {
    let (mut mesh, v, f) = cube();
    mesh.select_all();
    let before = mesh.clone();
    for (mode, element) in [
        (SelectMode::Vertex, Element::Face(f[0])),
        (SelectMode::Edge, Element::Vertex(v[0])),
        (SelectMode::Face, Element::Vertex(v[0])),
    ] {
        assert_eq!(
            mesh.select_elements(mode, &[element], SelectOp::Replace),
            Err(MeshError::WrongSelectMode)
        );
        assert_eq!(mesh, before);
    }
    let dead = mesh.add_vertex([0.0; 3]).unwrap();
    mesh.remove_vertex(dead).unwrap();
    let before = mesh.clone();
    assert_eq!(
        mesh.select_elements(
            SelectMode::Vertex,
            &[Element::Vertex(dead)],
            SelectOp::Replace
        ),
        Err(MeshError::StaleVertex(dead))
    );
    assert_eq!(mesh, before);
}

#[test]
fn center_is_the_average_of_selected_vertices() {
    let (mut mesh, v, _) = cube();
    mesh.select_elements(
        SelectMode::Vertex,
        &[Element::Vertex(v[0]), Element::Vertex(v[6])],
        SelectOp::Replace,
    )
    .unwrap();
    assert_eq!(mesh.selection_center(), Some([0.5, 0.5, 0.5]));
}

#[test]
fn linked_selects_one_island_only() {
    let mut mesh = Mesh::new();
    let first = mesh
        .add_primitive(Primitive::Cube { size: 1.0 }, [0.0; 3])
        .unwrap();
    mesh.add_primitive(Primitive::Cube { size: 1.0 }, [5.0, 0.0, 0.0])
        .unwrap();
    mesh.select_elements(
        SelectMode::Face,
        &[Element::Face(first[0])],
        SelectOp::Replace,
    )
    .unwrap();
    mesh.select_linked(SelectMode::Face);
    assert_eq!(counts(&mesh), (8, 12, 6));
    assert!(
        mesh.vertices()
            .filter(|(_, v)| v.is_selected())
            .all(|(_, v)| v.position()[0] < 2.0)
    );
    mesh.validate().unwrap();
}

#[test]
fn linked_crosses_wire_edges() {
    let mut mesh = Mesh::new();
    let v: Vec<_> = (0..4)
        .map(|i| mesh.add_vertex([i as f32, 0.0, 0.0]).unwrap())
        .collect();
    mesh.add_edge(v[0], v[1]).unwrap();
    mesh.add_edge(v[1], v[2]).unwrap();
    mesh.select_elements(
        SelectMode::Vertex,
        &[Element::Vertex(v[0])],
        SelectOp::Replace,
    )
    .unwrap();
    mesh.select_linked(SelectMode::Vertex);
    assert_eq!(counts(&mesh), (3, 2, 0));
}

#[test]
fn more_and_less_grow_and_shrink() {
    let (mut mesh, v) = grid(6, 6);
    let at = |x: usize, y: usize| v[y * 7 + x];
    mesh.select_elements(
        SelectMode::Vertex,
        &[Element::Vertex(at(3, 3))],
        SelectOp::Replace,
    )
    .unwrap();
    mesh.select_more(SelectMode::Vertex);
    assert_eq!(
        counts(&mesh).0,
        9,
        "one step reaches all vertices of the touching quads"
    );
    mesh.select_more(SelectMode::Vertex);
    assert_eq!(counts(&mesh).0, 25);
    mesh.validate().unwrap();
    mesh.select_less(SelectMode::Vertex);
    assert_eq!(counts(&mesh).0, 9);
    mesh.select_less(SelectMode::Vertex);
    assert_eq!(counts(&mesh).0, 1);
    assert!(mesh.vertex(at(3, 3)).unwrap().is_selected());
    mesh.select_less(SelectMode::Vertex);
    assert_eq!(counts(&mesh).0, 0);
    mesh.select_more(SelectMode::Vertex);
    assert_eq!(counts(&mesh).0, 0, "growing nothing gives nothing");
}

#[test]
fn more_and_less_work_in_face_mode() {
    let (mut mesh, v) = grid(6, 6);
    let face = mesh
        .face_with_vertices(&[v[3 * 7 + 3], v[3 * 7 + 4], v[4 * 7 + 4], v[4 * 7 + 3]])
        .unwrap()
        .unwrap();
    mesh.select_elements(SelectMode::Face, &[Element::Face(face)], SelectOp::Replace)
        .unwrap();
    mesh.select_more(SelectMode::Face);
    assert_eq!(counts(&mesh).2, 9);
    mesh.validate().unwrap();
    mesh.select_less(SelectMode::Face);
    assert_eq!(counts(&mesh).2, 1);
    mesh.select_less(SelectMode::Face);
    assert_eq!(counts(&mesh).2, 0);
}

#[test]
fn less_keeps_a_fully_selected_closed_shape() {
    let (mut mesh, _, _) = cube();
    mesh.select_all();
    mesh.select_less(SelectMode::Face);
    assert_eq!(counts(&mesh), (8, 12, 6));
}

#[test]
fn duplicate_copies_faces_and_selects_the_copy() {
    let (mut mesh, _, f) = cube();
    mesh.select_elements(SelectMode::Face, &[Element::Face(f[1])], SelectOp::Replace)
        .unwrap();
    mesh.duplicate_selected(SelectMode::Face).unwrap();
    mesh.validate().unwrap();
    assert_eq!(
        (mesh.vertex_count(), mesh.edge_count(), mesh.face_count()),
        (12, 16, 7)
    );
    assert_eq!(counts(&mesh), (4, 4, 1));
    assert!(
        !mesh.face(f[1]).unwrap().is_selected(),
        "the original is deselected"
    );
}

#[test]
fn duplicate_keeps_winding_and_a_whole_shape_stays_closed() {
    let (mut mesh, _, _) = cube();
    mesh.select_all();
    mesh.duplicate_selected(SelectMode::Vertex).unwrap();
    mesh.validate().unwrap();
    assert_eq!(
        (mesh.vertex_count(), mesh.edge_count(), mesh.face_count()),
        (16, 24, 12)
    );
    assert_eq!(mesh.open_edge_count(), 0);
    assert_eq!(counts(&mesh), (8, 12, 6));
    let normals: HashSet<[i32; 3]> = mesh
        .faces()
        .map(|(id, _)| {
            mesh.face_normal(id)
                .unwrap()
                .unwrap()
                .map(|c| c.round() as i32)
        })
        .collect();
    assert_eq!(normals.len(), 6);
}

#[test]
fn duplicate_copies_loose_parts_and_nothing_selected_does_nothing() {
    let mut mesh = Mesh::new();
    let a = mesh.add_vertex([0.0; 3]).unwrap();
    let b = mesh.add_vertex([1.0; 3]).unwrap();
    let c = mesh.add_vertex([2.0; 3]).unwrap();
    let e = mesh.add_edge(a, b).unwrap();
    let before = mesh.clone();
    mesh.duplicate_selected(SelectMode::Vertex).unwrap();
    assert_eq!(mesh, before);

    mesh.select_elements(SelectMode::Edge, &[Element::Edge(e)], SelectOp::Replace)
        .unwrap();
    mesh.duplicate_selected(SelectMode::Edge).unwrap();
    assert_eq!((mesh.vertex_count(), mesh.edge_count()), (5, 2));
    mesh.validate().unwrap();

    mesh.select_elements(SelectMode::Vertex, &[Element::Vertex(c)], SelectOp::Replace)
        .unwrap();
    mesh.duplicate_selected(SelectMode::Vertex).unwrap();
    assert_eq!((mesh.vertex_count(), mesh.edge_count()), (6, 2));
    mesh.validate().unwrap();
}

fn signed_volume(mesh: &Mesh) -> f64 {
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
fn mirror_reflects_and_keeps_faces_facing_outward() {
    let mut mesh = Mesh::new();
    mesh.add_primitive(
        Primitive::Cone {
            radius: 1.0,
            height: 2.0,
            segments: 8,
        },
        [3.0, 0.0, 0.0],
    )
    .unwrap();
    mesh.select_all();
    let center = mesh.selection_center().unwrap();
    let volume = signed_volume(&mesh);
    assert!(volume > 0.0);
    for axis in 0..3 {
        let before = mesh.clone();
        mesh.mirror_selected(axis, center).unwrap();
        mesh.validate().unwrap();
        assert!(
            (signed_volume(&mesh) - volume).abs() < 1e-3,
            "axis {axis} turned it inside out"
        );
        mesh.mirror_selected(axis, center).unwrap();
        for ((_, a), (_, b)) in mesh.vertices().zip(before.vertices()) {
            for i in 0..3 {
                assert!((a.position()[i] - b.position()[i]).abs() < 1e-4);
            }
        }
    }
    // The cone's tip must really change sides on Z.
    mesh.mirror_selected(2, [0.0; 3]).unwrap();
    let lowest = mesh
        .vertices()
        .map(|(_, v)| v.position()[2])
        .fold(f32::MAX, f32::min);
    let tip_count = mesh
        .vertices()
        .filter(|(_, v)| v.position()[2] == lowest)
        .count();
    assert_eq!(tip_count, 1, "the tip now points down");
    assert_eq!(
        mesh.mirror_selected(3, [0.0; 3]),
        Err(MeshError::InvalidAxis)
    );
}

#[test]
fn selection_is_undone_with_everything_else() {
    let (mesh, v, _) = cube();
    let mut doc = Document::new(mesh);
    let start = doc.mesh().clone();
    doc.edit("select", |m| {
        m.select_elements(
            SelectMode::Vertex,
            &[Element::Vertex(v[0])],
            SelectOp::Replace,
        )
    })
    .unwrap();
    assert_eq!(doc.undo_count(), 1);
    doc.edit("move", |m| m.duplicate_selected(SelectMode::Vertex))
        .unwrap();
    let end = doc.mesh().clone();
    doc.undo();
    assert_eq!(counts(doc.mesh()), (1, 0, 0));
    doc.undo();
    assert_eq!(*doc.mesh(), start);
    doc.redo();
    doc.redo();
    assert_eq!(*doc.mesh(), end);
}

#[test]
fn selecting_what_is_already_selected_makes_no_undo_step() {
    let (mesh, v, _) = cube();
    let mut doc = Document::new(mesh);
    let pick = [Element::Vertex(v[0])];
    doc.edit("first", |m| {
        m.select_elements(SelectMode::Vertex, &pick, SelectOp::Replace)
    })
    .unwrap();
    doc.edit("again", |m| {
        m.select_elements(SelectMode::Vertex, &pick, SelectOp::Replace)
    })
    .unwrap();
    assert_eq!(doc.undo_count(), 1);
}

#[test]
fn edits_keep_the_selection_consistent() {
    let (mut mesh, v, f) = cube();
    mesh.select_all();
    mesh.split_edge(edge(&mesh, v[0], v[1]), 0.5).unwrap();
    mesh.validate().unwrap();
    let (new_face, cut) = mesh.split_face(f[1], v[4], v[6]).unwrap();
    mesh.validate().unwrap();
    assert!(mesh.face(new_face).unwrap().is_selected());
    assert!(mesh.edge(cut).unwrap().is_selected());
    mesh.join_faces(cut).unwrap();
    mesh.validate().unwrap();
    mesh.flip_face(f[2]).unwrap();
    mesh.validate().unwrap();
    assert_eq!(counts(&mesh).2, 6);

    // Joining a selected and an unselected face gives an unselected face.
    let (mut mesh, v, f) = cube();
    mesh.select_elements(SelectMode::Face, &[Element::Face(f[1])], SelectOp::Replace)
        .unwrap();
    let shared = edge(&mesh, v[4], v[5]);
    let kept = mesh.join_faces(shared).unwrap();
    mesh.validate().unwrap();
    assert!(!mesh.face(kept).unwrap().is_selected());
    // A vertex removal takes its selection with it.
    let (mut mesh, v, _) = cube();
    mesh.select_all();
    mesh.remove_vertex(v[0]).unwrap();
    mesh.validate().unwrap();
}
