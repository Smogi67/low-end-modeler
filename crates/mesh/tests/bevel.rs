//! Edge bevel.

mod common;
use std::collections::HashSet;

use common::{cube, edge, grid, is_closed_and_consistent};
use mixer_mesh::{
    Document, EdgeId, Element, MAX_BEVEL_SEGMENTS, Mesh, MeshError, Primitive, SelectMode,
    SelectOp, VertexId,
};

fn position(mesh: &Mesh, v: VertexId) -> [f32; 3] {
    mesh.vertex(v).unwrap().position()
}

fn volume(mesh: &Mesh) -> f64 {
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

fn counts(mesh: &Mesh) -> (usize, usize, usize) {
    (mesh.vertex_count(), mesh.edge_count(), mesh.face_count())
}

fn select_edges(mesh: &mut Mesh, edges: &[EdgeId]) {
    let elements: Vec<Element> = edges.iter().map(|&e| Element::Edge(e)).collect();
    mesh.select_elements(SelectMode::Edge, &elements, SelectOp::Replace)
        .unwrap();
}

/// Every face with two corners on an edge must run along it the opposite way to its
/// neighbor.
fn winding_agrees(mesh: &Mesh) -> bool {
    mesh.edges().all(|(id, _)| {
        let starts: Vec<VertexId> = mesh
            .edge_corners(id)
            .unwrap()
            .map(|c| mesh.corner(c).unwrap().vertex())
            .collect();
        starts.len() != 2 || starts[0] != starts[1]
    })
}

/// How many separate holes the surface has: connected runs of edges with one face.
fn hole_count(mesh: &Mesh) -> usize {
    let border: Vec<[VertexId; 2]> = mesh
        .edges()
        .filter(|&(id, _)| mesh.edge_corners(id).unwrap().count() == 1)
        .map(|(_, e)| e.vertices())
        .collect();
    let mut parent: std::collections::HashMap<VertexId, VertexId> =
        border.iter().flatten().map(|&v| (v, v)).collect();
    fn root(parent: &mut std::collections::HashMap<VertexId, VertexId>, v: VertexId) -> VertexId {
        let mut r = v;
        while parent[&r] != r {
            r = parent[&r];
        }
        parent.insert(v, r);
        r
    }
    for [a, b] in &border {
        let (ra, rb) = (root(&mut parent, *a), root(&mut parent, *b));
        parent.insert(ra, rb);
    }
    let keys: Vec<VertexId> = parent.keys().copied().collect();
    keys.into_iter()
        .map(|v| root(&mut parent, v))
        .collect::<HashSet<_>>()
        .len()
}

/// True if no vertex lies outside any face's plane, for a shape that should be convex.
fn is_convex(mesh: &Mesh) -> bool {
    mesh.faces().all(|(face, _)| {
        let Some(normal) = mesh.face_normal(face).unwrap() else {
            return false;
        };
        let on = position(mesh, mesh.face_vertices(face).unwrap().next().unwrap());
        mesh.vertices().all(|(_, v)| {
            let p = v.position();
            (0..3).map(|i| (p[i] - on[i]) * normal[i]).sum::<f32>() < 1e-4
        })
    })
}

fn bevel(mesh: &mut Mesh, width: f32, segments: usize) -> mixer_mesh::Bevel {
    let result = mesh
        .bevel_selected_edges(width, segments, SelectMode::Edge)
        .unwrap();
    mesh.validate().unwrap();
    result
}

#[test]
fn one_cube_edge_cut_flat_removes_an_exact_prism() {
    let (mut mesh, v, _) = cube();
    let picked = edge(&mesh, v[5], v[6]);
    select_edges(&mut mesh, &[picked]);
    let w = 0.3;
    let result = bevel(&mut mesh, w, 1);
    assert_eq!(result.width, w);
    // Two vertices become four, and one new face appears.
    assert_eq!(counts(&mesh), (10, 15, 7));
    assert!(is_closed_and_consistent(&mesh));
    assert!(is_convex(&mesh));
    let expected = 1.0 - f64::from(w * w) / 2.0;
    assert!((volume(&mesh) - expected).abs() < 1e-5, "{}", volume(&mesh));
    assert_eq!(mesh.faces().filter(|(_, f)| f.is_selected()).count(), 1);
}

#[test]
fn one_cube_edge_cut_round_lies_on_a_quarter_circle() {
    for segments in [2, 3, 8] {
        let (mut mesh, v, _) = cube();
        // The edge from (1, 0, 1) to (1, 1, 1), along Y at x = 1, z = 1.
        let picked = edge(&mesh, v[5], v[6]);
        select_edges(&mut mesh, &[picked]);
        let w = 0.4;
        let result = bevel(&mut mesh, w, segments);
        assert!(is_closed_and_consistent(&mesh));
        assert!(is_convex(&mesh));
        assert_eq!(mesh.face_count(), 6 + segments);
        // Every new point lies on the round: a circle of radius w around the line at
        // x = 1 - w, z = 1 - w.
        for (id, _, _) in &result.moving {
            let p = position(&mesh, *id);
            let r = ((p[0] - (1.0 - w)).powi(2) + (p[2] - (1.0 - w)).powi(2)).sqrt();
            assert!((r - w).abs() < 1e-4, "{p:?} is {r} from the axis");
        }
        // Less is removed than by the flat cut, and the more segments, the closer to
        // the true round.
        let removed = 1.0 - volume(&mesh);
        let flat = f64::from(w * w) / 2.0;
        let round = f64::from(w * w) * (1.0 - std::f64::consts::FRAC_PI_4);
        assert!(removed < flat && removed > round, "{removed}");
        if segments == 8 {
            assert!((removed - round) / round < 0.06, "{removed} vs {round}");
        }
    }
}

#[test]
fn a_loop_round_the_top_of_a_cube_meets_in_mitres() {
    let (mut mesh, v, _) = cube();
    let top = [
        edge(&mesh, v[4], v[5]),
        edge(&mesh, v[5], v[6]),
        edge(&mesh, v[6], v[7]),
        edge(&mesh, v[7], v[4]),
    ];
    select_edges(&mut mesh, &top);
    let w = 0.25f32;
    bevel(&mut mesh, w, 1);
    assert_eq!(counts(&mesh), (12, 20, 10));
    assert!(is_closed_and_consistent(&mesh));
    assert!(is_convex(&mesh));
    // A mitred frame of triangles: area w²/2, swept along a square of side 1 - 2w/3.
    let w = f64::from(w);
    let expected = 1.0 - w * w / 2.0 * 4.0 * (1.0 - 2.0 * w / 3.0);
    assert!((volume(&mesh) - expected).abs() < 1e-5, "{}", volume(&mesh));

    // Rounded, it stays closed and removes less.
    let (mut round, v, _) = cube();
    let top = [
        edge(&round, v[4], v[5]),
        edge(&round, v[5], v[6]),
        edge(&round, v[6], v[7]),
        edge(&round, v[7], v[4]),
    ];
    select_edges(&mut round, &top);
    bevel(&mut round, 0.25, 4);
    assert!(is_closed_and_consistent(&round));
    assert!(is_convex(&round));
    assert!(volume(&round) > expected);
}

#[test]
fn all_cube_edges_flat_make_a_chamfered_cube() {
    let (mut mesh, _, _) = cube();
    mesh.select_all();
    let w = 0.2;
    bevel(&mut mesh, w, 1);
    // 8 corner triangles, 12 edge strips and 6 smaller faces.
    assert_eq!(counts(&mesh), (24, 48, 26));
    assert!(is_closed_and_consistent(&mesh));
    assert!(is_convex(&mesh));
    // Each point keeps one coordinate of its corner and moves w in along the others.
    for (id, _) in mesh.vertices() {
        let p = position(&mesh, id);
        let kept = p
            .iter()
            .filter(|&&c| c.abs() < 1e-6 || (c - 1.0).abs() < 1e-6)
            .count();
        let moved = p
            .iter()
            .filter(|&&c| (c - w).abs() < 1e-6 || (c - (1.0 - w)).abs() < 1e-6)
            .count();
        assert_eq!((kept, moved), (1, 2), "{p:?}");
    }
}

#[test]
fn all_cube_edges_round_make_a_closed_rounded_cube() {
    for segments in [2, 3] {
        let (mut mesh, _, _) = cube();
        mesh.select_all();
        // In face mode, so that only the new faces count as selected.
        let result = mesh
            .bevel_selected_edges(0.25, segments, SelectMode::Face)
            .unwrap();
        mesh.validate().unwrap();
        assert!(is_closed_and_consistent(&mesh), "segments {segments}");
        assert!(is_convex(&mesh), "a rounded cube must be convex");
        let v = volume(&mesh);
        assert!(v < 1.0 && v > 1.0 - 12.0 * 0.25 * 0.25 / 2.0, "{v}");
        assert!(!result.moving.is_empty());
        // Near each corner, every point lies on a sphere of radius w around the
        // corner moved w inward along each axis.
        let w = 0.25f32;
        for (id, _, _) in &result.moving {
            let p = position(&mesh, *id);
            let near: [f32; 3] = p.map(|c| if c < 0.5 { w } else { 1.0 - w });
            let inside = (0..3).all(|i| (p[i] - 0.5).abs() >= (near[i] - 0.5).abs() - 1e-5);
            if inside {
                let r = (0..3).map(|i| (p[i] - near[i]).powi(2)).sum::<f32>().sqrt();
                assert!(
                    (r - w).abs() < 0.02 * w,
                    "{p:?} is {r} from its corner's center"
                );
            }
        }
        // Every face of the result is selected except the six shrunk cube faces.
        let selected = mesh.faces().filter(|(_, f)| f.is_selected()).count();
        assert_eq!(selected, mesh.face_count() - 6);
    }
}

#[test]
fn a_cylinder_rim_bevels_into_a_closed_shape() {
    let mut mesh = Mesh::new();
    let (v, _) = mesh
        .add_primitive_parts(
            Primitive::Cylinder {
                radius: 1.0,
                height: 2.0,
                segments: 8,
            },
            [0.0; 3],
        )
        .unwrap();
    let before = volume(&mesh);
    let rim = mesh.edge_loop(edge(&mesh, v[8], v[9])).unwrap();
    assert_eq!(rim.len(), 8);
    select_edges(&mut mesh, &rim);
    for segments in [1, 3] {
        let mut copy = mesh.clone();
        bevel(&mut copy, 0.2, segments);
        assert!(is_closed_and_consistent(&copy));
        assert!(volume(&copy) < before);
        assert_eq!(copy.face_count(), 10 + 8 * segments);
    }
}

#[test]
fn a_loop_through_four_edge_vertices_shares_its_profiles() {
    // The equator of a sphere: each vertex has four edges, two of them beveled.
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
    let equator = mesh
        .edges()
        .map(|(id, _)| id)
        .find(|&e| {
            mesh.edge(e)
                .unwrap()
                .vertices()
                .iter()
                .all(|&v| position(&mesh, v)[2].abs() < 1e-5)
        })
        .unwrap();
    let ring = mesh.edge_loop(equator).unwrap();
    select_edges(&mut mesh, &ring);
    let before = counts(&mesh);
    bevel(&mut mesh, 0.1, 3);
    assert!(is_closed_and_consistent(&mesh));
    // Each of the 12 vertices becomes the 4 points of its profile; 12 x 3 new faces.
    assert_eq!(mesh.vertex_count(), before.0 - 12 + 12 * 4);
    assert_eq!(mesh.face_count(), before.2 + 36);
}

#[test]
fn a_single_edge_where_more_edges_meet_keeps_the_far_ones_attached() {
    let mut mesh = Mesh::new();
    mesh.add_primitive(
        Primitive::Icosphere {
            radius: 1.0,
            subdivisions: 1,
        },
        [0.0; 3],
    )
    .unwrap();
    let first = mesh.edges().next().unwrap().0;
    let before = volume(&mesh);
    for segments in [1, 2] {
        let mut copy = mesh.clone();
        select_edges(&mut copy, &[first]);
        bevel(&mut copy, 0.05, segments);
        assert!(is_closed_and_consistent(&copy));
        assert!(volume(&copy) < before);
    }
}

#[test]
fn bevelling_inside_an_open_grid_keeps_its_border_open() {
    let (mut mesh, v) = grid(3, 3);
    let at = |x: usize, y: usize| v[y * 4 + x];
    // An upright edge from the bottom border to the middle, and a whole middle line.
    let line = mesh.edge_loop(edge(&mesh, at(1, 0), at(1, 1))).unwrap();
    assert_eq!(line.len(), 3);
    select_edges(&mut mesh, &line);
    let open = mesh.open_edge_count();
    assert_eq!(hole_count(&mesh), 1);
    for segments in [1, 2] {
        let mut copy = mesh.clone();
        bevel(&mut copy, 0.2, segments);
        assert!(winding_agrees(&copy));
        assert_eq!(copy.face_count(), 9 + 3 * segments);
        // The cut's ends join the border, which stays one outline.
        assert_eq!(copy.open_edge_count(), open + 2 * segments);
        assert_eq!(hole_count(&copy), 1);
    }
}

#[test]
fn width_changes_by_moving_points_and_stops_at_the_limit() {
    let (mut mesh, _, _) = cube();
    mesh.select_all();
    let mut a = mesh.clone();
    let made = bevel(&mut a, 0.1, 2);
    let mut b = mesh.clone();
    bevel(&mut b, 0.3, 2);
    // Moving a's points to width 0.3 gives exactly b.
    for (id, p) in made.positions(0.3).collect::<Vec<_>>() {
        a.set_vertex_position(id, p).unwrap();
    }
    let sorted = |m: &Mesh| {
        let mut points: Vec<[i32; 3]> = m
            .vertices()
            .map(|(_, v)| v.position().map(|c| (c * 1e4).round() as i32))
            .collect();
        points.sort();
        points
    };
    assert_eq!(sorted(&a), sorted(&b));

    // Too wide is held back: on a unit cube, points may reach half way along edges.
    let mut wide = mesh.clone();
    let limited = bevel(&mut wide, 5.0, 1);
    assert!(
        limited.width <= 0.5 && limited.width > 0.45,
        "{}",
        limited.width
    );
    assert_eq!(limited.width, limited.max_width);
    assert!(is_closed_and_consistent(&wide));
    let far: Vec<_> = limited.positions(100.0).collect();
    let near: Vec<_> = limited.positions(limited.max_width).collect();
    assert_eq!(far, near);
}

#[test]
fn bevel_refusals_change_nothing() {
    // A border edge has a face on one side only.
    let (mesh, v) = grid(2, 2);
    let mut doc = Document::new(mesh);
    let border = edge(doc.mesh(), v[0], v[1]);
    doc.edit("select", |m| {
        m.select_elements(
            SelectMode::Edge,
            &[Element::Edge(border)],
            SelectOp::Replace,
        )
    })
    .unwrap();
    let before = doc.mesh().clone();
    let result = doc.edit("bevel", |m| {
        m.bevel_selected_edges(0.1, 1, SelectMode::Edge).map(|_| ())
    });
    assert_eq!(result, Err(MeshError::CannotBevel));
    assert_eq!(*doc.mesh(), before);

    let (mut mesh, v, _) = cube();
    let before = mesh.clone();
    assert_eq!(
        mesh.bevel_selected_edges(0.1, 1, SelectMode::Edge),
        Err(MeshError::NothingSelected)
    );
    let picked = edge(&mesh, v[0], v[1]);
    select_edges(&mut mesh, &[picked]);
    let selected = mesh.clone();
    for bad in [-0.1, f32::NAN, f32::INFINITY] {
        assert_eq!(
            mesh.bevel_selected_edges(bad, 1, SelectMode::Edge),
            Err(MeshError::InvalidDistance)
        );
    }
    for bad in [0, MAX_BEVEL_SEGMENTS + 1] {
        assert_eq!(
            mesh.bevel_selected_edges(0.1, bad, SelectMode::Edge),
            Err(MeshError::InvalidSegments)
        );
    }
    assert_eq!(mesh, selected);
    assert_ne!(mesh, before);

    // Loose edges cannot be beveled.
    let mut wire = Mesh::new();
    let a = wire.add_vertex([0.0; 3]).unwrap();
    let b = wire.add_vertex([1.0, 0.0, 0.0]).unwrap();
    let e = wire.add_edge(a, b).unwrap();
    select_edges(&mut wire, &[e]);
    assert_eq!(
        wire.bevel_selected_edges(0.1, 1, SelectMode::Edge),
        Err(MeshError::CannotBevel)
    );
}

#[test]
fn a_zero_width_bevel_is_valid_and_can_grow() {
    let (mut mesh, _, _) = cube();
    mesh.select_all();
    let made = bevel(&mut mesh, 0.0, 3);
    assert_eq!(made.width, 0.0);
    for (id, p) in made.positions(0.2).collect::<Vec<_>>() {
        mesh.set_vertex_position(id, p).unwrap();
    }
    assert!(is_closed_and_consistent(&mesh));
    assert!(volume(&mesh) < 1.0);
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

/// Random bevels on every kind of shape, with random selections, widths and segments.
/// A bevel must never open or close a hole, must keep winding consistent, and must
/// undo exactly. A refusal must change nothing.
#[test]
fn random_bevels_stay_valid_keep_holes_and_undo_exactly() {
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
            rows: 3,
        },
        Primitive::Cone {
            radius: 1.0,
            height: 2.0,
            segments: 6,
        },
    ];
    let mut worked = 0;
    let mut refused = 0;
    for seed in 0..60u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut doc = Document::new(Mesh::new());
        doc.edit("start", |m| {
            for (i, &shape) in shapes.iter().enumerate() {
                m.add_primitive(shape, [i as f32 * 4.0, 0.0, 0.0])?;
            }
            Ok(())
        })
        .unwrap();
        let mut states = vec![doc.mesh().clone()];
        for _ in 0..6 {
            let edges: Vec<EdgeId> = doc.mesh().edges().map(|(id, _)| id).collect();
            let picks: Vec<EdgeId> = (0..1 + rng.below(4))
                .map(|_| edges[rng.below(edges.len())])
                .collect();
            let how = rng.below(4);
            let width = [0.0, 0.05, 0.2, 0.6, 3.0][rng.below(5)];
            let segments = 1 + rng.below(4);
            let mode = [SelectMode::Vertex, SelectMode::Edge, SelectMode::Face][rng.below(3)];
            let before = doc.mesh().clone();
            let holes = hole_count(&before);
            let undo_before = doc.undo_count();
            let result = doc.edit("bevel", |m| {
                let mut chosen = picks.clone();
                match how {
                    0 => {}
                    1 => chosen = m.edge_loop(picks[0])?,
                    2 => chosen = m.edge_ring(picks[0])?,
                    _ => {
                        let part = m.linked_faces(&[Element::Edge(picks[0])]);
                        let elements: Vec<Element> = part.into_iter().map(Element::Face).collect();
                        m.select_elements(SelectMode::Face, &elements, SelectOp::Replace)?;
                        chosen.clear();
                    }
                }
                if !chosen.is_empty() {
                    let elements: Vec<Element> = chosen.iter().map(|&e| Element::Edge(e)).collect();
                    m.select_elements(SelectMode::Edge, &elements, SelectOp::Replace)?;
                }
                let made = m.bevel_selected_edges(width, segments, mode)?;
                // Widening afterwards must keep everything valid too.
                for (id, p) in made.positions(made.max_width).collect::<Vec<_>>() {
                    m.set_vertex_position(id, p)?;
                }
                Ok(())
            });
            match result {
                Ok(()) => {
                    worked += 1;
                    let mesh = doc.mesh();
                    assert_eq!(hole_count(mesh), holes, "seed {seed}: holes changed");
                    assert!(winding_agrees(mesh), "seed {seed}: winding broke");
                    if doc.undo_count() > undo_before {
                        states.push(before);
                    }
                }
                Err(_) => {
                    refused += 1;
                    assert_eq!(*doc.mesh(), before, "a refused bevel changed the mesh");
                }
            }
            doc.mesh().validate().unwrap();
            // Bevelling whole shapes again and again grows them fast.
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
    println!("worked {worked}, refused {refused}");
    assert!(worked >= 150, "only {worked} bevels worked");
}

fn settings(width: f32, segments: usize, shape: f32) -> mixer_mesh::BevelSettings {
    mixer_mesh::BevelSettings {
        width,
        segments,
        shape,
        ..Default::default()
    }
}

#[test]
fn shape_runs_from_inward_through_straight_and_round_to_sharp() {
    let removed = |shape: f32| {
        let (mut mesh, v, _) = cube();
        let picked = edge(&mesh, v[5], v[6]);
        select_edges(&mut mesh, &[picked]);
        mesh.bevel_selected(&settings(0.4, 6, shape), SelectMode::Edge)
            .unwrap();
        mesh.validate().unwrap();
        assert!(is_closed_and_consistent(&mesh));
        1.0 - volume(&mesh)
    };
    let flat = 0.4f64 * 0.4 / 2.0;
    // Straight: the same as one segment.
    assert!((removed(0.25) - flat).abs() < 1e-5, "{}", removed(0.25));
    // Round removes less, sharp almost nothing, inward more than straight.
    let (inward, round, sharp) = (removed(0.1), removed(0.5), removed(1.0));
    assert!(round < flat && round > 0.2 * flat);
    assert!(sharp < 0.1 * flat, "{sharp}");
    assert!(inward > flat, "{inward}");
    let (mut mesh, _, _) = cube();
    mesh.select_all();
    for bad in [-0.1, 1.1, f32::NAN] {
        assert_eq!(
            mesh.bevel_selected(&settings(0.1, 2, bad), SelectMode::Edge),
            Err(MeshError::InvalidShape)
        );
    }
}

#[test]
fn every_shape_keeps_a_fully_bevelled_cube_closed() {
    for shape in [0.0, 0.15, 0.25, 0.4, 0.5, 0.7, 1.0] {
        let (mut mesh, _, _) = cube();
        mesh.select_all();
        mesh.bevel_selected(&settings(0.2, 3, shape), SelectMode::Edge)
            .unwrap();
        mesh.validate().unwrap();
        assert!(is_closed_and_consistent(&mesh), "shape {shape}");
        assert!(volume(&mesh) > 0.5 && volume(&mesh) < 1.0, "shape {shape}");
    }
}

#[test]
fn without_clamping_the_width_is_not_limited() {
    let (mut mesh, _, _) = cube();
    mesh.select_all();
    let made = mesh
        .bevel_selected(
            &mixer_mesh::BevelSettings {
                width: 0.8,
                clamp: false,
                ..settings(0.0, 1, 0.5)
            },
            SelectMode::Edge,
        )
        .unwrap();
    mesh.validate().unwrap();
    assert_eq!(made.width, 0.8);
    assert!(made.max_width > 1000.0);
}

fn vertex_settings(width: f32, segments: usize) -> mixer_mesh::BevelSettings {
    mixer_mesh::BevelSettings {
        vertices: true,
        ..settings(width, segments, 0.5)
    }
}

#[test]
fn bevelling_a_cube_corner_cuts_off_an_exact_tetrahedron() {
    let (mut mesh, v, _) = cube();
    mesh.select_elements(
        SelectMode::Vertex,
        &[Element::Vertex(v[6])],
        SelectOp::Replace,
    )
    .unwrap();
    let w = 0.3;
    let made = mesh
        .bevel_selected(&vertex_settings(w, 1), SelectMode::Vertex)
        .unwrap();
    mesh.validate().unwrap();
    assert_eq!(made.width, w);
    assert_eq!(counts(&mesh), (10, 15, 7));
    assert!(is_closed_and_consistent(&mesh));
    assert!(is_convex(&mesh));
    let expected = 1.0 - f64::from(w).powi(3) / 6.0;
    assert!((volume(&mesh) - expected).abs() < 1e-6, "{}", volume(&mesh));
    // The new triangle is selected.
    assert_eq!(mesh.faces().filter(|(_, f)| f.is_selected()).count(), 1);
}

#[test]
fn vertex_bevel_rounds_with_segments_and_stops_half_way_between_neighbours() {
    let (mut mesh, v, _) = cube();
    mesh.select_elements(
        SelectMode::Vertex,
        &[Element::Vertex(v[6]), Element::Vertex(v[7])],
        SelectOp::Replace,
    )
    .unwrap();
    let made = mesh
        .bevel_selected(&vertex_settings(0.9, 4), SelectMode::Vertex)
        .unwrap();
    mesh.validate().unwrap();
    assert!(made.width < 0.5 && made.width > 0.49, "{}", made.width);
    assert!(is_closed_and_consistent(&mesh));
    assert!(volume(&mesh) < 1.0 && volume(&mesh) > 0.9);
}

#[test]
fn vertex_bevel_at_a_border_cuts_the_corner_off() {
    let (mut mesh, v) = grid(2, 2);
    mesh.select_elements(
        SelectMode::Vertex,
        &[Element::Vertex(v[0]), Element::Vertex(v[4])],
        SelectOp::Replace,
    )
    .unwrap();
    let faces = mesh.face_count();
    let holes = hole_count(&mesh);
    for segments in [1, 3] {
        let mut copy = mesh.clone();
        copy.bevel_selected(&vertex_settings(0.3, segments), SelectMode::Vertex)
            .unwrap();
        copy.validate().unwrap();
        assert!(winding_agrees(&copy));
        assert_eq!(hole_count(&copy), holes);
        // The middle vertex gets a corner face; the border corner does not.
        assert_eq!(
            copy.face_count(),
            faces + if segments == 1 { 1 } else { 4 * 3 }
        );
    }
}

#[test]
fn random_vertex_bevels_stay_valid_and_undo_exactly() {
    let mut worked = 0;
    for seed in 0..40u64 {
        let mut doc = Document::new(Mesh::new());
        doc.edit("start", |m| {
            m.add_primitive(Primitive::Cube { size: 2.0 }, [0.0; 3])?;
            m.add_primitive(
                Primitive::UvSphere {
                    radius: 1.0,
                    segments: 7,
                    rings: 4,
                },
                [4.0, 0.0, 0.0],
            )?;
            m.add_primitive(
                Primitive::Grid {
                    size: 2.0,
                    columns: 2,
                    rows: 3,
                },
                [0.0, 4.0, 0.0],
            )?;
            m.add_primitive(
                Primitive::Cone {
                    radius: 1.0,
                    height: 2.0,
                    segments: 5,
                },
                [0.0, 0.0, 4.0],
            )?;
            Ok(())
        })
        .unwrap();
        let start = doc.mesh().clone();
        let ids: Vec<VertexId> = start.vertices().map(|(id, _)| id).collect();
        let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let mut next = |limit: usize| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % limit as u64) as usize
        };
        let picks: Vec<Element> = (0..1 + next(5))
            .map(|_| Element::Vertex(ids[next(ids.len())]))
            .collect();
        let s = mixer_mesh::BevelSettings {
            vertices: true,
            width: [0.0, 0.1, 0.4, 3.0][next(4)],
            segments: 1 + next(4),
            shape: [0.2, 0.5, 0.9][next(3)],
            clamp: true,
        };
        let holes = hole_count(&start);
        let result = doc.edit("bevel", |m| {
            m.select_elements(SelectMode::Vertex, &picks, SelectOp::Replace)?;
            m.bevel_selected(&s, SelectMode::Vertex).map(|_| ())
        });
        doc.mesh().validate().unwrap();
        if result.is_ok() {
            worked += 1;
            assert_eq!(hole_count(doc.mesh()), holes, "seed {seed}");
            assert!(winding_agrees(doc.mesh()), "seed {seed}");
            doc.undo();
        }
        assert_eq!(*doc.mesh(), start, "seed {seed}");
    }
    assert!(worked >= 30, "only {worked} worked");
}
