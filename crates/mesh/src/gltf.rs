//! glTF export, as a single binary `.glb` file, for Roblox Studio and other tools.
//!
//! Each connected part becomes its own mesh and node, named Part1, Part2 and so on, so
//! Roblox's importer makes one MeshPart per part. Faces are triangulated and each
//! face's corners get the face's normal, which keeps the flat shading. glTF is Y up,
//! so positions turn the same way as in OBJ export: (x, y, z) becomes (x, z, -y).
//!
//! The file layout follows the glTF 2.0 specification: a 12-byte header, then a JSON
//! chunk and a binary chunk, each padded to four bytes.

use std::fmt::Write;

use crate::error::MeshError;
use crate::id::VertexId;
use crate::mesh::Mesh;

const ARRAY_BUFFER: u32 = 34962;
const ELEMENT_ARRAY_BUFFER: u32 = 34963;
const FLOAT: u32 = 5126;
const UNSIGNED_SHORT: u32 = 5123;
const UNSIGNED_INT: u32 = 5125;

fn to_y_up(p: [f32; 3], scale: f32) -> [f32; 3] {
    [p[0] * scale + 0.0, p[2] * scale + 0.0, -p[1] * scale + 0.0]
}

/// glTF's colors are linear light; Mixer's, like most color pickers', are sRGB, so they
/// are converted to look the same.
fn linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// A number for JSON, which has no NaN or infinity. Positions are already checked.
fn json_number(value: f32) -> String {
    if value.is_finite() {
        format!("{value}")
    } else {
        "0".to_string()
    }
}

impl Mesh {
    /// The mesh as a binary glTF file, with every position multiplied by `scale`.
    /// Loose vertices and edges are left out.
    pub fn to_glb(&self, scale: f32) -> Result<Vec<u8>, MeshError> {
        if !scale.is_finite() || scale <= 0.0 {
            return Err(MeshError::InvalidScale);
        }
        // What modifiers make is what gets exported.
        if self.has_modifiers() {
            return self.evaluated().to_glb(scale);
        }
        let mut bin: Vec<u8> = Vec::new();
        let mut views = String::new();
        let mut accessors = String::new();
        let mut meshes = String::new();
        let mut nodes = String::new();
        let mut view_count = 0;
        let mut accessor_count = 0;
        let parts = self.parts();

        let mut add_view = |bin: &mut Vec<u8>, bytes: &[u8], target: u32, views: &mut String| {
            while !bin.len().is_multiple_of(4) {
                bin.push(0);
            }
            let _ = write!(
                views,
                "{}{{\"buffer\":0,\"byteOffset\":{},\"byteLength\":{},\"target\":{target}}}",
                if view_count > 0 { "," } else { "" },
                bin.len(),
                bytes.len()
            );
            bin.extend_from_slice(bytes);
            view_count += 1;
            view_count - 1
        };

        // Materials: the project's, in order, then a plain one for faces without.
        let list = self.materials();
        let with_materials = !list.is_empty();
        let material_index = |m: Option<crate::MaterialId>| {
            m.and_then(|m| list.iter().position(|(id, _)| *id == m))
                .unwrap_or(list.len())
        };
        let shading = self.shading_normals();
        for (n, faces) in parts.iter().enumerate() {
            let mut positions: Vec<[f32; 3]> = Vec::new();
            let mut normals: Vec<[f32; 3]> = Vec::new();
            // Each corner's material color, linear, as COLOR_0: Roblox's importer keeps
            // vertex colors, while it may not read material colors.
            let mut colors: Vec<[f32; 4]> = Vec::new();
            // Triangle corners, per material used in this part.
            let mut by_material: Vec<(usize, Vec<u32>)> = Vec::new();
            for &face in faces {
                let which = material_index(self.faces[face].material);
                let slot = match by_material.iter().position(|(m, _)| *m == which) {
                    Some(slot) => slot,
                    None => {
                        by_material.push((which, Vec::new()));
                        by_material.len() - 1
                    }
                };
                let indices = &mut by_material[slot].1;
                let base = positions.len() as u32;
                let corners: Vec<VertexId> = self
                    .face_walk(face)
                    .map(|c| self.corners[c].vertex)
                    .collect();
                let color = self.faces[face]
                    .material
                    .and_then(|m| self.material(m))
                    .unwrap_or_else(|| crate::Material::new("Default"))
                    .color;
                let color = [
                    linear(color[0]),
                    linear(color[1]),
                    linear(color[2]),
                    color[3],
                ];
                for &v in &corners {
                    positions.push(to_y_up(self.vertices[v].position, scale));
                    normals.push(to_y_up(shading.at(face, v), 1.0));
                    colors.push(color);
                }
                for triangle in self.face_triangles(face)? {
                    for v in triangle {
                        let at = corners.iter().position(|&c| c == v).unwrap_or(0) as u32;
                        indices.push(base + at);
                    }
                }
            }
            let mut min = [f32::MAX; 3];
            let mut max = [f32::MIN; 3];
            for p in &positions {
                for i in 0..3 {
                    min[i] = min[i].min(p[i]);
                    max[i] = max[i].max(p[i]);
                }
            }
            let floats = |list: &[[f32; 3]]| -> Vec<u8> {
                list.iter()
                    .flat_map(|p| p.iter().flat_map(|c| c.to_le_bytes()))
                    .collect()
            };
            let wide = positions.len() > usize::from(u16::MAX);
            let position_view = add_view(&mut bin, &floats(&positions), ARRAY_BUFFER, &mut views);
            let normal_view = add_view(&mut bin, &floats(&normals), ARRAY_BUFFER, &mut views);
            let color_view = with_materials.then(|| {
                let bytes: Vec<u8> = colors
                    .iter()
                    .flat_map(|c| c.iter().flat_map(|v| v.to_le_bytes()))
                    .collect();
                add_view(&mut bin, &bytes, ARRAY_BUFFER, &mut views)
            });

            let comma = if accessor_count > 0 { "," } else { "" };
            let vec3 = |list: [f32; 3]| {
                format!(
                    "[{},{},{}]",
                    json_number(list[0]),
                    json_number(list[1]),
                    json_number(list[2])
                )
            };
            let _ = write!(
                accessors,
                "{comma}{{\"bufferView\":{position_view},\"componentType\":{FLOAT},\"count\":{},\"type\":\"VEC3\",\"min\":{},\"max\":{}}}\
                 ,{{\"bufferView\":{normal_view},\"componentType\":{FLOAT},\"count\":{},\"type\":\"VEC3\"}}",
                positions.len(),
                vec3(min),
                vec3(max),
                normals.len(),
            );
            let (p, nrm) = (accessor_count, accessor_count + 1);
            accessor_count += 2;
            let color_attribute = match color_view {
                Some(view) => {
                    let _ = write!(
                        accessors,
                        ",{{\"bufferView\":{view},\"componentType\":{FLOAT},\"count\":{},\"type\":\"VEC4\"}}",
                        colors.len()
                    );
                    accessor_count += 1;
                    format!(",\"COLOR_0\":{}", accessor_count - 1)
                }
                None => String::new(),
            };
            // One primitive per material, sharing the part's points.
            let mut primitives = String::new();
            for (k, (material, indices)) in by_material.iter().enumerate() {
                let index_bytes: Vec<u8> = if wide {
                    indices.iter().flat_map(|i| i.to_le_bytes()).collect()
                } else {
                    indices
                        .iter()
                        .flat_map(|&i| (i as u16).to_le_bytes())
                        .collect()
                };
                let index_view = add_view(&mut bin, &index_bytes, ELEMENT_ARRAY_BUFFER, &mut views);
                let _ = write!(
                    accessors,
                    ",{{\"bufferView\":{index_view},\"componentType\":{},\"count\":{},\"type\":\"SCALAR\"}}",
                    if wide { UNSIGNED_INT } else { UNSIGNED_SHORT },
                    indices.len()
                );
                let idx = accessor_count;
                accessor_count += 1;
                let material = if with_materials {
                    format!(",\"material\":{material}")
                } else {
                    String::new()
                };
                let _ = write!(
                    primitives,
                    "{}{{\"attributes\":{{\"POSITION\":{p},\"NORMAL\":{nrm}{color_attribute}}},\"indices\":{idx},\"mode\":4{material}}}",
                    if k > 0 { "," } else { "" }
                );
            }
            let comma = if n > 0 { "," } else { "" };
            let _ = write!(
                meshes,
                "{comma}{{\"name\":\"Part{0}Mesh\",\"primitives\":[{primitives}]}}",
                n + 1
            );
            let _ = write!(nodes, "{comma}{{\"name\":\"Part{0}\",\"mesh\":{n}}}", n + 1);
        }
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }

        let node_list: Vec<String> = (0..parts.len()).map(|i| i.to_string()).collect();
        // glTF forbids empty lists, so a file with nothing in it has a bare scene.
        let mut json = if parts.is_empty() {
            "{\"asset\":{\"version\":\"2.0\",\"generator\":\"Mixer\"},\"scene\":0,\"scenes\":[{}]"
                .to_string()
        } else {
            format!(
                "{{\"asset\":{{\"version\":\"2.0\",\"generator\":\"Mixer\"}},\"scene\":0,\
             \"scenes\":[{{\"nodes\":[{}]}}],\"nodes\":[{nodes}],\"meshes\":[{meshes}],\
             \"accessors\":[{accessors}],\"bufferViews\":[{views}]",
                node_list.join(",")
            )
        };
        if with_materials && !parts.is_empty() {
            // glTF's colors are linear light; Mixer's, like most color pickers', are
            // sRGB, so they are converted to look the same.
            let mut materials = String::new();
            let plain = crate::Material::new("Default");
            for (k, m) in list.iter().map(|(_, m)| m).chain([&plain]).enumerate() {
                let [r, g, b, a] = m.color;
                let name: String = m
                    .name()
                    .chars()
                    .filter(|c| !c.is_control() && *c != '"' && *c != '\\')
                    .collect();
                let _ = write!(
                    materials,
                    "{}{{\"name\":\"{name}\",\"pbrMetallicRoughness\":{{\"baseColorFactor\":[{},{},{},{}],\"metallicFactor\":{},\"roughnessFactor\":{}}}{}}}",
                    if k > 0 { "," } else { "" },
                    json_number(linear(r)),
                    json_number(linear(g)),
                    json_number(linear(b)),
                    json_number(a),
                    json_number(m.metal),
                    json_number(m.roughness),
                    if a < 1.0 {
                        ",\"alphaMode\":\"BLEND\""
                    } else {
                        ""
                    }
                );
            }
            let _ = write!(json, ",\"materials\":[{materials}]");
        }
        if bin.is_empty() {
            json.push('}');
        } else {
            let _ = write!(json, ",\"buffers\":[{{\"byteLength\":{}}}]}}", bin.len());
        }
        let mut json = json.into_bytes();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }

        let chunks = 8 + json.len() + if bin.is_empty() { 0 } else { 8 + bin.len() };
        let total = 12 + chunks;
        let mut out = Vec::with_capacity(total);
        out.extend_from_slice(b"glTF");
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&(total as u32).to_le_bytes());
        out.extend_from_slice(&(json.len() as u32).to_le_bytes());
        out.extend_from_slice(b"JSON");
        out.extend_from_slice(&json);
        if !bin.is_empty() {
            out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
            out.extend_from_slice(b"BIN\0");
            out.extend_from_slice(&bin);
        }
        Ok(out)
    }
}
