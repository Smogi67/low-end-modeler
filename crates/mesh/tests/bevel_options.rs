//! The bevel panel's options: width types, loop slide, mark sharp, custom profiles,
//! miters and intersections.

mod common;

use common::{cube, edge, is_closed_and_consistent};
use mixer_mesh::{
    BevelSettings, CustomProfile, Document, EdgeId, Element, Intersection, Mesh, MeshError, Miter,
    ProfilePreset, ProfileType, SelectMode, SelectOp, VertexId, WidthType,
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

fn select_edges(mesh: &mut Mesh, edges: &[EdgeId]) {
    let elements: Vec<Element> = edges.iter().map(|&e| Element::Edge(e)).collect();
    mesh.select_elements(SelectMode::Edge, &elements, SelectOp::Replace)
        .unwrap();
}

/// A cube with one top edge selected: from (1, 0, 1) to (1, 1, 1).
fn one_edge() -> (Mesh, [VertexId; 8]) {
    let (mut mesh, v, _) = cube();
    let picked = edge(&mesh, v[5], v[6]);
    select_edges(&mut mesh, &[picked]);
    (mesh, v)
}

/// A cube with the loop round its top selected.
fn top_loop() -> Mesh {
    let (mut mesh, v, _) = cube();
    let top = [
        edge(&mesh, v[4], v[5]),
        edge(&mesh, v[5], v[6]),
        edge(&mesh, v[6], v[7]),
        edge(&mesh, v[7], v[4]),
    ];
    select_edges(&mut mesh, &top);
    mesh
}

fn run(mesh: &mut Mesh, settings: &BevelSettings) -> mixer_mesh::Bevel {
    let made = mesh.bevel_selected(settings, SelectMode::Edge).unwrap();
    mesh.validate().unwrap();
    assert!(is_closed_and_consistent(mesh), "{settings:?}");
    made
}

#[test]
fn every_width_type_measures_what_it_says_on_a_square_edge() {
    let half_root = 0.5f32.sqrt();
    // The type, the value, and the offset across each face it should give.
    for (width_type, value, offset) in [
        (WidthType::Offset, 0.3, 0.3),
        // Straight across the band: the hypotenuse of the offsets.
        (WidthType::Width, 0.3, 0.3 * half_root),
        // From the edge in to the flat face.
        (WidthType::Depth, 0.2, 0.2 / half_root),
        // 30% of the unit edges next to it.
        (WidthType::Percent, 30.0, 0.3),
        (WidthType::Absolute, 0.3, 0.3),
    ] {
        let (mut mesh, _) = one_edge();
        let made = run(
            &mut mesh,
            &BevelSettings {
                width: value,
                width_type,
                ..Default::default()
            },
        );
        assert!(
            (made.width - offset).abs() < 1e-5,
            "{width_type:?}: {}",
            made.width
        );
        assert!((made.width - value * made.per_unit).abs() < 1e-5);
        let removed = 1.0 - volume(&mesh);
        let expected = f64::from(offset * offset) / 2.0;
        assert!(
            (removed - expected).abs() < 1e-5,
            "{width_type:?}: {removed}"
        );
    }
}

#[test]
fn vertex_bevels_measure_widths_too() {
    let (mut mesh, v, _) = cube();
    mesh.select_elements(
        SelectMode::Vertex,
        &[Element::Vertex(v[6])],
        SelectOp::Replace,
    )
    .unwrap();
    let made = mesh
        .bevel_selected(
            &BevelSettings {
                vertices: true,
                width: 25.0,
                width_type: WidthType::Percent,
                ..Default::default()
            },
            SelectMode::Vertex,
        )
        .unwrap();
    mesh.validate().unwrap();
    assert!((made.width - 0.25).abs() < 1e-5);
    // A tetrahedron with three legs of 0.25.
    let removed = 1.0 - volume(&mesh);
    assert!((removed - 0.25f64.powi(3) / 6.0).abs() < 1e-6, "{removed}");
}

#[test]
fn loop_slide_keeps_new_points_on_slanted_edges() {
    for slide in [true, false] {
        let (mut mesh, v) = one_edge();
        // Lean the edge below (1, 1, 1) outward, so it meets the bevel at a slant.
        mesh.set_vertex_position(v[2], [1.4, 1.0, 0.0]).unwrap();
        let made = run(
            &mut mesh,
            &BevelSettings {
                width: 0.2,
                loop_slide: slide,
                ..Default::default()
            },
        );
        // The new point that moved along the slanted edge from (1, 1, 1).
        let top = [1.0f32, 1.0, 1.0];
        let down = [0.4f32, 0.0, -1.0];
        let on_line = made.moving.iter().any(|&(id, origin, _)| {
            let p = position(&mesh, id);
            let d = [p[0] - top[0], p[1] - top[1], p[2] - top[2]];
            let cross = [
                d[1] * down[2] - d[2] * down[1],
                d[2] * down[0] - d[0] * down[2],
                d[0] * down[1] - d[1] * down[0],
            ];
            origin == top && d[2] < -1e-3 && cross.iter().all(|c| c.abs() < 1e-5)
        });
        assert_eq!(on_line, slide, "loop slide {slide}");
    }
}

#[test]
fn mark_sharp_marks_the_bevels_border() {
    let (mut mesh, _) = one_edge();
    run(
        &mut mesh,
        &BevelSettings {
            width: 0.3,
            segments: 3,
            mark_sharp: true,
            ..Default::default()
        },
    );
    // The two long sides, and the three profile edges at each end.
    assert_eq!(mesh.edges().filter(|(_, e)| e.is_sharp()).count(), 8);
    let (mut plain, _) = one_edge();
    run(
        &mut plain,
        &BevelSettings {
            width: 0.3,
            segments: 3,
            ..Default::default()
        },
    );
    assert_eq!(plain.edges().filter(|(_, e)| e.is_sharp()).count(), 0);
}

#[test]
fn a_custom_chamfer_cuts_like_one_segment() {
    for segments in [1, 2, 5] {
        let (mut mesh, _) = one_edge();
        run(
            &mut mesh,
            &BevelSettings {
                width: 0.4,
                segments,
                profile_type: ProfileType::Custom,
                custom: CustomProfile::preset(ProfilePreset::Chamfer),
                ..Default::default()
            },
        );
        let removed = 1.0 - volume(&mesh);
        assert!((removed - 0.08).abs() < 1e-5, "{segments}: {removed}");
    }
    // The round preset is close to the true round.
    let (mut mesh, _) = one_edge();
    run(
        &mut mesh,
        &BevelSettings {
            width: 0.4,
            segments: 8,
            profile_type: ProfileType::Custom,
            custom: CustomProfile::preset(ProfilePreset::Round),
            ..Default::default()
        },
    );
    let round = 0.16 * (1.0 - std::f64::consts::FRAC_PI_4);
    let removed = 1.0 - volume(&mesh);
    assert!(
        (removed - round).abs() / round < 0.1,
        "{removed} vs {round}"
    );
}

#[test]
fn every_preset_keeps_shapes_closed() {
    for preset in ProfilePreset::ALL {
        for segments in [1, 3, 8, 12] {
            for picks in 0..3 {
                let mut mesh = match picks {
                    0 => one_edge().0,
                    1 => top_loop(),
                    _ => {
                        let (mut m, _, _) = cube();
                        m.select_all();
                        m
                    }
                };
                let settings = BevelSettings {
                    width: 0.15,
                    segments,
                    profile_type: ProfileType::Custom,
                    custom: CustomProfile::preset(preset),
                    ..Default::default()
                };
                run(&mut mesh, &settings);
                assert!(volume(&mesh) > 0.5 && volume(&mesh) < 1.0001, "{preset:?}");
            }
        }
    }
}

#[test]
fn one_sided_profiles_face_the_same_way_on_every_edge() {
    // The steps preset is not symmetric. On the top loop, each band should be cut the
    // same way, so the four sides of the cube lose the same volume: compare the
    // highest point of each side face's top edge.
    let mut mesh = top_loop();
    run(
        &mut mesh,
        &BevelSettings {
            width: 0.2,
            segments: 8,
            profile_type: ProfileType::Custom,
            custom: CustomProfile::preset(ProfilePreset::Steps),
            ..Default::default()
        },
    );
    // By symmetry, the shape is the same when turned a quarter around the middle.
    let mut points: Vec<[i32; 3]> = mesh
        .vertices()
        .map(|(_, v)| v.position().map(|c| (c * 1000.0).round() as i32))
        .collect();
    points.sort_unstable();
    let mut turned: Vec<[i32; 3]> = points.iter().map(|&[x, y, z]| [1000 - y, x, z]).collect();
    turned.sort_unstable();
    assert_eq!(points, turned);
}

#[test]
fn miters_patch_and_round_the_corners_of_a_loop() {
    let sharp = {
        let mut mesh = top_loop();
        run(
            &mut mesh,
            &BevelSettings {
                width: 0.2,
                ..Default::default()
            },
        );
        mesh
    };
    for (outer, inner, segments) in [
        (Miter::Patch, Miter::Sharp, 1),
        (Miter::Patch, Miter::Sharp, 3),
        (Miter::Arc, Miter::Sharp, 1),
        (Miter::Arc, Miter::Sharp, 4),
        (Miter::Sharp, Miter::Arc, 3),
        (Miter::Arc, Miter::Arc, 4),
        (Miter::Patch, Miter::Arc, 2),
    ] {
        let mut mesh = top_loop();
        let made = run(
            &mut mesh,
            &BevelSettings {
                width: 0.2,
                segments,
                miter_outer: outer,
                miter_inner: inner,
                spread: 0.3,
                ..Default::default()
            },
        );
        let label = format!("{outer:?} {inner:?} {segments}");
        assert!(mesh.vertex_count() > sharp.vertex_count(), "{label}");
        assert!(volume(&mesh) > 0.8 && volume(&mesh) < 1.0, "{label}");
        // Every point still moves with the width.
        for (id, origin, step) in &made.moving {
            let p = position(&mesh, *id);
            for k in 0..3 {
                assert!(
                    (p[k] - (origin[k] + step[k] * made.width)).abs() < 1e-5,
                    "{label}"
                );
            }
        }
    }
}

#[test]
fn cutoff_corners_are_flat_ends_and_a_flat_middle() {
    let (mut mesh, _, _) = cube();
    mesh.select_all();
    run(
        &mut mesh,
        &BevelSettings {
            width: 0.2,
            segments: 3,
            intersection: Intersection::Cutoff,
            ..Default::default()
        },
    );
    // Six faces, three band faces per edge, and per corner a middle and three ends.
    assert_eq!(mesh.face_count(), 6 + 12 * 3 + 8 * 4);
    // Every face is flat.
    for (face, _) in mesh.faces() {
        let points: Vec<[f32; 3]> = mesh
            .face_corners(face)
            .unwrap()
            .map(|c| position(&mesh, mesh.corner(c).unwrap().vertex()))
            .collect();
        let n = mesh.face_normal(face).unwrap().unwrap();
        let d0: f32 = (0..3).map(|k| n[k] * points[0][k]).sum();
        for p in &points {
            let d: f32 = (0..3).map(|k| n[k] * p[k]).sum();
            assert!((d - d0).abs() < 1e-4);
        }
    }
}

#[test]
fn bad_option_values_are_refused_and_change_nothing() {
    let (mut mesh, _) = one_edge();
    let before = mesh.clone();
    for spread in [-0.1, 1.5, f32::NAN] {
        assert_eq!(
            mesh.bevel_selected(
                &BevelSettings {
                    width: 0.2,
                    spread,
                    ..Default::default()
                },
                SelectMode::Edge
            ),
            Err(MeshError::InvalidShape)
        );
    }
    assert_eq!(mesh, before);
}

#[test]
fn every_combination_stays_valid_and_undoes_exactly() {
    let mut worked = 0;
    for picks in 0..3 {
        for &segments in &[1usize, 2, 5] {
            for &(outer, inner) in &[
                (Miter::Sharp, Miter::Sharp),
                (Miter::Patch, Miter::Arc),
                (Miter::Arc, Miter::Sharp),
            ] {
                for &intersection in &[Intersection::GridFill, Intersection::Cutoff] {
                    for &custom in &[false, true] {
                        for &width_type in &WidthType::ALL {
                            let mesh = match picks {
                                0 => one_edge().0,
                                1 => top_loop(),
                                _ => {
                                    let (mut m, _, _) = cube();
                                    m.select_all();
                                    m
                                }
                            };
                            let settings = BevelSettings {
                                width: if width_type == WidthType::Percent {
                                    15.0
                                } else {
                                    0.15
                                },
                                width_type,
                                segments,
                                profile_type: if custom {
                                    ProfileType::Custom
                                } else {
                                    ProfileType::Superellipse
                                },
                                custom: CustomProfile::preset(ProfilePreset::CrownMolding),
                                miter_outer: outer,
                                miter_inner: inner,
                                intersection,
                                loop_slide: segments != 2,
                                mark_sharp: segments == 5,
                                ..Default::default()
                            };
                            let start = mesh.clone();
                            let mut document = Document::new(mesh);
                            document
                                .edit("Bevel", |m| {
                                    m.bevel_selected(&settings, SelectMode::Edge).map(|_| ())
                                })
                                .unwrap();
                            let after = document.mesh();
                            after.validate().unwrap();
                            assert!(is_closed_and_consistent(after), "{settings:?}");
                            document.undo();
                            assert_eq!(document.mesh(), &start);
                            worked += 1;
                        }
                    }
                }
            }
        }
    }
    assert_eq!(worked, 3 * 3 * 3 * 2 * 2 * 5);
}
