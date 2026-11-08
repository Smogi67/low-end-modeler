//! Knife and bisect: cutting faces along a line.
//!
//! The knife takes a path of points, each a corner or a spot on an edge, and cuts
//! every face between two neighboring points of the path. Bisect cuts the selected
//! faces where a flat plane passes through them.

use std::collections::HashSet;

use crate::error::MeshError;
use crate::id::{EdgeId, FaceId, VertexId};
use crate::mesh::Mesh;
use crate::selection::SelectMode;

type V3 = [f32; 3];

/// Splits closer to an end than this use the end instead.
const END: f32 = 1e-4;

/// One point of a knife path.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CutPoint {
    /// An existing corner.
    Vertex(VertexId),
    /// A spot on an edge.
    Edge {
        /// The edge.
        edge: EdgeId,
        /// How far along, from `edge.vertices()[0]` at 0 to `edge.vertices()[1]` at 1.
        factor: f32,
    },
}

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

impl Mesh {
    /// Cuts along a knife path. Each pair of neighboring points that share a face
    /// becomes a new edge across it; pairs that do not are skipped, so a path may
    /// leave the mesh and come back. Selects the cut. Returns how many edges it made.
    pub fn knife(&mut self, path: &[CutPoint], mode: SelectMode) -> Result<usize, MeshError> {
        for point in path {
            match *point {
                CutPoint::Vertex(v) => self.require_vertex(v)?,
                CutPoint::Edge { edge, factor } => {
                    self.require_edge(edge)?;
                    if !(0.0..=1.0).contains(&factor) {
                        return Err(MeshError::InvalidFactor);
                    }
                }
            }
        }
        let mut trial = self.clone();
        trial.knife_unchecked(path, mode)?;
        self.knife_unchecked(path, mode)
    }

    fn knife_unchecked(&mut self, path: &[CutPoint], mode: SelectMode) -> Result<usize, MeshError> {
        // Every spot on one edge, split in order from its first end.
        let mut spots: Vec<(EdgeId, f32, usize)> = Vec::new();
        let mut vertices: Vec<Option<VertexId>> = vec![None; path.len()];
        for (i, point) in path.iter().enumerate() {
            match *point {
                CutPoint::Vertex(v) => vertices[i] = Some(v),
                CutPoint::Edge { edge, factor } => spots.push((edge, factor, i)),
            }
        }
        spots.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
        let mut k = 0;
        while k < spots.len() {
            let edge = spots[k].0;
            let [start, end] = self.edges[edge].vertices;
            let mut current = edge;
            let mut done = 0.0f32;
            let mut last: Option<(f32, VertexId)> = None;
            while k < spots.len() && spots[k].0 == edge {
                let (_, factor, i) = spots[k];
                k += 1;
                let vertex = if let Some((_, v)) = last.filter(|(f, _)| (factor - f).abs() < END) {
                    v
                } else if factor < END {
                    start
                } else if factor > 1.0 - END {
                    end
                } else {
                    let local = ((factor - done) / (1.0 - done)).clamp(0.0, 1.0);
                    let (middle, rest) = self.split_edge(current, local)?;
                    current = rest;
                    done = factor;
                    middle
                };
                last = Some((factor, vertex));
                vertices[i] = Some(vertex);
            }
        }
        let mut chain: Vec<VertexId> = vertices.into_iter().flatten().collect();
        chain.dedup();

        let mut made: Vec<VertexId> = Vec::new();
        let mut count = 0;
        for pair in chain.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if let Some(face) = self.face_to_cut(a, b) {
                self.split_face(face, a, b)?;
                count += 1;
                made.extend([a, b]);
            }
        }
        if count == 0 {
            return Err(MeshError::NothingToCut);
        }
        self.select_cut(&made, mode);
        Ok(count)
    }

    /// A face with both corners that does not already have them side by side.
    fn face_to_cut(&self, a: VertexId, b: VertexId) -> Option<FaceId> {
        if a == b {
            return None;
        }
        let faces: Vec<FaceId> = self.corners_at(a).map(|c| self.corners[c].face).collect();
        faces.into_iter().find(|&face| {
            self.face_walk(face).any(|c| self.corners[c].vertex == b)
                && !self.face_walk(face).any(|c| {
                    let corner = self.corners[c];
                    let next = self.corners[corner.next].vertex;
                    (corner.vertex == a && next == b) || (corner.vertex == b && next == a)
                })
        })
    }

    /// Selects the edges between these corners, as Blender selects a knife cut.
    fn select_cut(&mut self, corners: &[VertexId], mode: SelectMode) {
        self.deselect_all();
        if mode == SelectMode::Face {
            return;
        }
        for &v in corners {
            self.vertices[v].selected = true;
        }
        self.flush_selection(SelectMode::Vertex);
        self.flush_selection(mode);
    }

    /// Cuts the selected faces where the plane through `point` facing `normal` passes
    /// through them. Selects the cut. Returns how many edges it made.
    pub fn bisect_selected(
        &mut self,
        point: V3,
        normal: V3,
        mode: SelectMode,
    ) -> Result<usize, MeshError> {
        let length = dot(normal, normal).sqrt();
        if length.is_nan() || length <= 1e-9 || !point.iter().all(|c| c.is_finite()) {
            return Err(MeshError::NonFinitePosition);
        }
        let normal = normal.map(|c| c / length);
        let faces: Vec<FaceId> = self
            .faces()
            .filter(|(_, f)| f.selected)
            .map(|(id, _)| id)
            .collect();
        if faces.is_empty() {
            return Err(MeshError::NoFacesSelected);
        }
        let mut trial = self.clone();
        trial.bisect_unchecked(&faces, point, normal, mode)?;
        self.bisect_unchecked(&faces, point, normal, mode)
    }

    fn bisect_unchecked(
        &mut self,
        faces: &[FaceId],
        point: V3,
        normal: V3,
        mode: SelectMode,
    ) -> Result<usize, MeshError> {
        // How close to the plane counts as on it, by the size of the selection.
        let mut size = 0.0f32;
        for &face in faces {
            for c in self.face_walk(face) {
                let p = self.vertices[self.corners[c].vertex].position;
                size = size.max(p.iter().map(|x| x.abs()).fold(0.0, f32::max));
            }
        }
        let eps = 1e-5 * size.max(1.0);
        let side = |mesh: &Mesh, v: VertexId| dot(sub(mesh.vertices[v].position, point), normal);

        // Split the edges that cross the plane.
        let mut edges: HashSet<EdgeId> = HashSet::new();
        for &face in faces {
            for c in self.face_walk(face) {
                edges.insert(self.corners[c].edge);
            }
        }
        let mut on: HashSet<VertexId> = HashSet::new();
        for &face in faces {
            for c in self.face_walk(face) {
                let v = self.corners[c].vertex;
                if side(self, v).abs() <= eps {
                    on.insert(v);
                }
            }
        }
        let mut edges: Vec<EdgeId> = edges.into_iter().collect();
        edges.sort_unstable();
        for edge in edges {
            let [a, b] = self.edges[edge].vertices;
            let (da, db) = (side(self, a), side(self, b));
            if da.abs() <= eps || db.abs() <= eps || (da > 0.0) == (db > 0.0) {
                continue;
            }
            let (middle, _) = self.split_edge(edge, da / (da - db))?;
            on.insert(middle);
        }

        // Join the points on the plane across each face, in pairs along the line.
        let mut made: Vec<VertexId> = Vec::new();
        let mut count = 0;
        for &face in faces {
            let Some(face_normal) = self.face_normal(face)? else {
                continue;
            };
            let along = cross(normal, face_normal);
            let mut points: Vec<VertexId> = self
                .face_walk(face)
                .map(|c| self.corners[c].vertex)
                .filter(|v| on.contains(v))
                .collect();
            points.sort_by(|&x, &y| {
                dot(self.vertices[x].position, along)
                    .total_cmp(&dot(self.vertices[y].position, along))
            });
            // Pieces of this face made so far, to find the one holding each pair.
            let mut pieces = vec![face];
            for pair in points.chunks(2).filter(|p| p.len() == 2) {
                let (a, b) = (pair[0], pair[1]);
                let target = pieces.iter().copied().find(|&piece| {
                    let corners: Vec<VertexId> = self
                        .face_walk(piece)
                        .map(|c| self.corners[c].vertex)
                        .collect();
                    let (Some(i), Some(j)) = (
                        corners.iter().position(|&v| v == a),
                        corners.iter().position(|&v| v == b),
                    ) else {
                        return false;
                    };
                    let n = corners.len();
                    (i + 1) % n != j && (j + 1) % n != i
                });
                let Some(piece) = target else {
                    continue;
                };
                // A bent face's cut must run inside it.
                let middle: V3 = std::array::from_fn(|k| {
                    (self.vertices[a].position[k] + self.vertices[b].position[k]) / 2.0
                });
                if !self.inside_face(piece, middle, face_normal) {
                    continue;
                }
                let (new_face, _) = self.split_face(piece, a, b)?;
                pieces.push(new_face);
                count += 1;
                made.extend([a, b]);
            }
        }
        if count == 0 {
            return Err(MeshError::NothingToCut);
        }
        self.select_cut(&made, mode);
        Ok(count)
    }

    /// True if a point in a face's plane lies inside the face's outline.
    fn inside_face(&self, face: FaceId, p: V3, normal: V3) -> bool {
        // Drop the axis the normal points along most, and test in 2D.
        let axis = (0..3)
            .max_by(|&i, &j| normal[i].abs().total_cmp(&normal[j].abs()))
            .unwrap_or(2);
        let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
        let outline: Vec<[f32; 2]> = self
            .face_walk(face)
            .map(|c| {
                let q = self.vertices[self.corners[c].vertex].position;
                [q[u], q[v]]
            })
            .collect();
        let (x, y) = (p[u], p[v]);
        let mut inside = false;
        for i in 0..outline.len() {
            let a = outline[i];
            let b = outline[(i + 1) % outline.len()];
            if (a[1] > y) != (b[1] > y) {
                let cross_x = a[0] + (y - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
                if x < cross_x {
                    inside = !inside;
                }
            }
        }
        inside
    }
}
