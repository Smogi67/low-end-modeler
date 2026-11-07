//! Project files, OBJ import, and glTF export.

mod common;
use common::{cube, is_closed_and_consistent};
use modeler_mesh::{FileKind, Mesh, Primitive, SelectMode, VertexId};

fn position(mesh: &Mesh, v: VertexId) -> [f32; 3] {
    mesh.vertex(v).unwrap().position()
}

fn volume(mesh: &Mesh) -> f64 {
    let mut total = 0.0;
    for (face, _) in mesh.faces() {
        for t in mesh.face_triangles(face).unwrap() {
            let [a, b, c] = t.map(|v| position(mesh, v).map(f64::from));
            total += triple(a, b, c);
        }
    }
    total
}

fn triple(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> f64 {
    (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
        + a[2] * (b[0] * c[1] - b[1] * c[0]))
        / 6.0
}

/// Everything about a mesh that does not depend on its IDs: positions as exact bits,
/// faces as lists of vertex numbers, and loose edges.
type Canonical = (Vec<[u32; 3]>, Vec<Vec<usize>>, Vec<[usize; 2]>);

fn canonical(mesh: &Mesh) -> Canonical {
    let ids: Vec<VertexId> = mesh.vertices().map(|(id, _)| id).collect();
    let number = |v: VertexId| ids.iter().position(|&i| i == v).unwrap();
    let positions = ids
        .iter()
        .map(|&v| position(mesh, v).map(|c| (c + 0.0).to_bits()))
        .collect();
    let faces = mesh
        .faces()
        .map(|(f, _)| mesh.face_vertices(f).unwrap().map(number).collect())
        .collect();
    let mut wires: Vec<[usize; 2]> = mesh
        .edges()
        .filter(|&(e, _)| mesh.edge_corners(e).unwrap().count() == 0)
        .map(|(_, e)| {
            let [a, b] = e.vertices().map(number);
            [a.min(b), a.max(b)]
        })
        .collect();
    wires.sort();
    (positions, faces, wires)
}

fn busy_mesh() -> Mesh {
    let mut mesh = Mesh::new();
    mesh.add_primitive(Primitive::Cube { size: 2.0 }, [0.1, 0.2, 0.3])
        .unwrap();
    mesh.add_primitive(
        Primitive::Cylinder {
            radius: 0.7,
            height: 1.3,
            segments: 9,
        },
        [4.0, -1.0, 0.5],
    )
    .unwrap();
    mesh.add_primitive(
        Primitive::UvSphere {
            radius: 1.0 / 3.0,
            segments: 7,
            rings: 4,
        },
        [-3.0, 2.0, 1.0],
    )
    .unwrap();
    // Round off the cube: n-gons, odd floats and many faces.
    let cube_faces = mesh.linked_faces(&[modeler_mesh::Element::Vertex(
        mesh.vertices().next().unwrap().0,
    )]);
    let elements: Vec<_> = cube_faces
        .into_iter()
        .map(modeler_mesh::Element::Face)
        .collect();
    mesh.select_elements(SelectMode::Face, &elements, modeler_mesh::SelectOp::Replace)
        .unwrap();
    mesh.bevel_selected_edges(0.137, 3, SelectMode::Face)
        .unwrap();
    // A loose vertex and loose edges.
    let a = mesh.add_vertex([9.0, 9.0, 9.0]).unwrap();
    let b = mesh.add_vertex([-1e-7, 1e7, f32::MIN_POSITIVE]).unwrap();
    let c = mesh.add_vertex([0.0, -0.0, 5.5]).unwrap();
    mesh.add_edge(b, c).unwrap();
    let _ = a;
    mesh.validate().unwrap();
    mesh
}

#[test]
fn a_project_reads_back_exactly() {
    let mesh = busy_mesh();
    let text = mesh.to_project();
    assert!(text.starts_with("# Low-End Modeler project\nlem 1\n"));
    let (back, report) = Mesh::from_text(&text, FileKind::Project).unwrap();
    back.validate().unwrap();
    assert_eq!(report.skipped_faces, 0);
    assert_eq!(canonical(&back), canonical(&mesh));
    // Saving again gives the same text.
    assert_eq!(back.to_project(), text);
}

#[test]
fn an_empty_project_round_trips() {
    let (back, _) = Mesh::from_text(&Mesh::new().to_project(), FileKind::Project).unwrap();
    assert_eq!(back.vertex_count(), 0);
}

#[test]
fn bad_project_files_are_refused_with_a_line_number() {
    let cases = [
        ("v 0 0 0\n", 0, "not a Low-End Modeler project"),
        ("lem 99\n", 1, "newer version"),
        ("lem 1\nv 0 0\n", 2, "three numbers"),
        ("lem 1\nv 0 x 0\n", 2, "not a number"),
        ("lem 1\nv 0 nan 0\n", 2, "not a number"),
        ("lem 1\nv 0 0 0\nf 1 2 3\n", 3, "no vertex 2"),
        ("lem 1\nv 0 0 0\nf 1 0 1\n", 3, "no vertex 0"),
        ("lem 1\nbanana\n", 2, "not part of a project"),
    ];
    for (text, line, words) in cases {
        let error = Mesh::from_text(text, FileKind::Project).unwrap_err();
        assert_eq!(error.line, line, "{text:?}");
        assert!(error.to_string().contains(words), "{text:?}: {error}");
    }
}

#[test]
fn faces_that_cannot_exist_are_skipped_and_counted() {
    let text = "lem 1\nv 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\n\
                f 1 2 3 4\nf 4 3 2 1\nf 1 2 2\nf 1 2\nf 2 3 4 1\n";
    let (mesh, report) = Mesh::from_text(text, FileKind::Project).unwrap();
    // The first face is kept. Its reverse, a repeated vertex, too few corners, and the
    // same face started elsewhere are all skipped.
    assert_eq!(mesh.face_count(), 1);
    assert_eq!(report.skipped_faces, 4);
}

#[test]
fn obj_export_imports_back_to_the_same_shape() {
    let mesh = busy_mesh();
    let obj = mesh.to_obj("thing", 1.0).unwrap();
    let (back, report) = Mesh::from_text(&obj, FileKind::Obj).unwrap();
    back.validate().unwrap();
    assert_eq!(report.skipped_faces, 0);
    assert_eq!(back.triangle_count(), mesh.triangle_count());
    assert!((volume(&back) - volume(&mesh)).abs() < 1e-4);
    // Z up again: the highest point is the same.
    // Only faces are exported, so only their corners count.
    let top = |m: &Mesh| {
        m.faces()
            .flat_map(|(f, _)| m.face_vertices(f).unwrap().collect::<Vec<_>>())
            .map(|v| position(m, v)[2])
            .fold(f32::MIN, f32::max)
    };
    assert!((top(&back) - top(&mesh)).abs() < 1e-5);
}

#[test]
fn obj_files_from_other_programs_read() {
    // Texture and normal numbers, negative numbers, groups, comments and lines.
    let text = "# made elsewhere\nmtllib x.mtl\no Box\nv 0 0 0\nv 1 0 0\nv 1 0 -1\nv 0 0 -1\n\
                vt 0 0\nvn 0 1 0\ng side\nusemtl red\ns 1\nf 1/1/1 2/1/1 3/1/1 4/1/1\n\
                v 5 5 5\nv 6 5 5\nl -2 -1\ncurv 0 1\n";
    let (mesh, _) = Mesh::from_text(text, FileKind::Obj).unwrap();
    mesh.validate().unwrap();
    assert_eq!(
        (mesh.vertex_count(), mesh.face_count(), mesh.edge_count()),
        (6, 1, 5)
    );
    // The face was facing +Y in the file, which is up; here up is +Z.
    let face = mesh.faces().next().unwrap().0;
    let n = mesh.face_normal(face).unwrap().unwrap();
    assert!((n[2] - 1.0).abs() < 1e-5, "{n:?}");
}

/// Reads a .glb file: its JSON, and its binary chunk.
fn read_glb(bytes: &[u8]) -> (serde_json::Value, Vec<u8>) {
    let u32_at = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
    assert_eq!(&bytes[0..4], b"glTF");
    assert_eq!(u32_at(4), 2);
    assert_eq!(u32_at(8), bytes.len());
    let json_length = u32_at(12);
    assert_eq!(&bytes[16..20], b"JSON");
    assert_eq!(json_length % 4, 0);
    let json: serde_json::Value = serde_json::from_slice(&bytes[20..20 + json_length]).unwrap();
    let mut bin = Vec::new();
    let at = 20 + json_length;
    if at < bytes.len() {
        let bin_length = u32_at(at);
        assert_eq!(&bytes[at + 4..at + 8], b"BIN\0");
        assert_eq!(bin_length % 4, 0);
        bin = bytes[at + 8..at + 8 + bin_length].to_vec();
        assert_eq!(at + 8 + bin_length, bytes.len());
    }
    (json, bin)
}

/// The triangles of one mesh in the file, as positions.
fn triangles(json: &serde_json::Value, bin: &[u8], mesh: usize) -> Vec<[[f64; 3]; 3]> {
    let primitive = &json["meshes"][mesh]["primitives"][0];
    let accessor = |i: &serde_json::Value| &json["accessors"][i.as_u64().unwrap() as usize];
    let view =
        |a: &serde_json::Value| &json["bufferViews"][a["bufferView"].as_u64().unwrap() as usize];
    let floats = |a: &serde_json::Value| -> Vec<[f64; 3]> {
        let v = view(a);
        let start = v["byteOffset"].as_u64().unwrap() as usize;
        let count = a["count"].as_u64().unwrap() as usize;
        assert_eq!(v["byteLength"].as_u64().unwrap() as usize, count * 12);
        (0..count)
            .map(|i| {
                std::array::from_fn(|k| {
                    let o = start + i * 12 + k * 4;
                    f64::from(f32::from_le_bytes(bin[o..o + 4].try_into().unwrap()))
                })
            })
            .collect()
    };
    let positions_accessor = accessor(&primitive["attributes"]["POSITION"]);
    let positions = floats(positions_accessor);
    let normals = floats(accessor(&primitive["attributes"]["NORMAL"]));
    // The bounds glTF requires on positions must be right.
    for k in 0..3 {
        let lo = positions.iter().map(|p| p[k]).fold(f64::MAX, f64::min);
        let hi = positions.iter().map(|p| p[k]).fold(f64::MIN, f64::max);
        assert!((positions_accessor["min"][k].as_f64().unwrap() - lo).abs() < 1e-5);
        assert!((positions_accessor["max"][k].as_f64().unwrap() - hi).abs() < 1e-5);
    }
    let index = accessor(&primitive["indices"]);
    let v = view(index);
    let start = v["byteOffset"].as_u64().unwrap() as usize;
    let wide = index["componentType"].as_u64().unwrap() == 5125;
    let count = index["count"].as_u64().unwrap() as usize;
    let at = |i: usize| -> usize {
        if wide {
            u32::from_le_bytes(bin[start + i * 4..start + i * 4 + 4].try_into().unwrap()) as usize
        } else {
            u16::from_le_bytes(bin[start + i * 2..start + i * 2 + 2].try_into().unwrap()) as usize
        }
    };
    (0..count / 3)
        .map(|t| {
            let corners = [at(t * 3), at(t * 3 + 1), at(t * 3 + 2)];
            let [a, b, c] = corners.map(|i| positions[i]);
            // Each triangle winds the way its normal points.
            let n = normals[corners[0]];
            let cross = [
                (b[1] - a[1]) * (c[2] - a[2]) - (b[2] - a[2]) * (c[1] - a[1]),
                (b[2] - a[2]) * (c[0] - a[0]) - (b[0] - a[0]) * (c[2] - a[2]),
                (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]),
            ];
            let agree: f64 = (0..3).map(|k| cross[k] * n[k]).sum();
            assert!(agree >= -1e-9, "a triangle winds against its normal");
            [a, b, c]
        })
        .collect()
}

#[test]
fn glb_has_one_mesh_per_part_with_the_right_volume_and_winding() {
    let mesh = busy_mesh();
    let scale = 2.0;
    let bytes = mesh.to_glb(scale).unwrap();
    let (json, bin) = read_glb(&bytes);
    assert_eq!(json["asset"]["version"], "2.0");
    let parts = mesh.parts();
    assert_eq!(parts.len(), 3);
    assert_eq!(json["meshes"].as_array().unwrap().len(), 3);
    assert_eq!(json["nodes"][0]["name"], "Part1");
    assert_eq!(
        json["buffers"][0]["byteLength"].as_u64().unwrap() as usize,
        bin.len()
    );
    let mut total_volume = 0.0;
    let mut total_triangles = 0;
    for m in 0..3 {
        let tris = triangles(&json, &bin, m);
        total_triangles += tris.len();
        total_volume += tris.iter().map(|t| triple(t[0], t[1], t[2])).sum::<f64>();
    }
    assert_eq!(total_triangles, mesh.triangle_count());
    let expected = volume(&mesh) * f64::from(scale).powi(3);
    assert!(
        (total_volume - expected).abs() < 1e-3 * expected,
        "{total_volume} vs {expected}"
    );
}

#[test]
fn glb_turns_z_up_into_y_up() {
    let (mesh, _, _) = cube();
    assert!(is_closed_and_consistent(&mesh));
    let (json, bin) = read_glb(&mesh.to_glb(1.0).unwrap());
    let accessor = &json["accessors"][0];
    // The unit cube runs 0 to 1 in Z, which becomes Y, and 0 to 1 in Y, which becomes -Z.
    let numbers = |v: &serde_json::Value| -> Vec<f64> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_f64().unwrap())
            .collect()
    };
    assert_eq!(numbers(&accessor["min"]), [0.0, 0.0, -1.0]);
    assert_eq!(numbers(&accessor["max"]), [1.0, 1.0, 0.0]);
    let tris = triangles(&json, &bin, 0);
    let v: f64 = tris.iter().map(|t| triple(t[0], t[1], t[2])).sum();
    assert!((v - 1.0).abs() < 1e-6);
}

#[test]
fn glb_of_an_empty_mesh_is_still_a_valid_file() {
    let mut mesh = Mesh::new();
    mesh.add_vertex([1.0, 2.0, 3.0]).unwrap();
    let (json, bin) = read_glb(&mesh.to_glb(1.0).unwrap());
    assert!(bin.is_empty());
    assert!(json.get("meshes").is_none());
    assert!(json.get("buffers").is_none());
    assert!(mesh.to_glb(0.0).is_err());
    assert!(mesh.to_glb(f32::NAN).is_err());
}

#[test]
fn glb_uses_wide_indices_for_big_parts() {
    let mut mesh = Mesh::new();
    mesh.add_primitive(
        Primitive::Grid {
            size: 10.0,
            columns: 140,
            rows: 140,
        },
        [0.0; 3],
    )
    .unwrap();
    let (json, bin) = read_glb(&mesh.to_glb(1.0).unwrap());
    assert_eq!(json["accessors"][2]["componentType"], 5125);
    assert_eq!(triangles(&json, &bin, 0).len(), mesh.triangle_count());
}

/// Writes sample files for checking with the official glTF validator by hand.
#[test]
fn write_samples_when_asked() {
    if let Ok(folder) = std::env::var("GLB_SAMPLES") {
        std::fs::write(
            format!("{folder}/busy.glb"),
            busy_mesh().to_glb(1.0).unwrap(),
        )
        .unwrap();
        std::fs::write(format!("{folder}/cube.glb"), cube().0.to_glb(1.0).unwrap()).unwrap();
        std::fs::write(
            format!("{folder}/empty.glb"),
            Mesh::new().to_glb(1.0).unwrap(),
        )
        .unwrap();
    }
}
