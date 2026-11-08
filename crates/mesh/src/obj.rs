//! Wavefront OBJ export, for Roblox Studio and other tools.
//!
//! Faces are triangulated, with one normal per face to match flat shading. The
//! modeler is Z up and OBJ is conventionally Y up, so (x, y, z) is written as
//! (x, z, -y). That is a pure rotation, so winding and outward normals stay correct.

use std::fmt::Write;

use crate::error::MeshError;
use crate::id::VertexId;
use crate::mesh::Mesh;

/// Roblox allows at most this many triangles per mesh. Checked 2026-11-06 at
/// <https://create.roblox.com/docs/en-us/art/modeling/specifications>.
pub const ROBLOX_MAX_TRIANGLES: usize = 20_000;

fn to_y_up(p: [f32; 3], scale: f32) -> [f32; 3] {
    // Adding zero turns -0 into 0, which keeps the file tidy.
    [p[0] * scale + 0.0, p[2] * scale + 0.0, -p[1] * scale + 0.0]
}

impl Mesh {
    /// The mesh as OBJ text. `name` becomes the object's name, and every position is
    /// multiplied by `scale`. Loose vertices and wire edges are left out. A face with
    /// no area gets an upward normal, so the file stays valid.
    pub fn to_obj(&self, name: &str, scale: f32) -> Result<String, MeshError> {
        if !scale.is_finite() || scale <= 0.0 {
            return Err(MeshError::InvalidScale);
        }
        let mut out = String::with_capacity(self.corner_count() * 32 + 128);
        // Writing to a String cannot fail.
        let _ = writeln!(out, "# Mixer\n# {} triangles", self.triangle_count());
        let safe_name: String = name
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        let _ = writeln!(out, "o {safe_name}");

        // OBJ counts from 1, in the order vertices are written. Slots can have gaps,
        // so a lookup by slot maps each used vertex to its number.
        let slots = self
            .vertices()
            .map(|(id, _)| id.index() as usize + 1)
            .max()
            .unwrap_or(0);
        let mut numbers = vec![0u32; slots];
        let mut next = 1u32;
        let mut number = |id: VertexId| {
            let entry = &mut numbers[id.index() as usize];
            let new = *entry == 0;
            if new {
                *entry = next;
                next += 1;
            }
            (*entry, new)
        };

        let mut body = String::with_capacity(self.corner_count() * 24);
        let mut normal_number = 0u32;
        let mut normals = String::with_capacity(self.face_count() * 24);
        for (face, _) in self.faces() {
            let [nx, ny, nz] = self.face_normal(face)?.unwrap_or([0.0, 0.0, 1.0]);
            let n = to_y_up([nx, ny, nz], 1.0);
            let _ = writeln!(normals, "vn {} {} {}", n[0], n[1], n[2]);
            normal_number += 1;
            for triangle in self.face_triangles(face)? {
                let _ = write!(body, "f");
                for vertex in triangle {
                    let (index, new) = number(vertex);
                    if new {
                        let p = to_y_up(self.vertices[vertex].position, scale);
                        let _ = writeln!(out, "v {} {} {}", p[0], p[1], p[2]);
                    }
                    let _ = write!(body, " {index}//{normal_number}");
                }
                let _ = writeln!(body);
            }
        }
        out.push_str(&normals);
        out.push_str("s off\n");
        out.push_str(&body);
        Ok(out)
    }
}
