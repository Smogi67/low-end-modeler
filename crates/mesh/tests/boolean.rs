//! Union, subtract and intersect.

mod common;
use common::{grid, is_closed_and_consistent};
use mixer_mesh::{
    BooleanOp, Document, Element, FaceId, Mesh, MeshError, Primitive, SelectMode, SelectOp,
    VertexId,
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

fn cube(mesh: &mut Mesh, size: f32, center: [f32; 3]) -> Vec<FaceId> {
    mesh.add_primitive(Primitive::Cube { size }, center)
        .unwrap()
}

fn select_everything(mesh: &mut Mesh) {
    let faces: Vec<Element> = mesh.faces().map(|(f, _)| Element::Face(f)).collect();
    mesh.select_elements(SelectMode::Face, &faces, SelectOp::Replace)
        .unwrap();
}

/// Runs a boolean on everything and checks the result is a valid closed surface.
fn combine(mesh: &mut Mesh, op: BooleanOp, cutter: Option<FaceId>) -> usize {
    select_everything(mesh);
    let faces = mesh.boolean_selected(op, cutter, SelectMode::Face).unwrap();
    mesh.validate().unwrap();
    assert!(is_closed_and_consistent(mesh), "the result has holes");
    assert_eq!(faces, mesh.face_count());
    faces
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-3
}

#[test]
fn two_overlapping_cubes() {
    for (op, expected) in [
        (BooleanOp::Union, 15.0),
        (BooleanOp::Subtract, 7.0),
        (BooleanOp::Intersect, 1.0),
    ] {
        let mut mesh = Mesh::new();
        cube(&mut mesh, 2.0, [0.0; 3]);
        let cutter = cube(&mut mesh, 2.0, [1.0, 1.0, 1.0])[0];
        let faces = combine(&mut mesh, op, Some(cutter));
        assert!(close(volume(&mesh), expected), "{op:?}: {}", volume(&mesh));
        assert_eq!(mesh.parts().len(), 1, "{op:?}");
        // Faces in one plane are joined again, so the result stays simple.
        assert!(faces <= 24, "{op:?} made {faces} faces");
    }
}

#[test]
fn cubes_sharing_a_face_become_one_box() {
    let mut mesh = Mesh::new();
    cube(&mut mesh, 2.0, [0.0; 3]);
    cube(&mut mesh, 2.0, [2.0, 0.0, 0.0]);
    combine(&mut mesh, BooleanOp::Union, None);
    assert!(close(volume(&mesh), 16.0));
    assert!(mesh.face_count() <= 10, "{} faces", mesh.face_count());
}

#[test]
fn a_cube_and_an_icosphere_melt_together() {
    let mut mesh = Mesh::new();
    cube(&mut mesh, 2.0, [0.0; 3]);
    mesh.add_primitive(
        Primitive::Icosphere {
            radius: 1.2,
            subdivisions: 2,
        },
        [1.0, 0.5, 0.8],
    )
    .unwrap();
    let separate = volume(&mesh);
    combine(&mut mesh, BooleanOp::Union, None);
    let joined = volume(&mesh);
    assert!(joined > 8.0 && joined < separate, "{joined} of {separate}");
    assert_eq!(mesh.parts().len(), 1);
}

#[test]
fn a_hole_through_a_cube() {
    let mut mesh = Mesh::new();
    cube(&mut mesh, 2.0, [0.0; 3]);
    let segments = 12;
    let cutter = mesh
        .add_primitive(
            Primitive::Cylinder {
                radius: 0.5,
                height: 4.0,
                segments,
            },
            [0.0; 3],
        )
        .unwrap()[0];
    combine(&mut mesh, BooleanOp::Subtract, Some(cutter));
    let n = segments as f64;
    let hole = 0.5 * n * 0.25 * (std::f64::consts::TAU / n).sin() * 2.0;
    assert!(close(volume(&mesh), 8.0 - hole), "{}", volume(&mesh));
    // A ring shape: one hole through it, so V - E + F = 0.
    let euler = mesh.vertex_count() as i64 - mesh.edge_count() as i64 + mesh.face_count() as i64;
    assert_eq!(euler, 0);
}

#[test]
fn subtract_cuts_one_part_out_of_each_other_part() {
    let mut mesh = Mesh::new();
    cube(&mut mesh, 2.0, [0.0; 3]);
    cube(&mut mesh, 2.0, [3.0, 0.0, 0.0]);
    let cutter = mesh
        .add_primitive(
            Primitive::Cylinder {
                radius: 0.6,
                height: 1.0,
                segments: 8,
            },
            [1.5, 0.0, 0.0],
        )
        .unwrap()[0];
    combine(&mut mesh, BooleanOp::Subtract, Some(cutter));
    assert_eq!(mesh.parts().len(), 2, "the two cubes stay separate");
}

#[test]
fn separate_shapes_union_into_one_part() {
    let mut mesh = Mesh::new();
    cube(&mut mesh, 2.0, [0.0; 3]);
    cube(&mut mesh, 2.0, [5.0, 0.0, 0.0]);
    combine(&mut mesh, BooleanOp::Union, None);
    assert!(close(volume(&mesh), 16.0));
    assert_eq!(mesh.parts().len(), 1, "joined into one group");
}

#[test]
fn refusals_change_nothing() {
    // Shapes that do not overlap have no intersection.
    let mut mesh = Mesh::new();
    cube(&mut mesh, 2.0, [0.0; 3]);
    cube(&mut mesh, 2.0, [5.0, 0.0, 0.0]);
    select_everything(&mut mesh);
    let before = mesh.clone();
    assert_eq!(
        mesh.boolean_selected(BooleanOp::Intersect, None, SelectMode::Face),
        Err(MeshError::EmptyResult)
    );
    assert_eq!(
        mesh.boolean_selected(BooleanOp::Subtract, None, SelectMode::Face),
        Err(MeshError::NoCutter)
    );
    assert_eq!(mesh, before);

    // One part is not enough.
    let mut single = Mesh::new();
    cube(&mut single, 2.0, [0.0; 3]);
    select_everything(&mut single);
    assert_eq!(
        single.boolean_selected(BooleanOp::Union, None, SelectMode::Face),
        Err(MeshError::NeedsTwoParts)
    );

    // An open grid is not a solid.
    let (mut open, _) = grid(2, 2);
    cube(&mut open, 1.0, [0.5, 0.5, 0.0]);
    select_everything(&mut open);
    let before = open.clone();
    assert_eq!(
        open.boolean_selected(BooleanOp::Union, None, SelectMode::Face),
        Err(MeshError::NotClosed)
    );
    assert_eq!(open, before);
}

#[test]
fn undo_puts_the_shapes_back_exactly() {
    let mut mesh = Mesh::new();
    cube(&mut mesh, 2.0, [0.0; 3]);
    cube(&mut mesh, 1.5, [1.0, 0.3, 0.2]);
    select_everything(&mut mesh);
    let mut document = Document::new(mesh.clone());
    document
        .edit("Union", |m| {
            m.boolean_selected(BooleanOp::Union, None, SelectMode::Face)
                .map(|_| ())
        })
        .unwrap();
    assert_ne!(document.mesh(), &mesh);
    document.undo();
    assert_eq!(document.mesh(), &mesh);
    document.redo();
    assert!(is_closed_and_consistent(document.mesh()));
}

#[test]
fn inside_out_shapes_are_turned_the_right_way() {
    let mut mesh = Mesh::new();
    let faces = cube(&mut mesh, 2.0, [0.0; 3]);
    for face in faces {
        mesh.flip_face(face).unwrap();
    }
    cube(&mut mesh, 2.0, [1.0, 1.0, 1.0]);
    combine(&mut mesh, BooleanOp::Union, None);
    assert!(close(volume(&mesh), 15.0), "{}", volume(&mesh));
}

/// Random shapes of every kind, often lined up exactly, combined every way. 600 such
/// cases ran without a crack while this was written; this runs 150 of them.
#[test]
fn random_shapes_combine_into_closed_surfaces() {
    let mut seed = 999u64;
    let mut next = || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((seed >> 33) as f32) / (u32::MAX >> 1) as f32
    };
    for round in 0..150 {
        let mut mesh = Mesh::new();
        for _ in 0..2 + round % 2 {
            let c = [next() * 2.0 - 1.0, next() * 2.0 - 1.0, next() * 2.0 - 1.0];
            // Some centers on a grid, so that faces often line up exactly.
            let c = if next() < 0.4 {
                c.map(|v| (v * 2.0).round() / 2.0)
            } else {
                c
            };
            let shape = match (next() * 5.0) as u32 {
                0 => Primitive::Cube {
                    size: 0.5 + next() * 2.0,
                },
                1 => Primitive::UvSphere {
                    radius: 0.4 + next(),
                    segments: 5 + (next() * 12.0) as u32,
                    rings: 3 + (next() * 8.0) as u32,
                },
                2 => Primitive::Icosphere {
                    radius: 0.4 + next(),
                    subdivisions: (next() * 3.0) as u32,
                },
                3 => Primitive::Cylinder {
                    radius: 0.3 + next(),
                    height: 0.5 + next() * 2.0,
                    segments: 3 + (next() * 14.0) as u32,
                },
                _ => Primitive::Cone {
                    radius: 0.3 + next(),
                    height: 0.5 + next() * 2.0,
                    segments: 3 + (next() * 14.0) as u32,
                },
            };
            mesh.add_primitive(shape, c).unwrap();
        }
        let op = [BooleanOp::Union, BooleanOp::Intersect, BooleanOp::Subtract]
            [(next() * 3.0) as usize % 3];
        let cutter = mesh.faces().last().map(|(f, _)| f);
        select_everything(&mut mesh);
        match mesh.boolean_selected(op, cutter, SelectMode::Face) {
            Ok(_) => {
                mesh.validate().unwrap();
                assert!(
                    is_closed_and_consistent(&mesh),
                    "round {round} {op:?} left holes"
                );
                assert!(volume(&mesh) > 0.0, "round {round} {op:?} is inside out");
            }
            Err(MeshError::EmptyResult) => {}
            Err(e) => panic!("round {round} {op:?}: {e}"),
        }
    }
}
