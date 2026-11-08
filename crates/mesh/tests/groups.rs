//! Joining parts into groups, and separating them.

use modeler_mesh::{Document, Element, FileKind, Mesh, MeshError, Primitive, SelectMode, SelectOp};

/// Three separate shapes: a cube, a cone, and a sphere, each its own part.
fn three_shapes() -> Mesh {
    let mut mesh = Mesh::new();
    mesh.add_primitive(Primitive::Cube { size: 2.0 }, [0.0; 3])
        .unwrap();
    mesh.add_primitive(
        Primitive::Cone {
            radius: 1.0,
            height: 2.0,
            segments: 8,
        },
        [4.0, 0.0, 0.0],
    )
    .unwrap();
    mesh.add_primitive(
        Primitive::Icosphere {
            subdivisions: 1,
            radius: 1.0,
        },
        [8.0, 0.0, 0.0],
    )
    .unwrap();
    mesh
}

/// Selects one face from each of the parts at these indices, as Part mode would after
/// clicking them.
fn pick_parts(mesh: &mut Mesh, which: &[usize]) {
    let parts = mesh.parts();
    let elements: Vec<Element> = which.iter().map(|&i| Element::Face(parts[i][0])).collect();
    let faces: Vec<Element> = mesh
        .linked_faces(&elements)
        .into_iter()
        .map(Element::Face)
        .collect();
    mesh.select_elements(SelectMode::Face, &faces, SelectOp::Replace)
        .unwrap();
}

#[test]
fn joined_parts_count_as_one_and_separate_again() {
    let mut mesh = three_shapes();
    assert_eq!(mesh.parts().len(), 3);
    pick_parts(&mut mesh, &[0, 2]);
    assert_eq!(mesh.join_selected_parts(), Ok(2));
    mesh.validate().unwrap();
    let parts = mesh.parts();
    assert_eq!(parts.len(), 2);
    let sizes: Vec<usize> = parts.iter().map(Vec::len).collect();
    assert!(
        sizes.contains(&(6 + 80)),
        "cube and sphere are one part: {sizes:?}"
    );

    // Clicking any face of the group picks the whole group.
    let cube_face = mesh.faces().next().unwrap().0;
    assert_eq!(mesh.linked_faces(&[Element::Face(cube_face)]).len(), 86);

    pick_parts(&mut mesh, &[0]);
    assert_eq!(mesh.separate_selected_parts(), Ok(2));
    assert_eq!(mesh.parts().len(), 3);
    assert!(mesh.faces().all(|(_, f)| f.group() == 0));
}

#[test]
fn joining_needs_two_parts_and_separating_needs_a_group() {
    let mut mesh = three_shapes();
    pick_parts(&mut mesh, &[1]);
    let before = mesh.clone();
    assert_eq!(mesh.join_selected_parts(), Err(MeshError::NeedsTwoParts));
    assert_eq!(mesh.separate_selected_parts(), Err(MeshError::NotJoined));
    assert_eq!(mesh, before);
}

#[test]
fn joining_two_groups_makes_one() {
    let mut mesh = three_shapes();
    pick_parts(&mut mesh, &[0, 1]);
    mesh.join_selected_parts().unwrap();
    // The group now counts as one part, so picking it and the sphere joins all three.
    pick_parts(&mut mesh, &[0, 1]);
    assert_eq!(mesh.join_selected_parts(), Ok(2));
    assert_eq!(mesh.parts().len(), 1);
}

#[test]
fn groups_undo_save_and_survive_edits() {
    let mut document = Document::new(three_shapes());
    document
        .edit("Select", |mesh| {
            pick_parts(mesh, &[0, 1]);
            Ok(())
        })
        .unwrap();
    document
        .edit("Join", |mesh| mesh.join_selected_parts().map(|_| ()))
        .unwrap();
    assert_eq!(document.mesh().parts().len(), 2);

    // Saved and opened again, the group is still there.
    let text = document.mesh().to_project();
    assert!(text.contains("\ng 1\n"));
    let (opened, _) = Mesh::from_text(&text, FileKind::Project).unwrap();
    assert_eq!(opened.parts().len(), 2);

    // Subdividing the grouped faces keeps the new faces in the group.
    document
        .edit("Subdivide", |mesh| {
            mesh.subdivide_selected(SelectMode::Face)
        })
        .unwrap();
    assert_eq!(document.mesh().parts().len(), 2);

    // Duplicating copies the group as a new group of its own.
    document
        .edit("Duplicate", |mesh| {
            mesh.duplicate_selected(SelectMode::Face)
        })
        .unwrap();
    assert_eq!(document.mesh().parts().len(), 3);
    document.mesh().validate().unwrap();

    document.undo();
    document.undo();
    document.undo();
    assert_eq!(document.mesh().parts().len(), 3);
    assert!(document.mesh().faces().all(|(_, f)| f.group() == 0));
}

#[test]
fn obj_groups_are_not_joins() {
    let text = "g a\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\ng b\nv 5 0 0\nv 6 0 0\nv 5 1 0\nf 4 5 6\n";
    let (mesh, _) = Mesh::from_text(text, FileKind::Obj).unwrap();
    assert_eq!(mesh.parts().len(), 2);
}

#[test]
fn a_bad_group_line_is_refused() {
    let text = "lem 1\nv 0 0 0\ng x\n";
    let error = Mesh::from_text(text, FileKind::Project).unwrap_err();
    assert_eq!(error.line, 3);
}
