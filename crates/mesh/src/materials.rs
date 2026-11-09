//! Materials: a name, a color, how metallic and how rough. The project keeps one list
//! (Blender keeps slots per object, but Mixer's parts are pieces of one mesh), and each
//! face uses one of them or none. Changes go through the mesh, so undo covers them.

use std::collections::HashSet;

use crate::error::MeshError;
use crate::id::{FaceId, MaterialId};
use crate::mesh::Mesh;

/// The longest name a material can have, in bytes.
pub const MAX_MATERIAL_NAME: usize = 31;
/// The most materials a project can have.
pub const MAX_MATERIALS: usize = 256;

/// How a surface looks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Material {
    name: [u8; MAX_MATERIAL_NAME],
    name_len: u8,
    /// Red, green, blue and opacity, from 0 to 1.
    pub color: [f32; 4],
    /// 0 for paint, plastic or wood; 1 for bare metal.
    pub metal: f32,
    /// 0 for a mirror finish; 1 for a dull one.
    pub roughness: f32,
    /// When it was made, so the list keeps that order.
    pub(crate) order: u32,
}

impl Material {
    /// A light grey, rough, non-metal material called `name`.
    pub fn new(name: &str) -> Self {
        let mut material = Self {
            name: [0; MAX_MATERIAL_NAME],
            name_len: 0,
            color: [0.8, 0.8, 0.8, 1.0],
            metal: 0.0,
            roughness: 0.5,
            order: 0,
        };
        material.set_name(name);
        material
    }

    /// The material's name.
    pub fn name(&self) -> &str {
        std::str::from_utf8(&self.name[..self.name_len as usize]).unwrap_or("Material")
    }

    /// Sets the name, cut at [`MAX_MATERIAL_NAME`] bytes on a character boundary, with
    /// line breaks turned into spaces. An empty name becomes "Material".
    pub fn set_name(&mut self, name: &str) {
        let clean: String = name
            .trim()
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        let clean = if clean.is_empty() {
            "Material".to_string()
        } else {
            clean
        };
        let mut end = clean.len().min(MAX_MATERIAL_NAME);
        while !clean.is_char_boundary(end) {
            end -= 1;
        }
        self.name = [0; MAX_MATERIAL_NAME];
        self.name[..end].copy_from_slice(&clean.as_bytes()[..end]);
        self.name_len = end as u8;
    }

    /// Brings every number into range; NaN becomes the default.
    pub fn clamped(mut self) -> Self {
        let fix = |v: f32, default: f32| {
            if v.is_finite() {
                v.clamp(0.0, 1.0)
            } else {
                default
            }
        };
        self.color = [
            fix(self.color[0], 0.8),
            fix(self.color[1], 0.8),
            fix(self.color[2], 0.8),
            fix(self.color[3], 1.0),
        ];
        self.metal = fix(self.metal, 0.0);
        self.roughness = fix(self.roughness, 0.5);
        self
    }
}

impl Mesh {
    /// The project's materials, in the order they were made.
    pub fn materials(&self) -> Vec<(MaterialId, Material)> {
        let mut list: Vec<(MaterialId, Material)> =
            self.materials.iter().map(|(id, m)| (id, *m)).collect();
        list.sort_by_key(|(_, m)| m.order);
        list
    }

    /// One material, if it still exists.
    pub fn material(&self, id: MaterialId) -> Option<Material> {
        self.materials.get(id).copied()
    }

    /// Adds a material at the end of the list.
    pub fn add_material(&mut self, material: Material) -> Result<MaterialId, MeshError> {
        if self.materials.live_count() >= MAX_MATERIALS {
            return Err(MeshError::TooManyMaterials);
        }
        if !self.materials.can_allocate(1) {
            return Err(MeshError::IdsExhausted);
        }
        let order = self
            .materials
            .iter()
            .map(|(_, m)| m.order + 1)
            .max()
            .unwrap_or(0);
        Ok(self.materials.insert_with(|_| Material {
            order,
            ..material.clamped()
        }))
    }

    /// Changes a material's name, color, metal or roughness.
    pub fn set_material(&mut self, id: MaterialId, material: Material) -> Result<(), MeshError> {
        let Some(old) = self.materials.get(id).copied() else {
            return Err(MeshError::StaleMaterial(id));
        };
        let new = Material {
            order: old.order,
            ..material.clamped()
        };
        if new != old {
            self.materials[id] = new;
        }
        Ok(())
    }

    /// Removes a material; its faces go back to the default look.
    pub fn remove_material(&mut self, id: MaterialId) -> Result<(), MeshError> {
        if !self.materials.contains(id) {
            return Err(MeshError::StaleMaterial(id));
        }
        let using: Vec<FaceId> = self
            .faces()
            .filter(|(_, f)| f.material == Some(id))
            .map(|(f, _)| f)
            .collect();
        for face in using {
            self.faces[face].material = None;
        }
        self.materials.remove(id);
        Ok(())
    }

    /// Gives the selected faces `material` (`None`: the default look). Returns how many
    /// changed.
    pub fn assign_material_selected(
        &mut self,
        material: Option<MaterialId>,
    ) -> Result<usize, MeshError> {
        if let Some(id) = material
            && !self.materials.contains(id)
        {
            return Err(MeshError::StaleMaterial(id));
        }
        let faces: Vec<FaceId> = self
            .faces()
            .filter(|(_, f)| f.is_selected())
            .map(|(id, _)| id)
            .collect();
        if faces.is_empty() {
            return Err(MeshError::NoFacesSelected);
        }
        let mut changed = 0;
        for face in faces {
            if self.faces[face].material != material {
                self.faces[face].material = material;
                changed += 1;
            }
        }
        Ok(changed)
    }

    /// The faces using `material` (`None`: the default look).
    pub fn faces_with_material(&self, material: Option<MaterialId>) -> Vec<FaceId> {
        self.faces()
            .filter(|(_, f)| f.material == material)
            .map(|(id, _)| id)
            .collect()
    }

    /// Sets a face's material directly, for tools and file loading.
    pub fn set_face_material(
        &mut self,
        face: FaceId,
        material: Option<MaterialId>,
    ) -> Result<(), MeshError> {
        self.require_face(face)?;
        if let Some(id) = material
            && !self.materials.contains(id)
        {
            return Err(MeshError::StaleMaterial(id));
        }
        self.faces[face].material = material;
        Ok(())
    }

    /// Faces made by an edit start with no material. This gives each such face the
    /// material of a neighbor, spreading across new faces, so that extruding or
    /// bevelling a red face keeps the new faces red. `before` is every face that existed
    /// when the edit began.
    pub(crate) fn inherit_materials(&mut self, before: &HashSet<FaceId>) {
        if self.materials.live_count() == 0 {
            return;
        }
        let mut waiting: Vec<FaceId> = self
            .faces()
            .filter(|(id, f)| f.material.is_none() && !before.contains(id))
            .map(|(id, _)| id)
            .collect();
        loop {
            let found: Vec<(FaceId, crate::id::MaterialId)> = waiting
                .iter()
                .filter_map(|&face| self.material_neighbor(face).map(|m| (face, m)))
                .collect();
            if found.is_empty() {
                break;
            }
            for (face, material) in found {
                self.faces[face].material = Some(material);
            }
            waiting.retain(|&face| self.faces[face].material.is_none());
        }
    }

    fn material_neighbor(&self, face: FaceId) -> Option<MaterialId> {
        for corner in self.face_walk(face) {
            for other in self.radial_walk(self.corners[corner].edge) {
                let neighbor = self.corners[other].face;
                if neighbor != face
                    && let Some(m) = self.faces[neighbor].material
                {
                    return Some(m);
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_cleaned_and_cut_on_a_character_boundary() {
        let mut m = Material::new("  Red\npaint ");
        assert_eq!(m.name(), "Red paint");
        m.set_name(&"é".repeat(40));
        assert!(m.name().len() <= MAX_MATERIAL_NAME);
        assert!(m.name().chars().all(|c| c == 'é'));
        m.set_name("");
        assert_eq!(m.name(), "Material");
        let wild = Material {
            color: [2.0, f32::NAN, -1.0, 0.5],
            metal: 9.0,
            roughness: f32::INFINITY,
            ..Material::new("x")
        }
        .clamped();
        assert_eq!(wild.color, [1.0, 0.8, 0.0, 0.5]);
        assert_eq!((wild.metal, wild.roughness), (1.0, 0.5));
    }
}
