//! Edge loops and rings, loop cut, edge slide and bridge.

mod common;
use std::collections::HashSet;

use common::{cube, edge, grid, is_closed_and_consistent};
use mixer_mesh::{
    Document, EdgeId, Element, MAX_LOOP_CUTS, Mesh, MeshError, Primitive, SelectMode, SelectOp,
    VertexId,
};

fn position(mesh: &Mesh, v: VertexId) -> [f32; 3] {
    mesh.vertex(v).unwrap().position()
}

fn ends(mesh: &Mesh, e: EdgeId) -> [[f32; 3]; 2] {
    mesh.edge(e).unwrap().vertices().map(|v| position(mesh, v))
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

fn counts(mesh: &Mesh) -> (usize, usize, usize) {
    (mesh.vertex_count(), mesh.edge_count(), mesh.face_count())
}

fn signed_volume(mesh: &Mesh) -> f64 {
    let mut total = 0.0;
    for (face, _) in mesh.faces() {
        for t in mesh.face_triangles(face).unwrap() {
            let [a, b, c] = t.map(|v| position(mesh, v).map(f64::from));
            total += (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                + a[2] * (b[0] * c[1] - b[1] * c[0]))
                / 6.0;
        }
    }
    total
}

fn cylinder(segments: u32) -> (Mesh, Vec<VertexId>) {
    let mut mesh = Mesh::new();
    let (v, _) = mesh
        .add_primitive_parts(
            Primitive::Cylinder {
                radius: 1.0,
                height: 2.0,
                segments,
            },
            [0.0; 3],
        )
        .unwrap();
    (mesh, v)
}

/// True if consecutive edges in the list share a vertex, and, for a closed list, the
/// last meets the first.
fn connected_in_order(mesh: &Mesh, edges: &[EdgeId], closed: bool) -> bool {
    let shares = |a: EdgeId, b: EdgeId| {
        let [p, q] = mesh.edge(a).unwrap().vertices();
        mesh.edge(b).unwrap().touches(p) || mesh.edge(b).unwrap().touches(q)
    };
    edges.windows(2).all(|w| shares(w[0], w[1]))
        && (!closed || edges.len() < 3 || shares(edges[edges.len() - 1], edges[0]))
}

#[test]
fn a_loop_runs_straight_through_four_edge_vertices_and_stops_elsewhere() {
    // Row 1 of a 4 by 3 grid: four edges, ending at the border.
    let (mesh, v) = grid(4, 3);
    let at = |x: usize, y: usize| v[y * 5 + x];
    let start = edge(&mesh, at(1, 1), at(2, 1));
    let found = mesh.edge_loop(start).unwrap();
    assert_eq!(found.len(), 4);
    assert!(connected_in_order(&mesh, &found, false));
    for e in &found {
        let [a, b] = ends(&mesh, *e);
        assert!(close(a[1], 1.0) && close(b[1], 1.0));
    }
    // Any edge of the loop finds the same loop.
    let again: HashSet<_> = mesh.edge_loop(found[3]).unwrap().into_iter().collect();
    assert_eq!(again, found.iter().copied().collect());
}

#[test]
fn a_border_edge_finds_the_whole_border() {
    let (mesh, v) = grid(4, 3);
    let found = mesh.edge_loop(edge(&mesh, v[0], v[1])).unwrap();
    assert_eq!(found.len(), 14);
    assert!(connected_in_order(&mesh, &found, true));
}

#[test]
fn sphere_loops_go_around_and_from_pole_to_pole() {
    let mut mesh = Mesh::new();
    mesh.add_primitive(
        Primitive::UvSphere {
            radius: 1.0,
            segments: 12,
            rings: 6,
        },
        [0.0; 3],
    )
    .unwrap();
    // An edge at the equator: both ends at height 0.
    let flat = mesh
        .edges()
        .map(|(id, _)| id)
        .find(|&e| ends(&mesh, e).iter().all(|p| close(p[2], 0.0)))
        .unwrap();
    let around = mesh.edge_loop(flat).unwrap();
    assert_eq!(around.len(), 12);
    assert!(connected_in_order(&mesh, &around, true));

    // An upright edge: its loop runs from one pole to the other.
    let upright = mesh
        .edges()
        .map(|(id, _)| id)
        .find(|&e| {
            let [a, b] = ends(&mesh, e);
            close(a[0].atan2(a[1]), b[0].atan2(b[1])) && !close(a[2], b[2])
        })
        .unwrap();
    let meridian = mesh.edge_loop(upright).unwrap();
    assert_eq!(meridian.len(), 6);
    assert!(connected_in_order(&mesh, &meridian, false));
}

#[test]
fn a_cube_edge_has_no_loop_beyond_itself_but_a_ring_around_the_cube() {
    let (mesh, v, _) = cube();
    let e = edge(&mesh, v[0], v[1]);
    assert_eq!(mesh.edge_loop(e).unwrap(), vec![e]);
    let ring = mesh.edge_ring(e).unwrap();
    assert_eq!(ring.len(), 4);
    assert_eq!(mesh.face_loop(e).unwrap().len(), 4);
    // The ring's edges are parallel to the first one.
    for r in ring {
        let [a, b] = ends(&mesh, r);
        assert!(close(a[1], b[1]) && close(a[2], b[2]), "{a:?} {b:?}");
    }
}

#[test]
fn rings_stop_at_faces_that_are_not_four_sided() {
    let (mesh, v) = cylinder(8);
    // An upright side edge: the ring goes all the way around the side.
    let side = edge(&mesh, v[0], v[8]);
    assert_eq!(mesh.edge_ring(side).unwrap().len(), 8);
    assert_eq!(mesh.face_loop(side).unwrap().len(), 8);
    // A rim edge: across one side face to the other rim, then the caps stop it.
    let rim = edge(&mesh, v[0], v[1]);
    assert_eq!(mesh.edge_ring(rim).unwrap().len(), 2);
    assert_eq!(mesh.face_loop(rim).unwrap().len(), 1);
}

#[test]
fn loose_edges_form_loops_but_no_rings() {
    let mut mesh = Mesh::new();
    let v: Vec<_> = (0..5)
        .map(|i| mesh.add_vertex([i as f32, 0.0, 0.0]).unwrap())
        .collect();
    let edges: Vec<_> = v
        .windows(2)
        .map(|w| mesh.add_edge(w[0], w[1]).unwrap())
        .collect();
    assert_eq!(mesh.edge_loop(edges[1]).unwrap(), edges);
    assert_eq!(mesh.edge_ring(edges[1]).unwrap(), vec![edges[1]]);
    assert!(mesh.face_loop(edges[1]).unwrap().is_empty());
}

#[test]
fn selecting_a_loop_in_each_mode() {
    let (mut mesh, v) = cylinder(8);
    let side = edge(&mesh, v[0], v[8]);
    let rim = edge(&mesh, v[0], v[1]);

    mesh.select_loop(SelectMode::Edge, rim, SelectOp::Replace)
        .unwrap();
    assert_eq!(mesh.edges().filter(|(_, e)| e.is_selected()).count(), 8);
    // The whole rim is selected, so the cap inside it is too.
    assert_eq!(mesh.faces().filter(|(_, f)| f.is_selected()).count(), 1);

    // Face mode selects the band of side faces across the edge.
    mesh.select_loop(SelectMode::Face, side, SelectOp::Replace)
        .unwrap();
    assert_eq!(mesh.faces().filter(|(_, f)| f.is_selected()).count(), 8);

    mesh.select_loop(SelectMode::Vertex, rim, SelectOp::Replace)
        .unwrap();
    assert_eq!(mesh.selected_vertices().len(), 8);
    // Adding keeps what was there.
    let top = edge(&mesh, v[8], v[9]);
    mesh.select_loop(SelectMode::Vertex, top, SelectOp::Add)
        .unwrap();
    assert_eq!(mesh.selected_vertices().len(), 16);
    mesh.validate().unwrap();
}

#[test]
fn the_loop_and_ring_buttons_grow_the_selection() {
    let (mut mesh, v) = cylinder(8);
    mesh.select_elements(
        SelectMode::Edge,
        &[Element::Edge(edge(&mesh, v[0], v[1]))],
        SelectOp::Replace,
    )
    .unwrap();
    mesh.select_loops_of_selection(SelectMode::Edge, false)
        .unwrap();
    assert_eq!(mesh.edges().filter(|(_, e)| e.is_selected()).count(), 8);

    mesh.select_elements(
        SelectMode::Edge,
        &[Element::Edge(edge(&mesh, v[0], v[8]))],
        SelectOp::Replace,
    )
    .unwrap();
    mesh.select_loops_of_selection(SelectMode::Edge, true)
        .unwrap();
    assert_eq!(mesh.edges().filter(|(_, e)| e.is_selected()).count(), 8);

    assert_eq!(
        mesh.select_loops_of_selection(SelectMode::Face, false),
        Err(MeshError::NeedsEdgeMode)
    );
    mesh.deselect_all();
    assert_eq!(
        mesh.select_loops_of_selection(SelectMode::Edge, true),
        Err(MeshError::NothingSelected)
    );
}

#[test]
fn loop_cut_around_a_cylinder_adds_rings_and_keeps_it_closed() {
    for cuts in [1, 3] {
        let (mut mesh, v) = cylinder(8);
        let before = counts(&mesh);
        let volume = signed_volume(&mesh);
        let side = edge(&mesh, v[0], v[8]);
        let made = mesh.loop_cut(side, cuts, SelectMode::Edge).unwrap();
        mesh.validate().unwrap();
        assert_eq!(made.len(), 8 * cuts);
        assert_eq!(
            counts(&mesh),
            (
                before.0 + 8 * cuts,
                before.1 + 16 * cuts,
                before.2 + 8 * cuts
            )
        );
        assert!(is_closed_and_consistent(&mesh));
        assert!((signed_volume(&mesh) - volume).abs() < 1e-4);
        // Evenly spaced, level cuts: the cylinder runs from -1 to 1.
        let heights: HashSet<i32> = made
            .iter()
            .flat_map(|&e| ends(&mesh, e))
            .map(|p| (p[2] * 1000.0).round() as i32)
            .collect();
        let expected: HashSet<i32> = (1..=cuts)
            .map(|k| ((-1.0 + 2.0 * k as f32 / (cuts + 1) as f32) * 1000.0).round() as i32)
            .collect();
        assert_eq!(heights, expected);
        // The new edges are the selection.
        let selected: HashSet<_> = mesh
            .edges()
            .filter(|(_, e)| e.is_selected())
            .map(|(id, _)| id)
            .collect();
        assert_eq!(selected, made.iter().copied().collect());
    }
}

#[test]
fn loop_cut_on_an_open_grid_lines_up_whatever_way_edges_point() {
    let (mut mesh, v) = grid(4, 3);
    let at = |x: usize, y: usize| v[y * 5 + x];
    let e = edge(&mesh, at(1, 0), at(2, 0));
    let made = mesh.loop_cut(e, 2, SelectMode::Edge).unwrap();
    mesh.validate().unwrap();
    assert_eq!(made.len(), 6);
    for e in made {
        let [a, b] = ends(&mesh, e);
        assert!(
            close(a[0], b[0]),
            "the cut must run straight up: {a:?} {b:?}"
        );
        assert!(close(a[0], 1.0 + 1.0 / 3.0) || close(a[0], 1.0 + 2.0 / 3.0));
    }
    assert_eq!(mesh.face_count(), 12 + 6);
}

#[test]
fn loop_cut_lines_up_on_a_cube_whose_edges_point_every_way() {
    let (mesh, v, _) = cube();
    for (a, b) in [(0, 1), (0, 3), (0, 4), (6, 7), (2, 6)] {
        let mut copy = mesh.clone();
        let made = copy
            .loop_cut(edge(&mesh, v[a], v[b]), 1, SelectMode::Edge)
            .unwrap();
        copy.validate().unwrap();
        assert!(is_closed_and_consistent(&copy));
        assert_eq!(made.len(), 4);
        // Every new vertex sits halfway along an edge of the old cube, in one plane.
        let [p, q] = [position(&mesh, v[a]), position(&mesh, v[b])];
        let axis = (0..3).find(|&i| !close(p[i], q[i])).unwrap();
        for e in made {
            for point in ends(&copy, e) {
                assert!(close(point[axis], (p[axis] + q[axis]) / 2.0), "{point:?}");
            }
        }
    }
    mesh.validate().unwrap();
}

#[test]
fn the_preview_shows_exactly_the_lines_the_cut_makes() {
    let (mut mesh, v) = cylinder(6);
    let side = edge(&mesh, v[2], v[8]);
    let preview = mesh.loop_cut_preview(side, 2).unwrap();
    let made = mesh.loop_cut(side, 2, SelectMode::Edge).unwrap();
    assert_eq!(preview.len(), made.len());
    let key = |[a, b]: [[f32; 3]; 2]| {
        let mut k = [a, b].map(|p| p.map(|c| (c * 1000.0).round() as i32));
        k.sort();
        k
    };
    let shown: HashSet<_> = preview.into_iter().map(key).collect();
    let real: HashSet<_> = made.into_iter().map(|e| key(ends(&mesh, e))).collect();
    assert_eq!(shown, real);
}

#[test]
fn loop_cut_refusals_change_nothing() {
    let mut mesh = Mesh::new();
    let a = mesh.add_vertex([0.0; 3]).unwrap();
    let b = mesh.add_vertex([1.0, 0.0, 0.0]).unwrap();
    let wire = mesh.add_edge(a, b).unwrap();
    let before = mesh.clone();
    assert_eq!(
        mesh.loop_cut(wire, 1, SelectMode::Edge),
        Err(MeshError::CannotLoopCut)
    );
    assert_eq!(
        mesh.loop_cut_preview(wire, 1),
        Err(MeshError::CannotLoopCut)
    );
    let (mut cyl, v) = cylinder(5);
    let side = edge(&cyl, v[0], v[5]);
    for bad in [0, MAX_LOOP_CUTS + 1] {
        assert_eq!(
            cyl.loop_cut(side, bad, SelectMode::Edge),
            Err(MeshError::InvalidCutCount)
        );
    }
    assert!(cyl.loop_cut(side, MAX_LOOP_CUTS, SelectMode::Edge).is_ok());
    cyl.validate().unwrap();
    assert_eq!(mesh, before);
}

#[test]
fn loop_cut_in_face_mode_selects_no_faces_and_undoes_exactly() {
    let (mesh, v) = cylinder(8);
    let side = edge(&mesh, v[0], v[8]);
    let mut doc = Document::new(mesh.clone());
    doc.edit("cut", |m| m.loop_cut(side, 2, SelectMode::Face).map(|_| ()))
        .unwrap();
    doc.mesh().validate().unwrap();
    assert_eq!(
        doc.mesh().faces().filter(|(_, f)| f.is_selected()).count(),
        0
    );
    doc.undo();
    assert_eq!(*doc.mesh(), mesh);
}

#[test]
fn slide_rails_point_to_both_sides_consistently() {
    let (mesh, v) = grid(4, 2);
    let at = |x: usize, y: usize| v[y * 5 + x];
    let row = mesh.edge_loop(edge(&mesh, at(1, 1), at(2, 1))).unwrap();
    let rails = mesh.edge_slide_rails(&row).unwrap();
    assert_eq!(rails.len(), 5);
    let mut sides = [HashSet::new(), HashSet::new()];
    for (vertex, start, targets) in rails {
        assert_eq!(start, position(&mesh, vertex));
        for (side, target) in targets.into_iter().enumerate() {
            let target = target.expect("every vertex has neighbors above and below");
            assert!(close(target[0], start[0]), "rails run straight across");
            sides[side].insert((target[1] * 10.0).round() as i32);
        }
    }
    // One side all goes to y = 0 and the other to y = 2.
    assert_eq!(sides[0].len(), 1);
    assert_eq!(sides[1].len(), 1);
    assert_ne!(sides[0], sides[1]);
}

#[test]
fn slide_rails_on_a_closed_loop_after_a_cut() {
    let (mut mesh, v) = cylinder(8);
    let made = mesh
        .loop_cut(edge(&mesh, v[0], v[8]), 1, SelectMode::Edge)
        .unwrap();
    let rails = mesh.edge_slide_rails(&made).unwrap();
    assert_eq!(rails.len(), 8);
    let side_heights: Vec<HashSet<i32>> = (0..2)
        .map(|side| {
            rails
                .iter()
                .map(|(_, _, t)| (t[side].unwrap()[2] * 10.0).round() as i32)
                .collect()
        })
        .collect();
    let mut both: Vec<i32> = side_heights.iter().flatten().copied().collect();
    both.sort();
    assert_eq!(both, vec![-10, 10]);
}

#[test]
fn slide_refusals() {
    let (mesh, v) = grid(2, 2);
    let at = |x: usize, y: usize| v[y * 3 + x];
    // Three edges meeting at the middle vertex branch.
    let branching = [
        edge(&mesh, at(0, 1), at(1, 1)),
        edge(&mesh, at(1, 1), at(2, 1)),
        edge(&mesh, at(1, 1), at(1, 2)),
    ];
    assert_eq!(mesh.edge_slide_rails(&branching), Err(MeshError::NotLoops));
    assert_eq!(mesh.edge_slide_rails(&[]), Err(MeshError::NothingSelected));
    let mut wire = Mesh::new();
    let a = wire.add_vertex([0.0; 3]).unwrap();
    let b = wire.add_vertex([1.0, 0.0, 0.0]).unwrap();
    let e = wire.add_edge(a, b).unwrap();
    assert_eq!(wire.edge_slide_rails(&[e]), Err(MeshError::NothingToSlide));
}

/// Two cylinders end to end, with the facing caps selected.
fn two_cylinders(turn: f32) -> Mesh {
    let mut mesh = Mesh::new();
    let lower = mesh
        .add_primitive(
            Primitive::Cylinder {
                radius: 1.0,
                height: 2.0,
                segments: 8,
            },
            [0.0; 3],
        )
        .unwrap();
    let upper = mesh
        .add_primitive(
            Primitive::Cylinder {
                radius: 1.0,
                height: 2.0,
                segments: 8,
            },
            [0.0, 0.0, 4.0],
        )
        .unwrap();
    // Turn the upper cylinder about Z, so that its vertex order no longer lines up.
    let mut upper_vertices = HashSet::new();
    for &face in &upper {
        upper_vertices.extend(mesh.face_vertices(face).unwrap());
    }
    for vertex in upper_vertices {
        let p = position(&mesh, vertex);
        let (s, c) = turn.sin_cos();
        mesh.set_vertex_position(vertex, [p[0] * c - p[1] * s, p[0] * s + p[1] * c, p[2]])
            .unwrap();
    }
    let cap = |faces: &[mixer_mesh::FaceId], height: f32| {
        *faces
            .iter()
            .find(|&&f| {
                mesh.face_vertices(f)
                    .unwrap()
                    .all(|v| close(position(&mesh, v)[2], height))
            })
            .unwrap()
    };
    let caps = [
        Element::Face(cap(&lower, 1.0)),
        Element::Face(cap(&upper, 3.0)),
    ];
    mesh.select_elements(SelectMode::Face, &caps, SelectOp::Replace)
        .unwrap();
    mesh
}

#[test]
fn bridging_two_facing_caps_makes_one_closed_tube_without_a_twist() {
    // Turning the upper part by 3 segments and a bit must not twist the bridge.
    for turn in [0.0, 1.0, 3.0 * std::f32::consts::TAU / 8.0 + 0.1] {
        let mut mesh = two_cylinders(turn);
        let before = counts(&mesh);
        mesh.bridge_selected_loops(SelectMode::Face).unwrap();
        mesh.validate().unwrap();
        assert_eq!(counts(&mesh), (before.0, before.1 + 8, before.2 - 2 + 8));
        assert!(is_closed_and_consistent(&mesh));
        assert!(signed_volume(&mesh) > 0.0, "the tube must face outward");
        let selected: Vec<_> = mesh.faces().filter(|(_, f)| f.is_selected()).collect();
        assert_eq!(selected.len(), 8);
        // No twist: each bridge edge is short, about the gap of 2.
        for (id, _) in mesh.edges() {
            let [a, b] = ends(&mesh, id);
            if close(a[2], 1.0) != close(b[2], 1.0) && (a[2] - b[2]).abs() > 1.5 {
                let length = (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f32>().sqrt();
                assert!(length < 2.2, "a twisted bridge edge, {length} long");
            }
        }
    }
}

#[test]
fn bridging_two_loose_lines_makes_a_strip() {
    let mut mesh = Mesh::new();
    let mut lines = Vec::new();
    for y in [0.0, 1.0] {
        // The second line runs backwards, which the bridge must notice.
        let xs: Vec<f32> = if y == 0.0 {
            vec![0.0, 1.0, 2.0, 3.0]
        } else {
            vec![3.0, 2.0, 1.0, 0.0]
        };
        let v: Vec<_> = xs
            .iter()
            .map(|&x| mesh.add_vertex([x, y, 0.0]).unwrap())
            .collect();
        for w in v.windows(2) {
            lines.push(Element::Edge(mesh.add_edge(w[0], w[1]).unwrap()));
        }
    }
    mesh.select_elements(SelectMode::Edge, &lines, SelectOp::Replace)
        .unwrap();
    mesh.bridge_selected_loops(SelectMode::Edge).unwrap();
    mesh.validate().unwrap();
    assert_eq!(mesh.face_count(), 3);
    for (face, _) in mesh.faces() {
        // Each strip face is a unit square, not a crossed shape.
        let area: f32 = mesh
            .face_triangles(face)
            .unwrap()
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|v| position(&mesh, v));
                ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])).abs() / 2.0
            })
            .sum();
        assert!(close(area, 1.0), "area {area}");
    }
}

#[test]
fn bridge_refusals_change_nothing() {
    // One loop only.
    let (mut mesh, v) = cylinder(6);
    mesh.select_loop(SelectMode::Edge, edge(&mesh, v[0], v[1]), SelectOp::Replace)
        .unwrap();
    let before = mesh.clone();
    assert_eq!(
        mesh.bridge_selected_loops(SelectMode::Edge),
        Err(MeshError::BridgeNeedsTwoLoops)
    );
    assert_eq!(mesh, before);

    // Two loops of different lengths.
    let mut mesh = Mesh::new();
    for (count, z) in [(6u32, 0.0), (8, 2.0)] {
        mesh.add_primitive(
            Primitive::Circle {
                radius: 1.0,
                segments: count,
            },
            [0.0, 0.0, z],
        )
        .unwrap();
    }
    mesh.select_all();
    let before = mesh.clone();
    assert_eq!(
        mesh.bridge_selected_loops(SelectMode::Face),
        Err(MeshError::BridgeNeedsTwoLoops)
    );
    assert_eq!(mesh, before);

    // Nothing selected.
    mesh.deselect_all();
    assert_eq!(
        mesh.bridge_selected_loops(SelectMode::Edge),
        Err(MeshError::BridgeNeedsTwoLoops)
    );
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

/// Random loop cuts, loop selections, slides and bridges on every kind of shape. After
/// each step the mesh must be valid, a refusal must change nothing, and undo must bring
/// back each earlier state exactly.
#[test]
fn random_loop_tools_stay_valid_and_undo_exactly() {
    let shapes = [
        Primitive::Cube { size: 2.0 },
        Primitive::Cylinder {
            radius: 1.0,
            height: 2.0,
            segments: 7,
        },
        Primitive::UvSphere {
            radius: 1.0,
            segments: 8,
            rings: 5,
        },
        Primitive::Icosphere {
            radius: 1.0,
            subdivisions: 1,
        },
        Primitive::Grid {
            size: 2.0,
            columns: 3,
            rows: 2,
        },
        Primitive::Cone {
            radius: 1.0,
            height: 2.0,
            segments: 6,
        },
        Primitive::Circle {
            radius: 1.0,
            segments: 6,
        },
    ];
    let mut worked = [0usize; 5];
    for seed in 0..30u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut doc = Document::new(Mesh::new());
        doc.edit("start", |m| {
            for (i, &shape) in shapes.iter().enumerate() {
                m.add_primitive(shape, [i as f32 * 3.0, 0.0, 0.0])?;
            }
            Ok(())
        })
        .unwrap();
        let mut states = vec![doc.mesh().clone()];
        for _ in 0..40 {
            let mode = [SelectMode::Vertex, SelectMode::Edge, SelectMode::Face][rng.below(3)];
            let edges: Vec<EdgeId> = doc.mesh().edges().map(|(id, _)| id).collect();
            let picked = edges[rng.below(edges.len())];
            let other = edges[rng.below(edges.len())];
            let choice = rng.below(5);
            let cuts = 1 + rng.below(3);
            let before = doc.mesh().clone();
            let undo_before = doc.undo_count();
            let result = doc.edit("loop tool", |m| match choice {
                0 => m.loop_cut(picked, cuts, mode).map(|_| ()),
                1 => m.select_loop(mode, picked, SelectOp::Replace),
                2 => {
                    m.select_loop(SelectMode::Edge, picked, SelectOp::Replace)?;
                    m.select_loop(SelectMode::Edge, other, SelectOp::Add)?;
                    m.bridge_selected_loops(SelectMode::Edge)
                }
                3 => {
                    let ring = m.edge_loop(picked)?;
                    let rails = m.edge_slide_rails(&ring)?;
                    let t = 0.3;
                    for (vertex, start, [a, _]) in rails {
                        if let Some(a) = a {
                            let p: [f32; 3] =
                                std::array::from_fn(|i| start[i] + (a[i] - start[i]) * t);
                            m.set_vertex_position(vertex, p)?;
                        }
                    }
                    Ok(())
                }
                _ => {
                    m.select_elements(
                        SelectMode::Edge,
                        &[Element::Edge(picked)],
                        SelectOp::Replace,
                    )?;
                    m.select_loops_of_selection(SelectMode::Edge, rng.below(2) == 0)
                }
            });
            match result {
                Ok(()) => {
                    worked[choice] += 1;
                    if doc.undo_count() > undo_before {
                        states.push(before);
                    }
                }
                Err(_) => assert_eq!(*doc.mesh(), before, "a refused tool changed the mesh"),
            }
            doc.mesh().validate().unwrap();
            if doc.mesh().vertex_count() > 6000 {
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
