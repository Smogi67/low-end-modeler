//! Primitives, and the OBJ text that comes out of them, read back with a small
//! separate parser so a mistake in the writer cannot hide itself.

use modeler_mesh::{Mesh, MeshError, Primitive};

const ALL: [Primitive; 7] = [
    Primitive::Cube { size: 2.0 },
    Primitive::Cone {
        radius: 1.0,
        height: 2.0,
        segments: 16,
    },
    Primitive::Cylinder {
        radius: 1.0,
        height: 2.0,
        segments: 16,
    },
    Primitive::UvSphere {
        radius: 1.0,
        segments: 16,
        rings: 8,
    },
    Primitive::Icosphere {
        radius: 1.0,
        subdivisions: 2,
    },
    Primitive::Grid {
        size: 2.0,
        columns: 4,
        rows: 3,
    },
    Primitive::Circle {
        radius: 1.0,
        segments: 16,
    },
];

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

#[derive(Default)]
struct Obj {
    positions: Vec<[f64; 3]>,
    normals: Vec<[f64; 3]>,
    triangles: Vec<[(usize, usize); 3]>,
    name: String,
}

fn parse(text: &str) -> Obj {
    let mut obj = Obj::default();
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let numbers = |parts: std::str::SplitWhitespace| -> Vec<f64> {
            parts.map(|p| p.parse().unwrap()).collect()
        };
        match parts.next() {
            Some("v") => obj.positions.push(numbers(parts).try_into().unwrap()),
            Some("vn") => obj.normals.push(numbers(parts).try_into().unwrap()),
            Some("o") => obj.name = parts.collect::<Vec<_>>().join(" "),
            Some("f") => {
                let corners: Vec<(usize, usize)> = parts
                    .map(|corner| {
                        let (v, n) = corner.split_once("//").unwrap();
                        (v.parse().unwrap(), n.parse().unwrap())
                    })
                    .collect();
                obj.triangles.push(corners.try_into().unwrap());
            }
            Some("#") | Some("s") | None => {}
            Some(other) => panic!("unexpected line kind {other}"),
        }
    }
    obj
}

fn signed_volume(obj: &Obj) -> f64 {
    obj.triangles
        .iter()
        .map(|t| {
            let [a, b, c] = t.map(|(v, _)| obj.positions[v - 1]);
            dot(a, cross(b, c)) / 6.0
        })
        .sum()
}

fn one(primitive: Primitive) -> Mesh {
    let mut mesh = Mesh::new();
    mesh.add_primitive(primitive, [0.0; 3]).unwrap();
    mesh.validate().unwrap();
    mesh
}

#[test]
fn every_primitive_is_valid_and_has_expected_counts() {
    // (vertices, faces, triangles)
    let expected = [
        (8, 6, 12),
        (17, 17, 30),
        (32, 18, 60),
        (16 * 7 + 2, 16 * 8, 16 * 2 + 16 * 6 * 2),
        (162, 320, 320),
        (20, 12, 24),
        (16, 1, 14),
    ];
    for (primitive, (v, f, t)) in ALL.into_iter().zip(expected) {
        let mesh = one(primitive);
        assert_eq!(mesh.vertex_count(), v, "{primitive:?}");
        assert_eq!(mesh.face_count(), f, "{primitive:?}");
        assert_eq!(mesh.triangle_count(), t, "{primitive:?}");
    }
}

#[test]
fn closed_primitives_are_closed_and_flat_ones_are_open() {
    for primitive in &ALL[..5] {
        let mesh = one(*primitive);
        assert_eq!(mesh.open_edge_count(), 0, "{primitive:?}");
        let euler =
            mesh.vertex_count() as i64 - mesh.edge_count() as i64 + mesh.face_count() as i64;
        assert_eq!(euler, 2, "{primitive:?}");
    }
    assert!(one(ALL[5]).open_edge_count() > 0);
    assert!(one(ALL[6]).open_edge_count() > 0);
}

#[test]
fn every_face_of_a_closed_primitive_points_outward() {
    for primitive in &ALL[..5] {
        let mesh = one(*primitive);
        for (face, _) in mesh.faces() {
            let normal = mesh.face_normal(face).unwrap().unwrap();
            let positions: Vec<[f32; 3]> = mesh
                .face_vertices(face)
                .unwrap()
                .map(|v| mesh.vertex(v).unwrap().position())
                .collect();
            let center: [f32; 3] = std::array::from_fn(|i| {
                positions.iter().map(|p| p[i]).sum::<f32>() / positions.len() as f32
            });
            let outward: f32 = (0..3).map(|i| normal[i] * center[i]).sum();
            assert!(outward > 0.0, "{primitive:?} has an inward face");
        }
    }
}

#[test]
fn flat_primitives_face_up() {
    for primitive in &ALL[5..] {
        let mesh = one(*primitive);
        for (face, _) in mesh.faces() {
            assert_eq!(mesh.face_normal(face).unwrap(), Some([0.0, 0.0, 1.0]));
        }
    }
}

#[test]
fn sizes_and_centers_are_respected() {
    let mut mesh = Mesh::new();
    mesh.add_primitive(Primitive::Cube { size: 4.0 }, [10.0, 20.0, 30.0])
        .unwrap();
    let p: Vec<_> = mesh.vertices().map(|(_, v)| v.position()).collect();
    for axis in 0..3 {
        let center = [10.0, 20.0, 30.0][axis];
        assert!(p.iter().all(|q| (q[axis] - center).abs() == 2.0));
    }
    let sphere = one(Primitive::Icosphere {
        radius: 3.0,
        subdivisions: 2,
    });
    for (_, v) in sphere.vertices() {
        let q = v.position();
        let r = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2]).sqrt();
        assert!((r - 3.0).abs() < 1e-4, "{r}");
    }
    let sphere = one(Primitive::UvSphere {
        radius: 3.0,
        segments: 8,
        rings: 2,
    });
    for (_, v) in sphere.vertices() {
        let q = v.position();
        let r = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2]).sqrt();
        assert!((r - 3.0).abs() < 1e-4, "{r}");
    }
}

#[test]
fn smallest_allowed_shapes_work() {
    for primitive in [
        Primitive::Cone {
            radius: 1.0,
            height: 1.0,
            segments: 3,
        },
        Primitive::Cylinder {
            radius: 1.0,
            height: 1.0,
            segments: 3,
        },
        Primitive::UvSphere {
            radius: 1.0,
            segments: 3,
            rings: 2,
        },
        Primitive::Icosphere {
            radius: 1.0,
            subdivisions: 0,
        },
        Primitive::Grid {
            size: 1.0,
            columns: 1,
            rows: 1,
        },
        Primitive::Circle {
            radius: 1.0,
            segments: 3,
        },
    ] {
        one(primitive);
    }
}

#[test]
fn bad_shapes_are_refused_without_changing_the_mesh() {
    for bad in [
        Primitive::Cube { size: 0.0 },
        Primitive::Cube { size: -1.0 },
        Primitive::Cube { size: f32::NAN },
        Primitive::Cone {
            radius: 0.0,
            height: 1.0,
            segments: 8,
        },
        Primitive::Cone {
            radius: 1.0,
            height: 0.0,
            segments: 8,
        },
        Primitive::Cone {
            radius: 1.0,
            height: 1.0,
            segments: 2,
        },
        Primitive::Cylinder {
            radius: 1.0,
            height: f32::INFINITY,
            segments: 8,
        },
        Primitive::Cylinder {
            radius: 1.0,
            height: 1.0,
            segments: 513,
        },
        Primitive::UvSphere {
            radius: 1.0,
            segments: 2,
            rings: 4,
        },
        Primitive::UvSphere {
            radius: 1.0,
            segments: 8,
            rings: 1,
        },
        Primitive::Icosphere {
            radius: -1.0,
            subdivisions: 1,
        },
        Primitive::Icosphere {
            radius: 1.0,
            subdivisions: 8,
        },
        Primitive::Grid {
            size: 1.0,
            columns: 0,
            rows: 1,
        },
        Primitive::Grid {
            size: 1.0,
            columns: 1,
            rows: 0,
        },
        Primitive::Circle {
            radius: 1.0,
            segments: 2,
        },
    ] {
        let mut mesh = Mesh::new();
        assert_eq!(
            mesh.add_primitive(bad, [0.0; 3]),
            Err(MeshError::InvalidPrimitive),
            "{bad:?}"
        );
        assert_eq!(mesh, Mesh::new());
    }
    let mut mesh = Mesh::new();
    assert_eq!(
        mesh.add_primitive(ALL[0], [f32::NAN, 0.0, 0.0]),
        Err(MeshError::NonFinitePosition)
    );
    assert_eq!(mesh, Mesh::new());
}

#[test]
fn obj_has_the_right_counts_for_every_primitive() {
    for primitive in ALL {
        let mesh = one(primitive);
        let obj = parse(&mesh.to_obj("Test", 1.0).unwrap());
        assert_eq!(obj.name, "Test");
        assert_eq!(obj.positions.len(), mesh.vertex_count());
        assert_eq!(obj.normals.len(), mesh.face_count());
        assert_eq!(obj.triangles.len(), mesh.triangle_count());
        for t in &obj.triangles {
            for &(v, n) in t {
                assert!(v >= 1 && v <= obj.positions.len());
                assert!(n >= 1 && n <= obj.normals.len());
            }
        }
    }
}

#[test]
fn obj_winding_matches_its_normals_and_points_outward() {
    for primitive in &ALL[..5] {
        let obj = parse(&one(*primitive).to_obj("Test", 1.0).unwrap());
        assert!(signed_volume(&obj) > 0.0, "{primitive:?} is inside out");
        for t in &obj.triangles {
            let [a, b, c] = t.map(|(v, _)| obj.positions[v - 1]);
            let geometric = cross(sub(b, a), sub(c, a));
            assert!(dot(geometric, obj.normals[t[0].1 - 1]) > 0.0);
            assert!(t[0].1 == t[1].1 && t[1].1 == t[2].1, "one normal per face");
        }
    }
}

#[test]
fn obj_volume_is_correct_and_follows_the_scale() {
    let cube = one(Primitive::Cube { size: 2.0 });
    let volume = signed_volume(&parse(&cube.to_obj("c", 1.0).unwrap()));
    assert!((volume - 8.0).abs() < 1e-3, "{volume}");
    let volume = signed_volume(&parse(&cube.to_obj("c", 3.0).unwrap()));
    assert!((volume - 8.0 * 27.0).abs() < 1e-2, "{volume}");
}

#[test]
fn obj_converts_z_up_to_y_up() {
    let mut mesh = Mesh::new();
    mesh.add_primitive(
        Primitive::Cone {
            radius: 1.0,
            height: 2.0,
            segments: 8,
        },
        [0.0; 3],
    )
    .unwrap();
    let obj = parse(&mesh.to_obj("c", 1.0).unwrap());
    // The cone's tip points along the modeler's +Z, which must become OBJ's +Y.
    let highest = obj.positions.iter().map(|p| p[1]).fold(f64::MIN, f64::max);
    assert!((highest - 1.0).abs() < 1e-5);
    let tips = obj
        .positions
        .iter()
        .filter(|p| (p[1] - 1.0).abs() < 1e-5)
        .count();
    assert_eq!(tips, 1);

    let mut mesh = Mesh::new();
    let a = mesh.add_vertex([1.0, 2.0, 3.0]).unwrap();
    let b = mesh.add_vertex([2.0, 2.0, 3.0]).unwrap();
    let c = mesh.add_vertex([1.0, 3.0, 3.0]).unwrap();
    mesh.add_face(&[a, b, c]).unwrap();
    let obj = parse(&mesh.to_obj("t", 1.0).unwrap());
    assert_eq!(obj.positions[0], [1.0, 3.0, -2.0]);
    // A face looking along +Z must look along +Y in the file.
    assert_eq!(obj.normals[0], [0.0, 1.0, 0.0]);
}

#[test]
fn obj_writes_no_negative_zeros() {
    let text = one(Primitive::Circle {
        radius: 1.0,
        segments: 8,
    })
    .to_obj("c", 1.0)
    .unwrap();
    assert!(!text.contains("-0 ") && !text.ends_with("-0"));
    assert!(text.lines().all(|line| !line.ends_with(" -0")));
}

#[test]
fn obj_leaves_out_loose_parts_and_copes_with_gaps_in_slots() {
    let mut mesh = Mesh::new();
    let dead = mesh.add_vertex([9.0; 3]).unwrap();
    let loose = mesh.add_vertex([8.0; 3]).unwrap();
    let a = mesh.add_vertex([0.0, 0.0, 0.0]).unwrap();
    let b = mesh.add_vertex([1.0, 0.0, 0.0]).unwrap();
    let c = mesh.add_vertex([0.0, 1.0, 0.0]).unwrap();
    mesh.add_face(&[a, b, c]).unwrap();
    let wire_end = mesh.add_vertex([7.0; 3]).unwrap();
    mesh.add_edge(a, wire_end).unwrap();
    mesh.remove_vertex(dead).unwrap();
    let _ = loose;
    let obj = parse(&mesh.to_obj("t", 1.0).unwrap());
    assert_eq!(obj.positions.len(), 3);
    assert_eq!(obj.triangles.len(), 1);
}

#[test]
fn obj_handles_empty_meshes_ngons_and_bad_input() {
    let empty = parse(&Mesh::new().to_obj("e", 1.0).unwrap());
    assert!(empty.positions.is_empty() && empty.triangles.is_empty());

    let circle = one(Primitive::Circle {
        radius: 1.0,
        segments: 12,
    });
    assert_eq!(parse(&circle.to_obj("c", 1.0).unwrap()).triangles.len(), 10);

    for bad in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert_eq!(circle.to_obj("c", bad), Err(MeshError::InvalidScale));
    }

    let named = parse(&circle.to_obj("two words\nf 1 2 3", 1.0).unwrap());
    assert_eq!(named.name, "two words f 1 2 3");
    assert_eq!(
        named.triangles.len(),
        10,
        "a name must not be able to add lines"
    );
}

#[test]
fn obj_writes_each_shape_in_a_mesh_with_two_shapes() {
    let mut mesh = Mesh::new();
    mesh.add_primitive(ALL[0], [-3.0, 0.0, 0.0]).unwrap();
    mesh.add_primitive(ALL[0], [3.0, 0.0, 0.0]).unwrap();
    let obj = parse(&mesh.to_obj("two", 1.0).unwrap());
    assert_eq!(obj.positions.len(), 16);
    assert_eq!(obj.triangles.len(), 24);
    assert!((signed_volume(&obj) - 16.0).abs() < 1e-2);
}
