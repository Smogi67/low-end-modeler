//! Wavefront OBJ export, for Roblox Studio and other tools.
//!
//! Faces are triangulated. Flat faces get one normal each, and smooth ones a normal
//! per point (see `shading.rs`), to match what Mixer shows. The
//! modeler is Z up and OBJ is conventionally Y up, so (x, y, z) is written as
//! (x, z, -y). That is a pure rotation, so winding and outward normals stay correct.

use std::collections::HashMap;
use std::fmt::Write;

use crate::error::MeshError;
use crate::id::VertexId;
use crate::mesh::Mesh;

/// The triangle limit Mixer warns about unless the settings say otherwise: Roblox's
/// limit per mesh, among others, checked 2026-11-06 at
/// <https://create.roblox.com/docs/en-us/art/modeling/specifications>.
pub const DEFAULT_TRIANGLE_LIMIT: usize = 20_000;

fn to_y_up(p: [f32; 3], scale: f32) -> [f32; 3] {
    // Adding zero turns -0 into 0, which keeps the file tidy.
    [p[0] * scale + 0.0, p[2] * scale + 0.0, -p[1] * scale + 0.0]
}

/// Material names for OBJ and MTL files: no spaces, and each one different.
pub(crate) fn file_material_names(mesh: &Mesh) -> Vec<(crate::MaterialId, String)> {
    let mut used: Vec<String> = Vec::new();
    mesh.materials()
        .into_iter()
        .map(|(id, m)| {
            let base: String = m
                .name()
                .chars()
                .map(|c| {
                    if c.is_alphanumeric() || c == '-' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            let mut name = base.clone();
            let mut n = 2;
            while used.contains(&name) || name == "Default" {
                name = format!("{base}_{n}");
                n += 1;
            }
            used.push(name.clone());
            (id, name)
        })
        .collect()
}

impl Mesh {
    /// The MTL file that goes with [`to_obj_with_materials`](Self::to_obj_with_materials),
    /// or `None` when the mesh has no materials. OBJ materials hold color and opacity;
    /// metal and roughness go in the `Pm` and `Pr` lines that newer programs read.
    pub fn to_mtl(&self) -> Option<String> {
        let names = file_material_names(self);
        if names.is_empty() {
            return None;
        }
        let mut out = String::from("# Mixer\n");
        let _ = writeln!(out, "newmtl Default\nKd 0.8 0.8 0.8\nd 1\n");
        for (id, name) in names {
            let m = self.materials[id];
            let [r, g, b, a] = m.color;
            let _ = writeln!(
                out,
                "newmtl {name}\nKd {r} {g} {b}\nd {a}\nPm {}\nPr {}\n",
                m.metal, m.roughness
            );
        }
        Some(out)
    }

    /// Like [`to_obj`](Self::to_obj), naming `library` (the MTL file's name) and each
    /// face's material when the mesh has materials.
    pub fn to_obj_with_materials(
        &self,
        name: &str,
        scale: f32,
        library: &str,
    ) -> Result<String, MeshError> {
        if self.has_modifiers() {
            return self.evaluated().to_obj_with_materials(name, scale, library);
        }
        self.obj_text(name, scale, Some(library))
    }

    /// The mesh as OBJ text. `name` becomes the object's name, and every position is
    /// multiplied by `scale`. Loose vertices and wire edges are left out. A face with
    /// no area gets an upward normal, so the file stays valid.
    pub fn to_obj(&self, name: &str, scale: f32) -> Result<String, MeshError> {
        if !scale.is_finite() || scale <= 0.0 {
            return Err(MeshError::InvalidScale);
        }
        // What modifiers make is what gets exported.
        if self.has_modifiers() {
            return self.evaluated().to_obj(name, scale);
        }
        self.obj_text(name, scale, None)
    }

    fn obj_text(&self, name: &str, scale: f32, library: Option<&str>) -> Result<String, MeshError> {
        if !scale.is_finite() || scale <= 0.0 {
            return Err(MeshError::InvalidScale);
        }
        let names = file_material_names(self);
        let library = library.filter(|_| !names.is_empty());
        let mut out = String::with_capacity(self.corner_count() * 32 + 128);
        // Writing to a String cannot fail.
        let _ = writeln!(out, "# Mixer\n# {} triangles", self.triangle_count());
        let safe_name: String = name
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        if let Some(library) = library {
            let _ = writeln!(out, "mtllib {library}");
        }
        let _ = writeln!(out, "o {safe_name}");
        let mut current: Option<&str> = None;

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
        let mut normals = String::with_capacity(self.face_count() * 24);
        // Each different normal is written once: one per flat face, and one per
        // point of a smooth surface.
        let shading = self.shading_normals();
        let mut normal_numbers: HashMap<[u32; 3], u32> = HashMap::new();
        for (face, f) in self.faces() {
            if library.is_some() {
                let wanted = f
                    .material
                    .and_then(|m| names.iter().find(|(id, _)| *id == m))
                    .map_or("Default", |(_, n)| n.as_str());
                if current != Some(wanted) {
                    let _ = writeln!(body, "usemtl {wanted}");
                    current = Some(wanted);
                }
            }
            for triangle in self.face_triangles(face)? {
                let _ = write!(body, "f");
                for vertex in triangle {
                    let (index, new) = number(vertex);
                    if new {
                        let p = to_y_up(self.vertices[vertex].position, scale);
                        let _ = writeln!(out, "v {} {} {}", p[0], p[1], p[2]);
                    }
                    let n = to_y_up(shading.at(face, vertex), 1.0);
                    let key = n.map(|c| (c + 0.0).to_bits());
                    let count = normal_numbers.len() as u32;
                    let normal_number = *normal_numbers.entry(key).or_insert_with(|| {
                        let _ = writeln!(normals, "vn {} {} {}", n[0], n[1], n[2]);
                        count + 1
                    });
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
