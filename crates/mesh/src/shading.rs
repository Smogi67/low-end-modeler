//! Smooth shading. Each face is flat, or smooth up to an angle: where it meets a
//! neighbor at a gentler angle than that, the two are lit as one curved surface, and
//! sharper edges stay crisp, as Blender's Shade Auto Smooth does. Each corner of a
//! smooth face gets a normal averaged from the faces around its point, weighted by
//! their area and by the angle at that corner, so a big flat face next to a bevel
//! stays flat and the bevel looks round.

use std::collections::HashMap;

use crate::error::MeshError;
use crate::id::{EdgeId, FaceId, VertexId};
use crate::mesh::Mesh;

type V3 = [f32; 3];

/// The usual angle for auto smooth, as in Blender.
pub const AUTO_SMOOTH_ANGLE: u8 = 30;

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn length(a: V3) -> f32 {
    dot(a, a).sqrt()
}

/// The normal to light each face corner with.
#[derive(Debug, Clone, Default)]
pub struct ShadingNormals {
    flat: HashMap<FaceId, V3>,
    smooth: HashMap<(FaceId, VertexId), V3>,
}

impl ShadingNormals {
    /// The normal at a corner of a face. Faces without area face up.
    pub fn at(&self, face: FaceId, vertex: VertexId) -> V3 {
        self.smooth
            .get(&(face, vertex))
            .or_else(|| self.flat.get(&face))
            .copied()
            .unwrap_or([0.0, 0.0, 1.0])
    }
}

impl Mesh {
    /// Works out the normal of every face corner, for drawing and exporting.
    pub fn shading_normals(&self) -> ShadingNormals {
        let faces: Vec<FaceId> = self.faces().map(|(id, _)| id).collect();
        self.shading_normals_of(&faces)
    }

    /// The normals of these faces' corners only, for redrawing part of the mesh.
    pub fn shading_normals_of(&self, faces: &[FaceId]) -> ShadingNormals {
        let mut normals = ShadingNormals::default();
        // Each face's unit normal and area, worked out once when first needed.
        let mut flat: HashMap<FaceId, (V3, f32)> = HashMap::new();
        let mut face_data = |mesh: &Mesh, face: FaceId| -> (V3, f32) {
            *flat.entry(face).or_insert_with(|| {
                let points: Vec<V3> = mesh
                    .face_walk(face)
                    .map(|c| mesh.vertices[mesh.corners[c].vertex].position)
                    .collect();
                let n = crate::triangulate::newell(&points).map(|c| c as f32);
                let len = length(n);
                let unit = if len > 1e-12 {
                    n.map(|c| c / len)
                } else {
                    [0.0, 0.0, 1.0]
                };
                (unit, len / 2.0)
            })
        };
        for &face in faces {
            if !self.faces.contains(face) {
                continue;
            }
            let (own, _) = face_data(self, face);
            normals.flat.insert(face, own);
            let smooth = self.faces[face].smooth;
            if smooth == 0 {
                continue;
            }
            for corner in self.face_walk(face) {
                let vertex = self.corners[corner].vertex;
                let mut sum = [0.0f32; 3];
                // Faces past a sharp edge, going around the vertex, are left out.
                let reachable = self.reachable_without_sharp(corner);
                for around in self.corners_at(vertex) {
                    let other = self.corners[around].face;
                    if let Some(reachable) = &reachable
                        && !reachable.contains(&other)
                    {
                        continue;
                    }
                    let limit = self.faces[other].smooth.min(smooth);
                    if limit == 0 {
                        continue;
                    }
                    let (n, area) = face_data(self, other);
                    // Within the angle of both faces, and so smooth across.
                    if dot(own, n) < f32::from(limit).to_radians().cos() - 1e-4 {
                        continue;
                    }
                    let weight = area * self.corner_angle(around);
                    for k in 0..3 {
                        sum[k] += n[k] * weight;
                    }
                }
                let len = length(sum);
                if len > 1e-12 {
                    normals.smooth.insert((face, vertex), sum.map(|c| c / len));
                }
            }
        }
        normals
    }

    /// The faces around this corner's vertex that can be reached from its face by
    /// crossing edges that are not sharp, or `None` if no edge at the vertex is sharp.
    fn reachable_without_sharp(
        &self,
        corner: crate::id::CornerId,
    ) -> Option<std::collections::HashSet<FaceId>> {
        let vertex = self.corners[corner].vertex;
        if !self.disk_walk(vertex).any(|e| self.edges[e].sharp) {
            return None;
        }
        let mut reached = std::collections::HashSet::new();
        let mut waiting = vec![corner];
        reached.insert(self.corners[corner].face);
        while let Some(c) = waiting.pop() {
            // The two edges of this face at the vertex, and the faces across them.
            let data = self.corners[c];
            for edge in [data.edge, self.corners[data.prev].edge] {
                if self.edges[edge].sharp {
                    continue;
                }
                for across in self.radial_walk(edge) {
                    let face = self.corners[across].face;
                    if reached.contains(&face) {
                        continue;
                    }
                    // That face's corner at this vertex.
                    let Some(at) = self
                        .face_walk(face)
                        .find(|&k| self.corners[k].vertex == vertex)
                    else {
                        continue;
                    };
                    reached.insert(face);
                    waiting.push(at);
                }
            }
        }
        Some(reached)
    }

    /// Marks the selected edges sharp, or clears them. Returns how many changed.
    pub fn set_sharp_selected(&mut self, sharp: bool) -> Result<usize, MeshError> {
        let edges: Vec<EdgeId> = self
            .edges
            .iter()
            .filter(|(_, e)| e.selected && e.sharp != sharp)
            .map(|(id, _)| id)
            .collect();
        if edges.is_empty() && !self.edges.iter().any(|(_, e)| e.selected) {
            return Err(MeshError::NothingSelected);
        }
        for &edge in &edges {
            self.edges[edge].sharp = sharp;
        }
        Ok(edges.len())
    }

    /// The angle a face makes at one of its corners, in radians.
    fn corner_angle(&self, corner: crate::id::CornerId) -> f32 {
        let c = self.corners[corner];
        let here = self.vertices[c.vertex].position;
        let next = self.vertices[self.corners[c.next].vertex].position;
        let previous = self.vertices[self.corners[c.prev].vertex].position;
        let (a, b) = (sub(next, here), sub(previous, here));
        let (la, lb) = (length(a), length(b));
        if la < 1e-12 || lb < 1e-12 {
            return 0.0;
        }
        (dot(a, b) / (la * lb)).clamp(-1.0, 1.0).acos()
    }

    /// Sets the shading of the selected faces: 0 for flat, otherwise smooth up to
    /// that many degrees. Returns how many faces changed.
    pub fn set_shading_selected(&mut self, angle: u8) -> Result<usize, MeshError> {
        if angle > 180 {
            return Err(MeshError::InvalidSmoothAngle);
        }
        let faces: Vec<FaceId> = self
            .faces()
            .filter(|(_, f)| f.selected)
            .map(|(id, _)| id)
            .collect();
        if faces.is_empty() {
            return Err(MeshError::NoFacesSelected);
        }
        let mut changed = 0;
        for face in faces {
            if self.faces[face].smooth != angle {
                self.faces[face].smooth = angle;
                changed += 1;
            }
        }
        Ok(changed)
    }

    /// Faces made by an edit start flat. This gives each new flat face the shading of
    /// a smooth neighbor that was there before, spreading across new faces, so that
    /// bevelling or extruding a smooth shape keeps it smooth. `before` is every face
    /// that existed when the edit began.
    pub(crate) fn inherit_shading(&mut self, before: &std::collections::HashSet<FaceId>) {
        let mut waiting: Vec<FaceId> = self
            .faces()
            .filter(|(id, f)| f.smooth == 0 && !before.contains(id))
            .map(|(id, _)| id)
            .collect();
        loop {
            let found: Vec<(FaceId, u8)> = waiting
                .iter()
                .map(|&face| (face, self.smooth_neighbor(face)))
                .filter(|&(_, value)| value > 0)
                .collect();
            if found.is_empty() {
                break;
            }
            for (face, value) in found {
                self.faces[face].smooth = value;
            }
            waiting.retain(|&face| self.faces[face].smooth == 0);
        }
    }

    fn smooth_neighbor(&self, face: FaceId) -> u8 {
        let mut best = 0u8;
        for corner in self.face_walk(face) {
            for other in self.radial_walk(self.corners[corner].edge) {
                let neighbor = self.corners[other].face;
                if neighbor != face {
                    best = best.max(self.faces[neighbor].smooth);
                }
            }
        }
        best
    }
}
