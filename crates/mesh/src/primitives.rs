//! Starting shapes. Each adds new, separate geometry to the mesh, with faces wound
//! counterclockwise when seen from outside, so normals point outward.

use std::collections::HashMap;
use std::f32::consts::{PI, TAU};

use crate::error::MeshError;
use crate::id::{FaceId, VertexId};
use crate::mesh::Mesh;

/// Most segments or cuts allowed. Far beyond low-poly use, it only guards against
/// a mistyped value using up all memory.
pub const MAX_SEGMENTS: u32 = 512;
/// Most icosphere subdivisions allowed. Level 7 already has 327,680 triangles.
pub const MAX_SUBDIVISIONS: u32 = 7;

/// A shape to add. Sizes are in model units, with Z up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Primitive {
    /// A cube `size` wide.
    Cube {
        /// Length of each side.
        size: f32,
    },
    /// A closed cone standing on Z, with its tip up.
    Cone {
        /// Radius of the base.
        radius: f32,
        /// Height from base to tip.
        height: f32,
        /// Sides around the base. At least 3.
        segments: u32,
    },
    /// A closed cylinder standing on Z.
    Cylinder {
        /// Radius of both caps.
        radius: f32,
        /// Distance between the caps.
        height: f32,
        /// Sides around. At least 3.
        segments: u32,
    },
    /// A sphere made of rings and segments.
    UvSphere {
        /// Radius.
        radius: f32,
        /// Slices around Z. At least 3.
        segments: u32,
        /// Bands from pole to pole. At least 2.
        rings: u32,
    },
    /// A sphere made of evenly sized triangles.
    Icosphere {
        /// Radius.
        radius: f32,
        /// Times each triangle is split into four. 0 gives an icosahedron.
        subdivisions: u32,
    },
    /// A flat square grid of quads on the XY plane, facing +Z.
    Grid {
        /// Length of each side.
        size: f32,
        /// Quads along X. At least 1.
        columns: u32,
        /// Quads along Y. At least 1.
        rows: u32,
    },
    /// A flat filled circle on the XY plane, facing +Z.
    Circle {
        /// Radius.
        radius: f32,
        /// Sides around. At least 3.
        segments: u32,
    },
}

impl Primitive {
    fn check(&self) -> Result<(), MeshError> {
        let bad = MeshError::InvalidPrimitive;
        let positive = |v: f32| v.is_finite() && v > 0.0;
        let count = |v: u32, min: u32| (min..=MAX_SEGMENTS).contains(&v);
        let ok = match *self {
            Primitive::Cube { size } => positive(size),
            Primitive::Cone {
                radius,
                height,
                segments,
            }
            | Primitive::Cylinder {
                radius,
                height,
                segments,
            } => positive(radius) && positive(height) && count(segments, 3),
            Primitive::UvSphere {
                radius,
                segments,
                rings,
            } => positive(radius) && count(segments, 3) && count(rings, 2),
            Primitive::Icosphere {
                radius,
                subdivisions,
            } => positive(radius) && subdivisions <= MAX_SUBDIVISIONS,
            Primitive::Grid {
                size,
                columns,
                rows,
            } => positive(size) && count(columns, 1) && count(rows, 1),
            Primitive::Circle { radius, segments } => positive(radius) && count(segments, 3),
        };
        if ok { Ok(()) } else { Err(bad) }
    }
}

fn ring(segments: u32, radius: f32, z: f32) -> impl Iterator<Item = [f32; 3]> {
    (0..segments).map(move |i| {
        let angle = TAU * i as f32 / segments as f32;
        [radius * angle.cos(), radius * angle.sin(), z]
    })
}

impl Mesh {
    /// Adds a shape centered on `center`, and returns its new faces. A refused shape
    /// changes nothing. If IDs run out part-way, which [`Document`](crate::Document)
    /// undoes automatically, a shape may be left incomplete outside an edit.
    pub fn add_primitive(
        &mut self,
        primitive: Primitive,
        center: [f32; 3],
    ) -> Result<Vec<FaceId>, MeshError> {
        primitive.check()?;
        if !center.iter().all(|c| c.is_finite()) {
            return Err(MeshError::NonFinitePosition);
        }
        let (positions, faces) = shape(primitive);
        let mut ids = Vec::with_capacity(positions.len());
        for p in positions {
            ids.push(self.add_vertex([p[0] + center[0], p[1] + center[1], p[2] + center[2]])?);
        }
        let mut out = Vec::with_capacity(faces.len());
        let mut cycle: Vec<VertexId> = Vec::new();
        for face in faces {
            cycle.clear();
            cycle.extend(face.iter().map(|&i| ids[i as usize]));
            out.push(self.add_face(&cycle)?);
        }
        Ok(out)
    }
}

type Shape = (Vec<[f32; 3]>, Vec<Vec<u32>>);

fn shape(primitive: Primitive) -> Shape {
    match primitive {
        Primitive::Cube { size } => cube(size / 2.0),
        Primitive::Cone {
            radius,
            height,
            segments,
        } => cone(radius, height, segments),
        Primitive::Cylinder {
            radius,
            height,
            segments,
        } => cylinder(radius, height, segments),
        Primitive::UvSphere {
            radius,
            segments,
            rings,
        } => uv_sphere(radius, segments, rings),
        Primitive::Icosphere {
            radius,
            subdivisions,
        } => icosphere(radius, subdivisions),
        Primitive::Grid {
            size,
            columns,
            rows,
        } => grid(size, columns, rows),
        Primitive::Circle { radius, segments } => (
            ring(segments, radius, 0.0).collect(),
            vec![(0..segments).collect()],
        ),
    }
}

fn cube(h: f32) -> Shape {
    let positions = vec![
        [-h, -h, -h],
        [h, -h, -h],
        [h, h, -h],
        [-h, h, -h],
        [-h, -h, h],
        [h, -h, h],
        [h, h, h],
        [-h, h, h],
    ];
    let faces = vec![
        vec![3, 2, 1, 0],
        vec![4, 5, 6, 7],
        vec![0, 1, 5, 4],
        vec![1, 2, 6, 5],
        vec![2, 3, 7, 6],
        vec![3, 0, 4, 7],
    ];
    (positions, faces)
}

fn cone(radius: f32, height: f32, segments: u32) -> Shape {
    let half = height / 2.0;
    let mut positions: Vec<_> = ring(segments, radius, -half).collect();
    let tip = segments;
    positions.push([0.0, 0.0, half]);
    let mut faces: Vec<Vec<u32>> = (0..segments)
        .map(|i| vec![i, (i + 1) % segments, tip])
        .collect();
    faces.push((0..segments).rev().collect());
    (positions, faces)
}

fn cylinder(radius: f32, height: f32, segments: u32) -> Shape {
    let half = height / 2.0;
    let mut positions: Vec<_> = ring(segments, radius, -half).collect();
    positions.extend(ring(segments, radius, half));
    let mut faces: Vec<Vec<u32>> = (0..segments)
        .map(|i| {
            let next = (i + 1) % segments;
            vec![i, next, next + segments, i + segments]
        })
        .collect();
    faces.push((0..segments).rev().collect());
    faces.push((segments..segments * 2).collect());
    (positions, faces)
}

fn uv_sphere(radius: f32, segments: u32, rings: u32) -> Shape {
    let mut positions = vec![[0.0, 0.0, -radius]];
    for level in 1..rings {
        let angle = PI * level as f32 / rings as f32 - PI / 2.0;
        positions.extend(ring(segments, radius * angle.cos(), radius * angle.sin()));
    }
    let top = positions.len() as u32;
    positions.push([0.0, 0.0, radius]);

    // Vertex `i` of band `level`, where band 0 is the first ring above the bottom pole.
    let at = |level: u32, i: u32| 1 + level * segments + i % segments;
    let mut faces = Vec::new();
    for i in 0..segments {
        faces.push(vec![0, at(0, i + 1), at(0, i)]);
        faces.push(vec![top, at(rings - 2, i), at(rings - 2, i + 1)]);
    }
    for level in 0..rings - 2 {
        for i in 0..segments {
            faces.push(vec![
                at(level, i),
                at(level, i + 1),
                at(level + 1, i + 1),
                at(level + 1, i),
            ]);
        }
    }
    (positions, faces)
}

fn normalize(p: [f32; 3], radius: f32) -> [f32; 3] {
    let length = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
    p.map(|c| c / length * radius)
}

fn icosphere(radius: f32, subdivisions: u32) -> Shape {
    let t = (1.0 + 5f32.sqrt()) / 2.0;
    let mut positions: Vec<[f32; 3]> = [
        [-1.0, t, 0.0],
        [1.0, t, 0.0],
        [-1.0, -t, 0.0],
        [1.0, -t, 0.0],
        [0.0, -1.0, t],
        [0.0, 1.0, t],
        [0.0, -1.0, -t],
        [0.0, 1.0, -t],
        [t, 0.0, -1.0],
        [t, 0.0, 1.0],
        [-t, 0.0, -1.0],
        [-t, 0.0, 1.0],
    ]
    .iter()
    .map(|&p| normalize(p, radius))
    .collect();
    let mut triangles: Vec<[u32; 3]> = vec![
        [0, 11, 5],
        [0, 5, 1],
        [0, 1, 7],
        [0, 7, 10],
        [0, 10, 11],
        [1, 5, 9],
        [5, 11, 4],
        [11, 10, 2],
        [10, 7, 6],
        [7, 1, 8],
        [3, 9, 4],
        [3, 4, 2],
        [3, 2, 6],
        [3, 6, 8],
        [3, 8, 9],
        [4, 9, 5],
        [2, 4, 11],
        [6, 2, 10],
        [8, 6, 7],
        [9, 8, 1],
    ];
    for _ in 0..subdivisions {
        let mut middles: HashMap<(u32, u32), u32> = HashMap::new();
        let mut middle = |a: u32, b: u32, positions: &mut Vec<[f32; 3]>| {
            *middles.entry((a.min(b), a.max(b))).or_insert_with(|| {
                let (p, q) = (positions[a as usize], positions[b as usize]);
                let m = [p[0] + q[0], p[1] + q[1], p[2] + q[2]];
                positions.push(normalize(m, radius));
                positions.len() as u32 - 1
            })
        };
        let mut next = Vec::with_capacity(triangles.len() * 4);
        for [a, b, c] in triangles {
            let ab = middle(a, b, &mut positions);
            let bc = middle(b, c, &mut positions);
            let ca = middle(c, a, &mut positions);
            next.extend_from_slice(&[[a, ab, ca], [b, bc, ab], [c, ca, bc], [ab, bc, ca]]);
        }
        triangles = next;
    }
    (positions, triangles.iter().map(|t| t.to_vec()).collect())
}

fn grid(size: f32, columns: u32, rows: u32) -> Shape {
    let half = size / 2.0;
    let mut positions = Vec::with_capacity(((columns + 1) * (rows + 1)) as usize);
    for y in 0..=rows {
        for x in 0..=columns {
            positions.push([
                -half + size * x as f32 / columns as f32,
                -half + size * y as f32 / rows as f32,
                0.0,
            ]);
        }
    }
    let at = |x: u32, y: u32| y * (columns + 1) + x;
    let mut faces = Vec::with_capacity((columns * rows) as usize);
    for y in 0..rows {
        for x in 0..columns {
            faces.push(vec![at(x, y), at(x + 1, y), at(x + 1, y + 1), at(x, y + 1)]);
        }
    }
    (positions, faces)
}
