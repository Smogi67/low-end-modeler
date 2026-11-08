//! Smooth and flat shading.

mod common;
use common::cube;
use mixer_mesh::{
    AUTO_SMOOTH_ANGLE, BevelSettings, BooleanOp, Document, Element, FileKind, Mesh, MeshError,
    Primitive, SelectMode, SelectOp,
};

fn select_all_faces(mesh: &mut Mesh) {
    let faces: Vec<Element> = mesh.faces().map(|(f, _)| Element::Face(f)).collect();
    mesh.select_elements(SelectMode::Face, &faces, SelectOp::Replace)
        .unwrap();
}

fn close(a: [f32; 3], b: [f32; 3]) -> bool {
    (0..3).all(|i| (a[i] - b[i]).abs() < 1e-4)
}

#[test]
fn flat_faces_keep_their_own_normal() {
    let (mesh, _, _) = cube();
    let normals = mesh.shading_normals();
    for (face, _) in mesh.faces() {
        let n = mesh.face_normal(face).unwrap().unwrap();
        for corner in mesh.face_corners(face).unwrap() {
            let v = mesh.corner(corner).unwrap().vertex();
            assert!(close(normals.at(face, v), n));
        }
    }
}

#[test]
fn a_fully_smooth_cube_points_its_corners_outward() {
    let (mut mesh, v, _) = cube();
    select_all_faces(&mut mesh);
    assert_eq!(mesh.set_shading_selected(180), Ok(6));
    let normals = mesh.shading_normals();
    // Corner v[0] is at the origin, so it points away along (-1, -1, -1).
    let third = -(1.0f32 / 3.0).sqrt();
    for (face, _) in mesh.faces() {
        let has = mesh
            .face_corners(face)
            .unwrap()
            .any(|c| mesh.corner(c).unwrap().vertex() == v[0]);
        if has {
            assert!(close(normals.at(face, v[0]), [third; 3]));
        }
    }
}

#[test]
fn auto_smooth_keeps_sharp_corners_and_rounds_curves() {
    // The cube's edges are 90 degrees: sharper than 30, so it stays flat.
    let (mut mesh, _, _) = cube();
    select_all_faces(&mut mesh);
    mesh.set_shading_selected(AUTO_SMOOTH_ANGLE).unwrap();
    let normals = mesh.shading_normals();
    for (face, _) in mesh.faces() {
        let n = mesh.face_normal(face).unwrap().unwrap();
        for corner in mesh.face_corners(face).unwrap() {
            let v = mesh.corner(corner).unwrap().vertex();
            assert!(close(normals.at(face, v), n));
        }
    }

    // A cylinder's sides meet at 22.5 degrees, so they round, and the caps stay flat.
    let mut mesh = Mesh::new();
    mesh.add_primitive(
        Primitive::Cylinder {
            radius: 1.0,
            height: 2.0,
            segments: 16,
        },
        [0.0; 3],
    )
    .unwrap();
    select_all_faces(&mut mesh);
    mesh.set_shading_selected(AUTO_SMOOTH_ANGLE).unwrap();
    let normals = mesh.shading_normals();
    for (face, _) in mesh.faces() {
        let n = mesh.face_normal(face).unwrap().unwrap();
        for corner in mesh.face_corners(face).unwrap() {
            let v = mesh.corner(corner).unwrap().vertex();
            let p = mesh.vertex(v).unwrap().position();
            let s = normals.at(face, v);
            if n[2].abs() > 0.99 {
                assert!(close(s, n), "a cap stays flat");
            } else {
                // A side corner points straight out from the axis.
                let r = (p[0] * p[0] + p[1] * p[1]).sqrt();
                assert!(close(s, [p[0] / r, p[1] / r, 0.0]), "{s:?} at {p:?}");
            }
        }
    }
}

#[test]
fn a_smooth_bevel_stays_smooth_and_flat_faces_stay_flat() {
    let (mut mesh, _, _) = cube();
    select_all_faces(&mut mesh);
    mesh.set_shading_selected(AUTO_SMOOTH_ANGLE).unwrap();
    let edges: Vec<Element> = mesh.edges().map(|(e, _)| Element::Edge(e)).collect();
    mesh.select_elements(SelectMode::Edge, &edges, SelectOp::Replace)
        .unwrap();
    let mut document = Document::new(mesh);
    document
        .edit("Bevel", |m| {
            let settings = BevelSettings {
                width: 0.1,
                segments: 3,
                ..BevelSettings::default()
            };
            m.bevel_selected(&settings, SelectMode::Edge).map(|_| ())
        })
        .unwrap();
    let mesh = document.mesh();
    assert!(
        mesh.faces().all(|(_, f)| f.smooth() == AUTO_SMOOTH_ANGLE),
        "new faces take the shading of the faces beside them"
    );
    // The six big faces are lit almost exactly flat, because they outweigh the thin
    // bevel faces beside them.
    let normals = mesh.shading_normals();
    let mut big = 0;
    for (face, _) in mesh.faces() {
        let area: f32 = mesh
            .face_triangles(face)
            .unwrap()
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|v| mesh.vertex(v).unwrap().position());
                let (u, w) = (
                    [b[0] - a[0], b[1] - a[1], b[2] - a[2]],
                    [c[0] - a[0], c[1] - a[1], c[2] - a[2]],
                );
                let x = [
                    u[1] * w[2] - u[2] * w[1],
                    u[2] * w[0] - u[0] * w[2],
                    u[0] * w[1] - u[1] * w[0],
                ];
                (x[0] * x[0] + x[1] * x[1] + x[2] * x[2]).sqrt() / 2.0
            })
            .sum();
        // The six big faces are 0.8 by 0.8.
        if area < 0.5 {
            continue;
        }
        big += 1;
        let n = mesh.face_normal(face).unwrap().unwrap();
        for corner in mesh.face_corners(face).unwrap() {
            let v = mesh.corner(corner).unwrap().vertex();
            let s = normals.at(face, v);
            let d: f32 = (0..3).map(|i| s[i] * n[i]).sum();
            assert!(d > 0.95, "{d}");
        }
    }
    assert_eq!(big, 6);
}

#[test]
fn shading_saves_undoes_and_refuses_bad_values() {
    let (mut mesh, _, _) = cube();
    let before = mesh.clone();
    assert_eq!(
        mesh.set_shading_selected(30),
        Err(MeshError::NoFacesSelected)
    );
    select_all_faces(&mut mesh);
    assert_eq!(
        mesh.set_shading_selected(181),
        Err(MeshError::InvalidSmoothAngle)
    );
    mesh.set_shading_selected(45).unwrap();
    let text = mesh.to_project();
    assert!(text.contains("\ns 45\n"));
    let (opened, _) = Mesh::from_text(&text, FileKind::Project).unwrap();
    assert!(opened.faces().all(|(_, f)| f.smooth() == 45));

    // OBJ smoothing groups mean smooth.
    let obj = "v 0 0 0\nv 1 0 0\nv 0 1 0\ns 1\nf 1 2 3\n";
    let (imported, _) = Mesh::from_text(obj, FileKind::Obj).unwrap();
    assert!(imported.faces().all(|(_, f)| f.smooth() == 180));

    let mut document = Document::new(before.clone());
    document
        .edit("Select", |m| {
            select_all_faces(m);
            Ok(())
        })
        .unwrap();
    let selected = document.mesh().clone();
    document
        .edit("Smooth", |m| m.set_shading_selected(180).map(|_| ()))
        .unwrap();
    document.undo();
    assert_eq!(document.mesh(), &selected);
}

#[test]
fn union_keeps_each_shapes_shading() {
    let mut mesh = Mesh::new();
    mesh.add_primitive(Primitive::Cube { size: 2.0 }, [0.0; 3])
        .unwrap();
    let sphere = mesh
        .add_primitive(
            Primitive::UvSphere {
                radius: 1.0,
                segments: 12,
                rings: 6,
            },
            [1.0, 0.0, 0.0],
        )
        .unwrap();
    let sphere: Vec<Element> = sphere.into_iter().map(Element::Face).collect();
    mesh.select_elements(SelectMode::Face, &sphere, SelectOp::Replace)
        .unwrap();
    mesh.set_shading_selected(180).unwrap();
    select_all_faces(&mut mesh);
    mesh.boolean_selected(BooleanOp::Union, None, SelectMode::Face)
        .unwrap();
    let smooth = mesh.faces().filter(|(_, f)| f.smooth() == 180).count();
    let flat = mesh.faces().filter(|(_, f)| f.smooth() == 0).count();
    assert!(smooth > 0 && flat > 0, "{smooth} smooth, {flat} flat");
}

#[test]
fn exports_carry_smooth_normals() {
    let mut mesh = Mesh::new();
    mesh.add_primitive(
        Primitive::Cylinder {
            radius: 1.0,
            height: 2.0,
            segments: 8,
        },
        [0.0; 3],
    )
    .unwrap();
    let flat = mesh.to_obj("C", 1.0).unwrap();
    let count = |text: &str| text.lines().filter(|l| l.starts_with("vn ")).count();
    // Flat: eight sides and two caps.
    assert_eq!(count(&flat), 10);
    select_all_faces(&mut mesh);
    mesh.set_shading_selected(AUTO_SMOOTH_ANGLE).unwrap();
    // Smooth: one normal per side point pair direction (8), and the two caps.
    let smooth = mesh.to_obj("C", 1.0).unwrap();
    assert_eq!(count(&smooth), 10);
    assert_ne!(flat, smooth);
    // The glTF file changes too, and stays the same size: one normal per corner.
    let flat_glb = {
        let mut m = mesh.clone();
        select_all_faces(&mut m);
        m.set_shading_selected(0).unwrap();
        m.to_glb(1.0).unwrap()
    };
    let smooth_glb = mesh.to_glb(1.0).unwrap();
    assert_eq!(flat_glb.len(), smooth_glb.len());
    assert_ne!(flat_glb, smooth_glb);
}

#[test]
fn sharp_edges_split_smooth_shading_and_are_saved() {
    let (mut mesh, v, _) = cube();
    select_all_faces(&mut mesh);
    mesh.set_shading_selected(180).unwrap();
    // Mark one edge sharp: a single sharp edge at a corner changes nothing there.
    let edge = mesh
        .edges()
        .find(|(_, e)| {
            let ends = e.vertices();
            ends.contains(&v[0]) && ends.contains(&v[1])
        })
        .unwrap()
        .0;
    mesh.select_elements(SelectMode::Edge, &[Element::Edge(edge)], SelectOp::Replace)
        .unwrap();
    assert_eq!(mesh.set_sharp_selected(true), Ok(1));
    assert!(mesh.edge(edge).unwrap().is_sharp());
    let one = mesh.shading_normals();
    let third = (1.0f32 / 3.0).sqrt();
    let corner_face = mesh.vertex_faces(v[0]).unwrap().next().unwrap();
    assert!(close(one.at(corner_face, v[0]), [-third; 3]));

    // Every edge sharp: each face is lit flat again.
    let edges: Vec<Element> = mesh.edges().map(|(e, _)| Element::Edge(e)).collect();
    mesh.select_elements(SelectMode::Edge, &edges, SelectOp::Replace)
        .unwrap();
    assert_eq!(mesh.set_sharp_selected(true), Ok(11));
    let all = mesh.shading_normals();
    for (face, _) in mesh.faces() {
        let n = mesh.face_normal(face).unwrap().unwrap();
        for corner in mesh.face_corners(face).unwrap() {
            let vertex = mesh.corner(corner).unwrap().vertex();
            assert!(close(all.at(face, vertex), n));
        }
    }

    // Saved and read back.
    let text = mesh.to_project();
    assert_eq!(text.matches("\nsharp ").count(), 12);
    let (back, _) = Mesh::from_text(&text, FileKind::Project).unwrap();
    assert_eq!(back.edges().filter(|(_, e)| e.is_sharp()).count(), 12);
    // Clearing them.
    assert_eq!(mesh.set_sharp_selected(false), Ok(12));
    assert_eq!(mesh.edges().filter(|(_, e)| e.is_sharp()).count(), 0);
    let mut empty = Mesh::new();
    assert_eq!(
        empty.set_sharp_selected(true),
        Err(MeshError::NothingSelected)
    );
}
