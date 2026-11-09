//! Materials: making, assigning, undo, tools carrying them, files and exports.

mod common;

use common::cube;
use mixer_mesh::{
    BevelSettings, Document, Element, FileKind, Material, Mesh, Primitive, SelectMode, SelectOp,
};

fn select_face(mesh: &mut Mesh, face: mixer_mesh::FaceId) {
    mesh.select_elements(SelectMode::Face, &[Element::Face(face)], SelectOp::Replace)
        .unwrap();
}

fn red() -> Material {
    let mut m = Material::new("Red paint");
    m.color = [0.9, 0.1, 0.1, 1.0];
    m.metal = 0.25;
    m.roughness = 0.75;
    m
}

#[test]
fn assigning_and_removing_is_undone_in_one_step_each() {
    let (mesh, _, faces) = cube();
    let mut doc = Document::new(mesh);
    let id = doc.edit("add", |m| m.add_material(red())).unwrap();
    doc.edit("assign", |m| {
        select_face(m, faces[0]);
        m.assign_material_selected(Some(id))
    })
    .unwrap();
    assert_eq!(doc.mesh().face(faces[0]).unwrap().material(), Some(id));
    assert_eq!(doc.mesh().faces_with_material(Some(id)), vec![faces[0]]);
    doc.edit("remove", |m| m.remove_material(id)).unwrap();
    assert!(doc.mesh().materials().is_empty());
    assert_eq!(doc.mesh().face(faces[0]).unwrap().material(), None);
    doc.undo();
    assert_eq!(doc.mesh().face(faces[0]).unwrap().material(), Some(id));
    assert_eq!(doc.mesh().material(id).unwrap().name(), "Red paint");
    doc.undo();
    assert_eq!(doc.mesh().face(faces[0]).unwrap().material(), None);
    doc.undo();
    assert!(doc.mesh().materials().is_empty());
}

#[test]
fn new_faces_take_their_neighbors_material() {
    let (mesh, _, faces) = cube();
    let mut doc = Document::new(mesh);
    let id = doc.edit("add", |m| m.add_material(red())).unwrap();
    doc.edit("assign", |m| {
        m.select_all();
        m.assign_material_selected(Some(id))
    })
    .unwrap();
    // Extruding a face makes four sides and a new top; all stay red.
    doc.edit("extrude", |m| {
        select_face(m, faces[0]);
        m.extrude_selected_faces(SelectMode::Face)
    })
    .unwrap();
    assert!(doc.mesh().faces().all(|(_, f)| f.material() == Some(id)));
    // A bevel too, unless it is given its own material (Blender's Material Index).
    let (mesh, _, _) = cube();
    let mut doc = Document::new(mesh);
    let id = doc.edit("add", |m| m.add_material(red())).unwrap();
    doc.edit("assign", |m| {
        m.select_all();
        m.assign_material_selected(Some(id))
    })
    .unwrap();
    let edge_material = doc
        .edit("add", |m| m.add_material(Material::new("Trim")))
        .unwrap();
    doc.edit("bevel", |m| {
        m.select_all();
        m.bevel_selected(
            &BevelSettings {
                width: 0.1,
                material: Some(edge_material),
                ..Default::default()
            },
            SelectMode::Edge,
        )
    })
    .unwrap();
    let mesh = doc.mesh();
    let trim = mesh.faces_with_material(Some(edge_material)).len();
    let painted = mesh.faces_with_material(Some(id)).len();
    assert!(trim > 0 && painted > 0, "{trim} {painted}");
    assert_eq!(trim + painted, mesh.face_count());
    // The cube's six faces stay red; bands and corners are trim.
    assert_eq!(painted, 6);
}

#[test]
fn duplicates_and_booleans_keep_materials() {
    let mut mesh = Mesh::new();
    mesh.add_primitive(Primitive::Cube { size: 2.0 }, [0.0; 3])
        .unwrap();
    let id = mesh.add_material(red()).unwrap();
    mesh.select_all();
    mesh.assign_material_selected(Some(id)).unwrap();
    mesh.duplicate_selected(SelectMode::Face).unwrap();
    assert!(mesh.faces().all(|(_, f)| f.material() == Some(id)));
    assert_eq!(mesh.face_count(), 12);

    // A plain cube cut by the red one: the cut's walls come from the red cube.
    let mut mesh = Mesh::new();
    mesh.add_primitive(Primitive::Cube { size: 2.0 }, [0.0; 3])
        .unwrap();
    let cutter = mesh
        .add_primitive(Primitive::Cube { size: 1.0 }, [1.0, 1.0, 1.0])
        .unwrap();
    let id = mesh.add_material(red()).unwrap();
    for &f in &cutter {
        mesh.set_face_material(f, Some(id)).unwrap();
    }
    mesh.select_all();
    mesh.boolean_selected(
        mixer_mesh::BooleanOp::Subtract,
        Some(cutter[0]),
        SelectMode::Face,
    )
    .unwrap();
    let red_faces = mesh.faces_with_material(Some(id)).len();
    let plain = mesh.faces_with_material(None).len();
    assert!(red_faces >= 3 && plain >= 6, "{red_faces} {plain}");
}

#[test]
fn projects_keep_materials_and_old_files_still_open() {
    let (mut mesh, _, faces) = cube();
    let id = mesh.add_material(red()).unwrap();
    let mut blue = Material::new("Blue");
    blue.color = [0.1, 0.2, 0.9, 0.5];
    let blue = mesh.add_material(blue).unwrap();
    mesh.set_face_material(faces[1], Some(id)).unwrap();
    mesh.set_face_material(faces[2], Some(blue)).unwrap();
    let text = mesh.to_project();
    assert!(text.contains("mixer 2"));
    let (back, _) = Mesh::from_text(&text, FileKind::Project).unwrap();
    let names: Vec<String> = back
        .materials()
        .iter()
        .map(|(_, m)| m.name().to_string())
        .collect();
    assert_eq!(names, ["Red paint", "Blue"]);
    let (_, red_back) = back.materials()[0];
    assert_eq!(red_back.color, red().color);
    assert_eq!((red_back.metal, red_back.roughness), (0.25, 0.75));
    let used: Vec<Option<String>> = back
        .faces()
        .map(|(_, f)| {
            f.material()
                .map(|m| back.material(m).unwrap().name().to_string())
        })
        .collect();
    assert_eq!(used.iter().filter(|m| m.is_none()).count(), 4);
    assert_eq!(
        used.iter().filter(|m| m.as_deref() == Some("Blue")).count(),
        1
    );
    // Without materials, the file stays readable by older versions.
    let (plain, _, _) = cube();
    assert!(plain.to_project().contains("mixer 1"));
    // A face naming a material that is not there is refused, not misread.
    assert!(Mesh::from_text("mixer 2\nu 3\n", FileKind::Project).is_err());
}

#[test]
fn exports_carry_materials() {
    let (mut mesh, _, faces) = cube();
    let id = mesh.add_material(red()).unwrap();
    mesh.set_face_material(faces[0], Some(id)).unwrap();
    let obj = mesh.to_obj_with_materials("Box", 1.0, "Box.mtl").unwrap();
    assert!(obj.contains("mtllib Box.mtl"));
    assert!(obj.contains("usemtl Red_paint"));
    assert!(obj.contains("usemtl Default"));
    let mtl = mesh.to_mtl().unwrap();
    assert!(mtl.contains("newmtl Red_paint\nKd 0.9 0.1 0.1\nd 1\nPm 0.25\nPr 0.75"));
    // Plain OBJ export is unchanged.
    assert!(!mesh.to_obj("Box", 1.0).unwrap().contains("usemtl"));
    let (plain, _, _) = cube();
    assert!(plain.to_mtl().is_none());

    let glb = mesh.to_glb(1.0).unwrap();
    let json_len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
    let json = std::str::from_utf8(&glb[20..20 + json_len]).unwrap();
    assert!(
        json.contains("\"materials\":[{\"name\":\"Red paint\""),
        "{json}"
    );
    assert!(json.contains("\"metallicFactor\":0.25"));
    assert!(json.contains("\"name\":\"Default\""));
    // Two primitives in the one part: the red face and the rest.
    assert_eq!(json.matches("\"mode\":4").count(), 2);
    assert!(json.contains("\"material\":0") && json.contains("\"material\":1"));
    // Vertex colors too, which Roblox's importer keeps.
    assert!(json.contains("\"COLOR_0\""));
    assert!(
        !plain
            .to_glb(1.0)
            .is_ok_and(|g| String::from_utf8_lossy(&g).contains("COLOR_0"))
    );
}
