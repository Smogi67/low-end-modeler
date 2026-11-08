//! Delete, extrude, inset, merge, fill and subdivide, plus a random test of all of them.

mod common;
use common::{cube, edge, grid};
use mixer_mesh::{Document, Element, Mesh, MeshError, Primitive, SelectMode, SelectOp};

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

fn counts(mesh: &Mesh) -> (usize, usize, usize) {
    (mesh.vertex_count(), mesh.edge_count(), mesh.face_count())
}

fn select_face(mesh: &mut Mesh, face: mixer_mesh::FaceId) {
    mesh.select_elements(SelectMode::Face, &[Element::Face(face)], SelectOp::Replace)
        .unwrap();
}

fn selected_face_count(mesh: &Mesh) -> usize {
    mesh.faces().filter(|(_, f)| f.is_selected()).count()
}

/// Moves the selection, the way the app does after extruding.
fn move_selection(mesh: &mut Mesh, offset: [f32; 3]) {
    for vertex in mesh.selected_vertices() {
        let p = mesh.vertex(vertex).unwrap().position();
        mesh.set_vertex_position(
            vertex,
            [p[0] + offset[0], p[1] + offset[1], p[2] + offset[2]],
        )
        .unwrap();
    }
}

#[test]
fn delete_in_each_mode() {
    let (mut mesh, v, _) = cube();
    mesh.select_elements(
        SelectMode::Vertex,
        &[Element::Vertex(v[0])],
        SelectOp::Replace,
    )
    .unwrap();
    mesh.delete_selected(SelectMode::Vertex).unwrap();
    mesh.validate().unwrap();
    assert_eq!(counts(&mesh), (7, 9, 3));

    let (mut mesh, _, _) = cube();
    let e = edge(&mesh, v_of(&mesh, 0), v_of(&mesh, 1));
    mesh.select_elements(SelectMode::Edge, &[Element::Edge(e)], SelectOp::Replace)
        .unwrap();
    mesh.delete_selected(SelectMode::Edge).unwrap();
    mesh.validate().unwrap();
    assert_eq!(counts(&mesh), (8, 11, 4), "vertices still have other edges");

    let (mut mesh, _, _) = cube();
    let first = f_of(&mesh);
    select_face(&mut mesh, first);
    mesh.delete_selected(SelectMode::Face).unwrap();
    mesh.validate().unwrap();
    assert_eq!(
        counts(&mesh),
        (8, 12, 5),
        "edges shared with other faces stay"
    );
}

fn v_of(mesh: &Mesh, i: usize) -> mixer_mesh::VertexId {
    mesh.vertices().nth(i).unwrap().0
}
fn f_of(mesh: &Mesh) -> mixer_mesh::FaceId {
    mesh.faces().next().unwrap().0
}

#[test]
fn deleting_all_faces_of_a_shape_leaves_nothing() {
    let (mut mesh, _, _) = cube();
    mesh.select_all();
    mesh.delete_selected(SelectMode::Face).unwrap();
    mesh.validate().unwrap();
    assert_eq!(counts(&mesh), (0, 0, 0));
}

#[test]
fn deleting_with_nothing_selected_is_refused() {
    let (mut mesh, _, _) = cube();
    let before = mesh.clone();
    assert_eq!(
        mesh.delete_selected(SelectMode::Face),
        Err(MeshError::NothingSelected)
    );
    assert_eq!(mesh, before);
}

#[test]
fn extruding_a_face_grows_a_closed_shape() {
    let (mut mesh, _, f) = cube();
    select_face(&mut mesh, f[1]); // The top, facing +Z.
    assert_eq!(mesh.selected_faces_normal(), Some([0.0, 0.0, 1.0]));
    mesh.extrude_selected_faces(SelectMode::Face).unwrap();
    mesh.validate().unwrap();
    assert_eq!(counts(&mesh), (12, 20, 10));
    assert_eq!(mesh.open_edge_count(), 0);
    assert_eq!(selected_face_count(&mesh), 1);
    assert!(
        (signed_volume(&mesh) - 1.0).abs() < 1e-5,
        "nothing has moved yet"
    );

    move_selection(&mut mesh, [0.0, 0.0, 2.0]);
    mesh.validate().unwrap();
    assert!(
        (signed_volume(&mesh) - 3.0).abs() < 1e-5,
        "1 x 1 x 3, facing outward"
    );
}

#[test]
fn extruding_a_region_makes_walls_only_around_its_outline() {
    let (mut mesh, v) = grid(2, 2);
    mesh.select_all();
    mesh.extrude_selected_faces(SelectMode::Face).unwrap();
    mesh.validate().unwrap();
    // 4 original faces replaced, and 8 outline edges get walls.
    assert_eq!(mesh.face_count(), 4 + 8);
    // The old inner vertex is gone. The 8 outline vertices stay.
    assert_eq!(mesh.vertex_count(), 9 + 8);
    assert!(mesh.vertex(v[4]).is_none());
    assert_eq!(selected_face_count(&mesh), 4);
}

#[test]
fn extruding_every_face_of_a_closed_shape_keeps_it_whole() {
    let (mut mesh, _, _) = cube();
    mesh.select_all();
    mesh.extrude_selected_faces(SelectMode::Face).unwrap();
    mesh.validate().unwrap();
    assert_eq!(counts(&mesh), (8, 12, 6));
    assert_eq!(mesh.open_edge_count(), 0);
}

#[test]
fn extrude_and_inset_need_faces() {
    let (mut mesh, v, _) = cube();
    mesh.select_elements(
        SelectMode::Vertex,
        &[Element::Vertex(v[0])],
        SelectOp::Replace,
    )
    .unwrap();
    let before = mesh.clone();
    assert_eq!(
        mesh.extrude_selected_faces(SelectMode::Vertex),
        Err(MeshError::NoFacesSelected)
    );
    assert_eq!(
        mesh.inset_selected_faces(SelectMode::Vertex).err(),
        Some(MeshError::NoFacesSelected)
    );
    assert_eq!(mesh, before);
}

#[test]
fn inset_adds_a_ring_and_keeps_the_shape_closed() {
    let (mut mesh, _, f) = cube();
    select_face(&mut mesh, f[1]);
    let moving = mesh.inset_selected_faces(SelectMode::Face).unwrap();
    mesh.validate().unwrap();
    assert_eq!(counts(&mesh), (12, 20, 10));
    assert_eq!(mesh.open_edge_count(), 0);
    assert_eq!(moving.len(), 4);
    assert_eq!(selected_face_count(&mesh), 1);
    for (_, center) in &moving {
        assert_eq!(*center, [0.5, 0.5, 1.0]);
    }
    // Moving the corners halfway must not change the volume, as the top stays flat.
    for &(vertex, center) in &moving {
        let p = mesh.vertex(vertex).unwrap().position();
        mesh.set_vertex_position(vertex, std::array::from_fn(|i| (p[i] + center[i]) / 2.0))
            .unwrap();
    }
    mesh.validate().unwrap();
    assert!((signed_volume(&mesh) - 1.0).abs() < 1e-5);
}

#[test]
fn inset_treats_each_face_separately() {
    let (mut mesh, _, f) = cube();
    mesh.select_elements(
        SelectMode::Face,
        &[Element::Face(f[1]), Element::Face(f[2])],
        SelectOp::Replace,
    )
    .unwrap();
    let moving = mesh.inset_selected_faces(SelectMode::Face).unwrap();
    mesh.validate().unwrap();
    assert_eq!(moving.len(), 8);
    assert_eq!(
        mesh.face_count(),
        4 + 10,
        "4 untouched, and 5 new for each of the 2"
    );
    assert_eq!(selected_face_count(&mesh), 2);
}

#[test]
fn merging_a_cube_face_makes_a_point() {
    let (mut mesh, _, f) = cube();
    select_face(&mut mesh, f[1]);
    mesh.merge_selected_at_center(SelectMode::Face).unwrap();
    mesh.validate().unwrap();
    assert_eq!(counts(&mesh), (5, 8, 5), "a square pyramid");
    assert_eq!(mesh.open_edge_count(), 0);
    let top = mesh
        .vertices()
        .map(|(_, v)| v.position())
        .find(|p| p[2] == 1.0)
        .unwrap();
    assert_eq!(top, [0.5, 0.5, 1.0]);
    assert!(
        (signed_volume(&mesh) - 1.0 / 3.0).abs() < 1e-5,
        "still facing outward"
    );
}

#[test]
fn merging_an_edge_keeps_the_surface_closed() {
    let (mut mesh, v, _) = cube();
    mesh.select_elements(
        SelectMode::Vertex,
        &[Element::Vertex(v[4]), Element::Vertex(v[5])],
        SelectOp::Replace,
    )
    .unwrap();
    mesh.merge_selected_at_center(SelectMode::Vertex).unwrap();
    mesh.validate().unwrap();
    assert_eq!(counts(&mesh), (7, 11, 6));
    assert_eq!(mesh.open_edge_count(), 0);
    assert_eq!(mesh.selected_vertices().len(), 1);
    let euler = mesh.vertex_count() as i64 - mesh.edge_count() as i64 + mesh.face_count() as i64;
    assert_eq!(euler, 2);
}

#[test]
fn merging_keeps_wire_edges_and_needs_two_vertices() {
    let mut mesh = Mesh::new();
    let v: Vec<_> = (0..4)
        .map(|i| mesh.add_vertex([i as f32, 0.0, 0.0]).unwrap())
        .collect();
    mesh.add_edge(v[0], v[1]).unwrap();
    mesh.add_edge(v[1], v[2]).unwrap();
    mesh.add_edge(v[2], v[3]).unwrap();
    mesh.select_elements(
        SelectMode::Vertex,
        &[Element::Vertex(v[1])],
        SelectOp::Replace,
    )
    .unwrap();
    let before = mesh.clone();
    assert_eq!(
        mesh.merge_selected_at_center(SelectMode::Vertex),
        Err(MeshError::NothingSelected)
    );
    assert_eq!(mesh, before);
    mesh.select_elements(SelectMode::Vertex, &[Element::Vertex(v[2])], SelectOp::Add)
        .unwrap();
    mesh.merge_selected_at_center(SelectMode::Vertex).unwrap();
    mesh.validate().unwrap();
    assert_eq!((mesh.vertex_count(), mesh.edge_count()), (3, 2));
}

#[test]
fn merge_by_distance_joins_only_close_vertices() {
    let mut mesh = Mesh::new();
    // Two cubes sharing one touching side, so four pairs sit at the same spot.
    mesh.add_primitive(Primitive::Cube { size: 2.0 }, [0.0; 3])
        .unwrap();
    mesh.add_primitive(Primitive::Cube { size: 2.0 }, [2.0, 0.0, 0.0])
        .unwrap();
    mesh.select_all();
    let removed = mesh
        .merge_selected_by_distance(0.001, SelectMode::Vertex)
        .unwrap();
    mesh.validate().unwrap();
    assert_eq!(removed, 4);
    assert_eq!(mesh.vertex_count(), 12);
    // The two touching faces became one, since duplicate faces are not kept.
    assert_eq!(mesh.face_count(), 11);

    let before = mesh.clone();
    assert_eq!(
        mesh.merge_selected_by_distance(0.001, SelectMode::Vertex),
        Ok(0)
    );
    assert_eq!(mesh.vertex_count(), before.vertex_count());
    for bad in [0.0, -1.0, f32::NAN] {
        assert_eq!(
            mesh.merge_selected_by_distance(bad, SelectMode::Vertex),
            Err(MeshError::InvalidDistance)
        );
    }
}

#[test]
fn merge_by_distance_chains_through_close_neighbors() {
    let mut mesh = Mesh::new();
    for x in [0.0, 0.4, 0.8, 5.0] {
        mesh.add_vertex([x, 0.0, 0.0]).unwrap();
    }
    mesh.select_all();
    assert_eq!(
        mesh.merge_selected_by_distance(0.5, SelectMode::Vertex),
        Ok(2)
    );
    assert_eq!(mesh.vertex_count(), 2);
    mesh.validate().unwrap();
}

#[test]
fn fill_closes_a_hole_facing_outward() {
    let (mut mesh, v, f) = cube();
    select_face(&mut mesh, f[1]);
    mesh.delete_selected(SelectMode::Face).unwrap();
    assert_eq!(mesh.open_edge_count(), 4);
    let top = [v[4], v[5], v[6], v[7]].map(Element::Vertex);
    mesh.select_elements(SelectMode::Vertex, &top, SelectOp::Replace)
        .unwrap();
    mesh.fill_selected(SelectMode::Vertex).unwrap();
    mesh.validate().unwrap();
    assert_eq!(mesh.open_edge_count(), 0);
    assert!(
        (signed_volume(&mesh) - 1.0).abs() < 1e-5,
        "must face outward"
    );
}

#[test]
fn fill_makes_an_edge_or_an_ordered_face_from_loose_vertices() {
    let mut mesh = Mesh::new();
    // Listed out of order on purpose: a square would cross itself if used as given.
    let square = [
        [0.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
    ];
    let v: Vec<_> = square
        .iter()
        .map(|&p| mesh.add_vertex(p).unwrap())
        .collect();
    mesh.select_elements(
        SelectMode::Vertex,
        &[Element::Vertex(v[0])],
        SelectOp::Replace,
    )
    .unwrap();
    assert_eq!(
        mesh.fill_selected(SelectMode::Vertex),
        Err(MeshError::NothingSelected)
    );

    mesh.select_elements(SelectMode::Vertex, &[Element::Vertex(v[1])], SelectOp::Add)
        .unwrap();
    mesh.fill_selected(SelectMode::Vertex).unwrap();
    assert_eq!(mesh.edge_count(), 1);
    assert_eq!(
        mesh.fill_selected(SelectMode::Vertex),
        Err(MeshError::CannotFill),
        "the edge exists"
    );

    mesh.remove_edge(edge(&mesh, v[0], v[1])).unwrap();
    mesh.select_all();
    mesh.fill_selected(SelectMode::Vertex).unwrap();
    mesh.validate().unwrap();
    let (face, _) = mesh.faces().next().unwrap();
    let triangles = mesh.face_triangles(face).unwrap();
    let area: f32 = triangles
        .iter()
        .map(|t| {
            let [a, b, c] = t.map(|x| mesh.vertex(x).unwrap().position());
            ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])).abs() / 2.0
        })
        .sum();
    assert!(
        (area - 1.0).abs() < 1e-5,
        "the vertices must be in order around the square"
    );
    assert_eq!(
        mesh.fill_selected(SelectMode::Vertex),
        Err(MeshError::CannotFill),
        "the face exists"
    );
    mesh.validate().unwrap();
}

#[test]
fn subdividing_a_whole_cube_keeps_it_closed() {
    let (mut mesh, _, _) = cube();
    mesh.select_all();
    mesh.subdivide_selected(SelectMode::Face).unwrap();
    mesh.validate().unwrap();
    assert_eq!(counts(&mesh), (26, 48, 24));
    assert_eq!(mesh.open_edge_count(), 0);
    assert!((signed_volume(&mesh) - 1.0).abs() < 1e-5);
    assert_eq!(selected_face_count(&mesh), 24);
}

#[test]
fn subdividing_one_face_leaves_neighbors_whole_with_an_extra_corner() {
    let (mut mesh, _, f) = cube();
    select_face(&mut mesh, f[1]);
    mesh.subdivide_selected(SelectMode::Face).unwrap();
    mesh.validate().unwrap();
    assert_eq!(mesh.face_count(), 5 + 4);
    assert_eq!(mesh.open_edge_count(), 0);
    assert!((signed_volume(&mesh) - 1.0).abs() < 1e-5);
    assert_eq!(mesh.face(f[2]).unwrap().corner_count(), 5);
    assert_eq!(selected_face_count(&mesh), 4);
}

#[test]
fn subdividing_triangles_makes_four() {
    let mut mesh = Mesh::new();
    mesh.add_primitive(
        Primitive::Icosphere {
            radius: 1.0,
            subdivisions: 0,
        },
        [0.0; 3],
    )
    .unwrap();
    mesh.select_all();
    mesh.subdivide_selected(SelectMode::Face).unwrap();
    mesh.validate().unwrap();
    assert_eq!(counts(&mesh), (42, 120, 80));
    assert_eq!(mesh.open_edge_count(), 0);
    assert!(mesh.faces().all(|(_, f)| f.corner_count() == 3));
    assert!(signed_volume(&mesh) > 0.0);
}

#[test]
fn subdividing_edges_alone_and_nothing_at_all() {
    let (mut mesh, v, _) = cube();
    let before = mesh.clone();
    assert_eq!(
        mesh.subdivide_selected(SelectMode::Edge),
        Err(MeshError::NothingSelected)
    );
    assert_eq!(mesh, before);
    let e = edge(&mesh, v[0], v[1]);
    mesh.select_elements(SelectMode::Edge, &[Element::Edge(e)], SelectOp::Replace)
        .unwrap();
    mesh.subdivide_selected(SelectMode::Edge).unwrap();
    mesh.validate().unwrap();
    assert_eq!(mesh.vertex_count(), 9);
    assert_eq!(mesh.edge_count(), 13);
    assert_eq!(mesh.open_edge_count(), 0);
}

#[test]
fn linked_faces_finds_the_whole_part_from_any_element() {
    let mut mesh = Mesh::new();
    let first = mesh
        .add_primitive(Primitive::Cube { size: 1.0 }, [0.0; 3])
        .unwrap();
    mesh.add_primitive(Primitive::Cube { size: 1.0 }, [5.0, 0.0, 0.0])
        .unwrap();
    assert_eq!(mesh.linked_faces(&[Element::Face(first[0])]).len(), 6);
    let vertex = mesh.face_vertices(first[0]).unwrap().next().unwrap();
    assert_eq!(mesh.linked_faces(&[Element::Vertex(vertex)]).len(), 6);
    let e = mesh.face_edges(first[0]).unwrap().next().unwrap();
    assert_eq!(mesh.linked_faces(&[Element::Edge(e)]).len(), 6);
    assert!(mesh.linked_faces(&[]).is_empty());
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, limit: usize) -> usize {
        (self.next() % limit.max(1) as u64) as usize
    }
}

/// Random selections and tools on real shapes. After every step the mesh must be valid,
/// a refused tool must change nothing, and undo must restore each earlier state exactly.
#[test]
fn random_tool_use_stays_valid_and_undoes_exactly() {
    const MODES: [SelectMode; 3] = [SelectMode::Vertex, SelectMode::Edge, SelectMode::Face];
    let mut worked = [0usize; 8];
    for seed in 0..20u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut doc = Document::new(Mesh::new());
        doc.edit("start", |m| {
            m.add_primitive(Primitive::Cube { size: 2.0 }, [0.0; 3])?;
            m.add_primitive(
                Primitive::Cylinder {
                    radius: 1.0,
                    height: 2.0,
                    segments: 6,
                },
                [4.0, 0.0, 0.0],
            )?;
            m.add_primitive(
                Primitive::Grid {
                    size: 2.0,
                    columns: 2,
                    rows: 2,
                },
                [0.0, 4.0, 0.0],
            )?;
            m.add_primitive(
                Primitive::Icosphere {
                    radius: 1.0,
                    subdivisions: 0,
                },
                [0.0, 0.0, 4.0],
            )?;
            Ok(())
        })
        .unwrap();
        let mut states = vec![doc.mesh().clone()];
        for _ in 0..80 {
            let mode = MODES[rng.below(3)];
            let elements: Vec<Element> = {
                let mesh = doc.mesh();
                let count = 1 + rng.below(4);
                (0..count)
                    .filter_map(|_| match mode {
                        SelectMode::Vertex => {
                            let n = mesh.vertex_count();
                            mesh.vertices()
                                .nth(rng.below(n))
                                .map(|(id, _)| Element::Vertex(id))
                        }
                        SelectMode::Edge => {
                            let n = mesh.edge_count();
                            mesh.edges()
                                .nth(rng.below(n))
                                .map(|(id, _)| Element::Edge(id))
                        }
                        SelectMode::Face => {
                            let n = mesh.face_count();
                            mesh.faces()
                                .nth(rng.below(n))
                                .map(|(id, _)| Element::Face(id))
                        }
                    })
                    .collect()
            };
            let choice = rng.below(8);
            let before = doc.mesh().clone();
            let undo_before = doc.undo_count();
            let result = doc.edit("tool", |m| {
                m.select_elements(mode, &elements, SelectOp::Replace)?;
                if rng_grow(choice) {
                    m.select_more(mode);
                }
                match choice {
                    0 => m.delete_selected(mode),
                    1 => m.extrude_selected_faces(mode),
                    2 => m.inset_selected_faces(mode).map(|_| ()),
                    3 => m.merge_selected_at_center(mode),
                    4 => m.merge_selected_by_distance(0.5, mode).map(|_| ()),
                    5 => m.fill_selected(mode),
                    6 => m.subdivide_selected(mode),
                    _ => Ok(()),
                }
            });
            match result {
                Ok(()) => {
                    worked[choice] += 1;
                    if doc.undo_count() > undo_before {
                        states.push(before.clone());
                    }
                }
                Err(_) => assert_eq!(*doc.mesh(), before, "a refused tool changed the mesh"),
            }
            doc.mesh().validate().unwrap();
            if doc.mesh().vertex_count() > 4000 {
                break;
            }
        }
        let mut expected = states.iter().rev();
        while doc.undo_count() > 1 {
            doc.undo();
            assert_eq!(Some(doc.mesh()), expected.next(), "seed {seed}");
        }
    }
    println!("{worked:?}");
    for (choice, count) in worked.iter().enumerate() {
        assert!(*count >= 10, "tool {choice} worked only {count} times");
    }
}

fn rng_grow(choice: usize) -> bool {
    choice.is_multiple_of(2)
}
