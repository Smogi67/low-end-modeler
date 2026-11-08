//! Mirror and array modifiers.

mod common;
use common::is_closed_and_consistent;
use mixer_mesh::{
    Document, Element, FileKind, Mesh, MeshError, Modifier, Primitive, SelectMode, SelectOp,
};

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

/// A cube from 0 to 1, as one selected part.
fn unit_cube() -> Mesh {
    let mut mesh = Mesh::new();
    let faces = mesh
        .add_primitive(Primitive::Cube { size: 1.0 }, [0.5, 0.5, 0.5])
        .unwrap();
    let faces: Vec<Element> = faces.into_iter().map(Element::Face).collect();
    mesh.select_elements(SelectMode::Face, &faces, SelectOp::Replace)
        .unwrap();
    mesh
}

#[test]
fn a_mirrored_cube_merges_into_one_closed_box() {
    let mut mesh = unit_cube();
    mesh.add_modifier_to_selected(Modifier::mirror()).unwrap();
    // The edited mesh is unchanged.
    assert_eq!(mesh.face_count(), 6);
    let result = mesh.evaluated();
    result.validate().unwrap();
    assert!(is_closed_and_consistent(&result));
    // The walls on the middle cancel out: a box from -1 to 1 along X.
    assert_eq!(result.face_count(), 10);
    assert!((volume(&result) - 2.0).abs() < 1e-5);
    assert_eq!(result.modifiers().count(), 0);
}

#[test]
fn an_open_half_closes_when_mirrored() {
    // Half a box, open on the middle side (x = 0).
    let text = "v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nv 0 0 1\nv 1 0 1\nv 1 1 1\nv 0 1 1\n\
                f 1 4 3 2\nf 5 6 7 8\nf 1 2 6 5\nf 2 3 7 6\nf 3 4 8 7\n";
    let (mut mesh, _) = Mesh::from_text(text, FileKind::Obj).unwrap();
    let faces: Vec<Element> = mesh.faces().map(|(f, _)| Element::Face(f)).collect();
    mesh.select_elements(SelectMode::Face, &faces, SelectOp::Replace)
        .unwrap();
    assert!(!is_closed_and_consistent(&mesh));
    mesh.add_modifier_to_selected(Modifier::mirror()).unwrap();
    let result = mesh.evaluated();
    assert!(is_closed_and_consistent(&result));
    assert!((volume(&result) - 2.0).abs() < 1e-5);
}

#[test]
fn arrays_repeat_and_can_merge() {
    for (merge, faces) in [(None, 18), (Some(0.001), 14)] {
        let mut mesh = unit_cube();
        mesh.add_modifier_to_selected(Modifier::Array {
            count: 3,
            offset: [1.0, 0.0, 0.0],
            merge,
        })
        .unwrap();
        let result = mesh.evaluated();
        result.validate().unwrap();
        assert!(is_closed_and_consistent(&result), "{merge:?}");
        assert_eq!(result.face_count(), faces, "{merge:?}");
        assert!((volume(&result) - 3.0).abs() < 1e-5);
    }
}

#[test]
fn modifiers_run_in_order_and_only_on_their_part() {
    let mut mesh = unit_cube();
    mesh.add_primitive(Primitive::Cube { size: 1.0 }, [5.0, 5.0, 5.0])
        .unwrap();
    mesh.add_modifier_to_selected(Modifier::Array {
        count: 2,
        offset: [0.0, 2.0, 0.0],
        merge: None,
    })
    .unwrap();
    mesh.add_modifier_to_selected(Modifier::mirror()).unwrap();
    let list = mesh.modifiers_of_selected();
    assert!(matches!(list[0].1, Modifier::Array { .. }));
    assert!(matches!(list[1].1, Modifier::Mirror { .. }));
    // Two cubes in a row, then mirrored into a 2-wide box each: 2 boxes of 10
    // faces, and the other cube untouched.
    let result = mesh.evaluated();
    assert_eq!(result.face_count(), 20 + 6);
    assert!((volume(&result) - (4.0 + 1.0)).abs() < 1e-5);
}

#[test]
fn changes_undo_save_and_apply() {
    let mut document = Document::new(unit_cube());
    let start = document.mesh().clone();
    let mut id = None;
    document
        .edit("Add mirror", |m| {
            id = Some(m.add_modifier_to_selected(Modifier::mirror())?);
            Ok(())
        })
        .unwrap();
    let id = id.unwrap();
    document
        .edit("Mirror settings", |m| {
            m.set_modifier(
                id,
                Modifier::Mirror {
                    axes: [true, true, false],
                    merge: Some(0.001),
                },
            )
        })
        .unwrap();
    assert_eq!(document.mesh().evaluated().face_count(), 16);

    // Saved and opened again.
    let text = document.mesh().to_project();
    assert!(text.contains("mirror 1 1 0 0.001"), "{text}");
    let (opened, _) = Mesh::from_text(&text, FileKind::Project).unwrap();
    assert_eq!(opened.evaluated().face_count(), 16);

    // Applying makes it real.
    document
        .edit("Apply", |m| {
            m.apply_modifiers_of_selected(SelectMode::Face).map(|_| ())
        })
        .unwrap();
    let mesh = document.mesh();
    assert!(!mesh.has_modifiers());
    assert_eq!(mesh.face_count(), 16);
    mesh.validate().unwrap();
    assert!(is_closed_and_consistent(mesh));

    document.undo();
    document.undo();
    document.undo();
    assert_eq!(document.mesh(), &start);
}

#[test]
fn bad_modifiers_are_refused() {
    let mut mesh = unit_cube();
    let before = mesh.clone();
    let bad = Modifier::Array {
        count: 0,
        offset: [1.0, 0.0, 0.0],
        merge: None,
    };
    assert_eq!(
        mesh.add_modifier_to_selected(bad),
        Err(MeshError::InvalidModifier)
    );
    assert_eq!(mesh, before);
    let id = mesh.add_modifier_to_selected(Modifier::mirror()).unwrap();
    assert_eq!(
        mesh.set_modifier(id, Modifier::array()),
        Err(MeshError::InvalidModifier)
    );
    mesh.remove_modifier(id).unwrap();
    assert_eq!(mesh.remove_modifier(id), Err(MeshError::StaleModifier));
    assert!(matches!(mesh.evaluated(), std::borrow::Cow::Borrowed(_)));

    // Nothing selected, or two parts.
    let mut empty = Mesh::new();
    assert_eq!(
        empty.add_modifier_to_selected(Modifier::mirror()),
        Err(MeshError::NeedsOnePart)
    );
    let mut two = unit_cube();
    let more = two
        .add_primitive(Primitive::Cube { size: 1.0 }, [5.0, 0.0, 0.0])
        .unwrap();
    let more: Vec<Element> = more.into_iter().map(Element::Face).collect();
    two.select_elements(SelectMode::Face, &more, SelectOp::Add)
        .unwrap();
    assert_eq!(
        two.add_modifier_to_selected(Modifier::mirror()),
        Err(MeshError::NeedsOnePart)
    );
}

/// The top face of the cube, selected alone.
fn select_top(mesh: &mut Mesh) {
    let top = mesh
        .faces()
        .map(|(id, _)| id)
        .find(|&f| mesh.face_normal(f).unwrap().unwrap()[1] > 0.9)
        .unwrap();
    mesh.select_elements(SelectMode::Face, &[Element::Face(top)], SelectOp::Replace)
        .unwrap();
}

#[test]
fn new_faces_from_tools_join_the_modified_part() {
    let mut document = Document::new(unit_cube());
    document
        .edit("Add mirror", |m| {
            m.add_modifier_to_selected(Modifier::mirror()).map(|_| ())
        })
        .unwrap();
    document
        .edit("Select", |m| {
            select_top(m);
            Ok(())
        })
        .unwrap();
    document
        .edit("Extrude", |m| {
            m.extrude_selected_faces(SelectMode::Face)?;
            let top = m.selected_vertices();
            for v in top {
                let p = m.vertex(v).unwrap().position();
                m.set_vertex_position(v, [p[0], p[1] + 1.0, p[2]])?;
            }
            Ok(())
        })
        .unwrap();
    let mesh = document.mesh();
    assert_eq!(mesh.face_count(), 10);
    assert_eq!(mesh.modifiers_of_selected().len(), 1);
    // All ten faces are mirrored, and the two on the middle cancel out.
    let result = mesh.evaluated();
    result.validate().unwrap();
    assert!(is_closed_and_consistent(&result));
    assert_eq!(result.face_count(), 16);
    assert!((volume(&result) - 4.0).abs() < 1e-5);
}

#[test]
fn deleting_a_part_drops_its_modifiers() {
    let mut document = Document::new(unit_cube());
    document
        .edit("Add mirror", |m| {
            m.add_modifier_to_selected(Modifier::mirror()).map(|_| ())
        })
        .unwrap();
    document
        .edit("Delete", |m| m.delete_selected(SelectMode::Face))
        .unwrap();
    assert!(!document.mesh().has_modifiers());
    document.undo();
    assert!(document.mesh().has_modifiers());
}

#[test]
fn copies_keep_and_combining_refuses_modifiers() {
    let mut mesh = unit_cube();
    mesh.add_modifier_to_selected(Modifier::array()).unwrap();
    mesh.duplicate_selected(SelectMode::Face).unwrap();
    assert_eq!(mesh.modifiers().count(), 2);
    assert_eq!(mesh.modifiers_of_selected().len(), 1);
    // Both selected: two lists cannot become one.
    let all: Vec<Element> = mesh.faces().map(|(f, _)| Element::Face(f)).collect();
    mesh.select_elements(SelectMode::Face, &all, SelectOp::Replace)
        .unwrap();
    let before = mesh.clone();
    assert_eq!(mesh.join_selected_parts(), Err(MeshError::HasModifiers));
    assert_eq!(
        mesh.boolean_selected(mixer_mesh::BooleanOp::Union, None, SelectMode::Face),
        Err(MeshError::HasModifiers)
    );
    assert_eq!(mesh, before);
}

#[test]
fn joining_shares_and_separating_copies_the_list() {
    let mut document = Document::new(unit_cube());
    document
        .edit("Add mirror", |m| {
            m.add_modifier_to_selected(Modifier::mirror()).map(|_| ())
        })
        .unwrap();
    document
        .edit("Add cube", |m| {
            let faces = m.add_primitive(Primitive::Cube { size: 1.0 }, [0.5, 3.0, 0.5])?;
            let faces: Vec<Element> = faces.into_iter().map(Element::Face).collect();
            m.select_elements(SelectMode::Face, &faces, SelectOp::Add)
        })
        .unwrap();
    document
        .edit("Join", |m| m.join_selected_parts().map(|_| ()))
        .unwrap();
    assert_eq!(document.mesh().modifiers().count(), 1);
    assert_eq!(document.mesh().evaluated().face_count(), 20);
    document
        .edit("Separate", |m| m.separate_selected_parts().map(|_| ()))
        .unwrap();
    let mesh = document.mesh();
    assert_eq!(mesh.parts().len(), 2);
    assert_eq!(mesh.modifiers().count(), 2);
    assert_eq!(mesh.evaluated().face_count(), 20);
}

#[test]
fn exports_include_what_modifiers_make() {
    let mut mesh = unit_cube();
    mesh.add_modifier_to_selected(Modifier::mirror()).unwrap();
    let obj = mesh.to_obj("Box", 1.0).unwrap();
    let (back, _) = Mesh::from_text(&obj, FileKind::Obj).unwrap();
    assert_eq!(back.triangle_count(), mesh.evaluated().triangle_count());
    assert!(back.triangle_count() > mesh.triangle_count());
    assert!(mesh.to_glb(1.0).unwrap().len() > mesh.evaluated().triangle_count());
}

#[test]
fn a_shape_on_the_middle_stays_whole_when_mirrored() {
    // A cube centered on the middle is its own mirror image.
    let mut mesh = Mesh::new();
    let faces = mesh
        .add_primitive(Primitive::Cube { size: 2.0 }, [0.0; 3])
        .unwrap();
    let faces: Vec<Element> = faces.into_iter().map(Element::Face).collect();
    mesh.select_elements(SelectMode::Face, &faces, SelectOp::Replace)
        .unwrap();
    mesh.add_modifier_to_selected(Modifier::Mirror {
        axes: [true, true, true],
        merge: Some(0.001),
    })
    .unwrap();
    let result = mesh.evaluated();
    result.validate().unwrap();
    assert!(is_closed_and_consistent(&result));
    assert_eq!(result.face_count(), 6);
    assert!((volume(&result) - 8.0).abs() < 1e-5);
}
