//! Modeling tools that work on the selection: delete, extrude, inset, merge, fill and
//! subdivide.
//!
//! These combine several low-level edits, so run them inside a `Document` edit. If one
//! fails part-way, which only running out of IDs can cause, the edit restores the mesh.

use std::collections::{HashMap, HashSet};

use crate::error::MeshError;
use crate::id::{EdgeId, FaceId, VertexId};
use crate::mesh::Mesh;
use crate::selection::{Element, SelectMode, SelectOp};

type V3 = [f32; 3];

fn average(points: impl Iterator<Item = V3>) -> V3 {
    let mut sum = [0.0f64; 3];
    let mut count = 0u32;
    for p in points {
        for (total, value) in sum.iter_mut().zip(p) {
            *total += value as f64;
        }
        count += 1;
    }
    sum.map(|s| (s / f64::from(count.max(1))) as f32)
}

impl Mesh {
    fn selected_faces(&self) -> Vec<FaceId> {
        self.faces()
            .filter(|(_, f)| f.selected)
            .map(|(id, _)| id)
            .collect()
    }

    fn cycle(&self, face: FaceId) -> Vec<VertexId> {
        self.face_walk(face)
            .map(|c| self.corners[c].vertex)
            .collect()
    }

    fn position_of(&self, vertex: VertexId) -> V3 {
        self.vertices[vertex].position
    }

    /// Makes `faces` the selection, then updates the flags for `mode`.
    fn select_only_faces(&mut self, faces: &[FaceId], mode: SelectMode) -> Result<(), MeshError> {
        let elements: Vec<Element> = faces.iter().map(|&f| Element::Face(f)).collect();
        self.select_elements(SelectMode::Face, &elements, SelectOp::Replace)?;
        self.flush_selection(mode);
        Ok(())
    }

    /// The average normal of the selected faces, if they have a clear direction.
    pub fn selected_faces_normal(&self) -> Option<V3> {
        let mut sum = [0.0f32; 3];
        for face in self.selected_faces() {
            if let Ok(Some(normal)) = self.face_normal(face) {
                for (total, value) in sum.iter_mut().zip(normal) {
                    *total += value;
                }
            }
        }
        let length = (sum[0] * sum[0] + sum[1] * sum[1] + sum[2] * sum[2]).sqrt();
        (length > 1e-4).then(|| sum.map(|c| c / length))
    }

    /// Removes the selection. In vertex mode everything using a selected vertex goes too.
    /// In edge and face mode, parts left with nothing using them are cleaned up as well.
    pub fn delete_selected(&mut self, mode: SelectMode) -> Result<(), MeshError> {
        let vertices = self.selected_vertices();
        let edges: Vec<EdgeId> = self
            .edges()
            .filter(|(_, e)| e.selected)
            .map(|(id, _)| id)
            .collect();
        let faces = self.selected_faces();
        if vertices.is_empty() {
            return Err(MeshError::NothingSelected);
        }
        match mode {
            SelectMode::Vertex => {
                for vertex in vertices {
                    self.remove_vertex(vertex)?;
                }
            }
            SelectMode::Edge | SelectMode::Face => {
                if mode == SelectMode::Face {
                    for face in faces {
                        self.remove_face(face)?;
                    }
                }
                for edge in edges {
                    let unused = self.edges[edge].first_corner.is_none();
                    if mode == SelectMode::Edge || unused {
                        self.remove_edge(edge)?;
                    }
                }
                for vertex in vertices {
                    if self.vertices[vertex].first_edge.is_none() {
                        self.remove_vertex(vertex)?;
                    }
                }
            }
        }
        self.flush_selection(mode);
        Ok(())
    }

    /// Extrudes the selected faces as one region: they get new vertices at the same
    /// positions, and side walls join their outline to where they were. Move the
    /// selection afterwards to see it. The new faces become the selection.
    pub fn extrude_selected_faces(&mut self, mode: SelectMode) -> Result<(), MeshError> {
        let faces = self.selected_faces();
        if faces.is_empty() {
            return Err(MeshError::NoFacesSelected);
        }
        let selected: HashSet<FaceId> = faces.iter().copied().collect();
        let cycles: Vec<Vec<VertexId>> = faces.iter().map(|&f| self.cycle(f)).collect();

        // An outline edge is used by exactly one face of the region.
        let mut outline: Vec<[VertexId; 2]> = Vec::new();
        let mut region_edges: HashSet<EdgeId> = HashSet::new();
        for &face in &faces {
            for corner in self.face_walk(face).collect::<Vec<_>>() {
                let data = self.corners[corner];
                region_edges.insert(data.edge);
                let uses = self
                    .radial_walk(data.edge)
                    .filter(|&c| selected.contains(&self.corners[c].face))
                    .count();
                if uses == 1 {
                    outline.push([data.vertex, self.corners[data.next].vertex]);
                }
            }
        }

        let old: Vec<VertexId> = {
            let mut seen = HashSet::new();
            cycles
                .iter()
                .flatten()
                .copied()
                .filter(|&v| seen.insert(v))
                .collect()
        };
        let mut map: HashMap<VertexId, VertexId> = HashMap::with_capacity(old.len());
        for &vertex in &old {
            map.insert(vertex, self.add_vertex(self.position_of(vertex))?);
        }

        for face in faces {
            self.remove_face(face)?;
        }
        for [a, b] in outline {
            self.add_face(&[a, b, map[&b], map[&a]])?;
        }
        let mut new_faces = Vec::with_capacity(cycles.len());
        for cycle in &cycles {
            let moved: Vec<VertexId> = cycle.iter().map(|v| map[v]).collect();
            new_faces.push(self.add_face(&moved)?);
        }

        // What was inside the region now has nothing using it.
        for edge in region_edges {
            if self.edges.contains(edge) && self.edges[edge].first_corner.is_none() {
                self.remove_edge(edge)?;
            }
        }
        for vertex in old {
            if self.vertices[vertex].first_edge.is_none() {
                self.remove_vertex(vertex)?;
            }
        }
        self.select_only_faces(&new_faces, mode)
    }

    /// Insets each selected face separately: a smaller copy appears inside it, joined
    /// by a ring of quads. The copy starts at full size, so move its vertices toward
    /// the returned centers to see it. The inner faces become the selection.
    pub fn inset_selected_faces(
        &mut self,
        mode: SelectMode,
    ) -> Result<Vec<(VertexId, V3)>, MeshError> {
        let faces = self.selected_faces();
        if faces.is_empty() {
            return Err(MeshError::NoFacesSelected);
        }
        let mut moving = Vec::new();
        let mut inner_faces = Vec::with_capacity(faces.len());
        for face in faces {
            let cycle = self.cycle(face);
            let center = average(cycle.iter().map(|&v| self.position_of(v)));
            let mut inner = Vec::with_capacity(cycle.len());
            for &vertex in &cycle {
                let copy = self.add_vertex(self.position_of(vertex))?;
                inner.push(copy);
                moving.push((copy, center));
            }
            self.remove_face(face)?;
            let n = cycle.len();
            for i in 0..n {
                let j = (i + 1) % n;
                self.add_face(&[cycle[i], cycle[j], inner[j], inner[i]])?;
            }
            inner_faces.push(self.add_face(&inner)?);
        }
        self.select_only_faces(&inner_faces, mode)?;
        Ok(moving)
    }

    /// Replaces each group of vertices with one at the given position. Faces and edges
    /// follow. A face left with too few corners, or crossing itself, is removed.
    fn merge_groups(
        &mut self,
        groups: &[(Vec<VertexId>, V3)],
        mode: SelectMode,
    ) -> Result<(), MeshError> {
        let mut map: HashMap<VertexId, VertexId> = HashMap::new();
        let mut keepers = Vec::with_capacity(groups.len());
        for (group, position) in groups {
            let keeper = self.add_vertex(*position)?;
            keepers.push(keeper);
            for &vertex in group {
                map.insert(vertex, keeper);
            }
        }
        let mapped = |v: VertexId| map.get(&v).copied().unwrap_or(v);

        let mut affected_faces = HashSet::new();
        let mut wires = Vec::new();
        for &old in map.keys() {
            affected_faces.extend(self.corners_at(old).map(|c| self.corners[c].face));
            for edge in self.disk_walk(old) {
                if self.edges[edge].first_corner.is_none() {
                    let [a, b] = self.edges[edge].vertices;
                    wires.push([mapped(a), mapped(b)]);
                }
            }
        }
        let mut cycles = Vec::with_capacity(affected_faces.len());
        for &face in &affected_faces {
            let mut cycle: Vec<VertexId> = self.cycle(face).into_iter().map(mapped).collect();
            cycle.dedup();
            if cycle.len() > 1 && cycle.first() == cycle.last() {
                cycle.pop();
            }
            cycles.push(cycle);
        }

        for old in map.keys().copied().collect::<Vec<_>>() {
            self.remove_vertex(old)?;
        }
        for cycle in cycles {
            let unique: HashSet<_> = cycle.iter().collect();
            if cycle.len() >= 3 && unique.len() == cycle.len() {
                match self.add_face(&cycle) {
                    Ok(_) | Err(MeshError::FaceExists(_)) => {}
                    Err(error) => return Err(error),
                }
            }
        }
        for [a, b] in wires {
            if a != b && self.find_edge(a, b).is_none() {
                self.add_edge(a, b)?;
            }
        }
        self.deselect_all();
        for keeper in keepers {
            self.vertices[keeper].selected = true;
        }
        self.flush_selection(SelectMode::Vertex);
        self.flush_selection(mode);
        Ok(())
    }

    /// Merges all selected vertices into one at their center.
    pub fn merge_selected_at_center(&mut self, mode: SelectMode) -> Result<(), MeshError> {
        let selected = self.selected_vertices();
        if selected.len() < 2 {
            return Err(MeshError::NothingSelected);
        }
        let center = average(selected.iter().map(|&v| self.position_of(v)));
        self.merge_groups(&[(selected, center)], mode)
    }

    /// Merges selected vertices closer together than `distance`. Returns how many
    /// vertices were removed.
    pub fn merge_selected_by_distance(
        &mut self,
        distance: f32,
        mode: SelectMode,
    ) -> Result<usize, MeshError> {
        if !distance.is_finite() || distance <= 0.0 {
            return Err(MeshError::InvalidDistance);
        }
        let selected = self.selected_vertices();
        let cell = |p: V3| p.map(|c| (c / distance).floor() as i64);
        let mut cells: HashMap<[i64; 3], Vec<usize>> = HashMap::new();
        for (i, &vertex) in selected.iter().enumerate() {
            cells
                .entry(cell(self.position_of(vertex)))
                .or_default()
                .push(i);
        }

        // Joins close pairs. Close points are always in neighboring cells.
        let mut parent: Vec<usize> = (0..selected.len()).collect();
        fn root(parent: &mut [usize], mut i: usize) -> usize {
            while parent[i] != i {
                parent[i] = parent[parent[i]];
                i = parent[i];
            }
            i
        }
        for (i, &vertex) in selected.iter().enumerate() {
            let p = self.position_of(vertex);
            let c = cell(p);
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        let Some(others) = cells.get(&[c[0] + dx, c[1] + dy, c[2] + dz]) else {
                            continue;
                        };
                        for &j in others {
                            let q = self.position_of(selected[j]);
                            let squared = (0..3).map(|k| (p[k] - q[k]).powi(2)).sum::<f32>();
                            if j != i && squared <= distance * distance {
                                let (a, b) = (root(&mut parent, i), root(&mut parent, j));
                                parent[a] = b;
                            }
                        }
                    }
                }
            }
        }
        let mut clusters: HashMap<usize, Vec<VertexId>> = HashMap::new();
        for (i, &vertex) in selected.iter().enumerate() {
            clusters
                .entry(root(&mut parent, i))
                .or_default()
                .push(vertex);
        }
        let groups: Vec<(Vec<VertexId>, V3)> = clusters
            .into_values()
            .filter(|group| group.len() > 1)
            .map(|group| {
                let center = average(group.iter().map(|&v| self.position_of(v)));
                (group, center)
            })
            .collect();
        let removed = groups.iter().map(|(group, _)| group.len() - 1).sum();
        if !groups.is_empty() {
            self.merge_groups(&groups, mode)?;
        }
        Ok(removed)
    }

    /// Joins the selection: two vertices get an edge, and three or more get a face. An
    /// open loop's order is followed when the selected edges form one. Otherwise the
    /// vertices are ordered around their center. The new face winds to match neighbors.
    pub fn fill_selected(&mut self, mode: SelectMode) -> Result<(), MeshError> {
        let selected = self.selected_vertices();
        match selected.len() {
            0 | 1 => return Err(MeshError::NothingSelected),
            2 => {
                if self.find_edge(selected[0], selected[1]).is_some() {
                    return Err(MeshError::CannotFill);
                }
                self.add_edge(selected[0], selected[1])?;
                self.flush_selection(SelectMode::Vertex);
                self.flush_selection(mode);
                return Ok(());
            }
            _ => {}
        }
        let mut cycle = self
            .selected_edge_loop(&selected)
            .unwrap_or_else(|| self.ordered_around_center(&selected));

        // A neighbor running a -> b needs this face to run b -> a.
        let n = cycle.len();
        let mut agree = 0i32;
        for i in 0..n {
            let (a, b) = (cycle[i], cycle[(i + 1) % n]);
            if let Some(edge) = self.find_edge(a, b) {
                for corner in self.radial_walk(edge) {
                    agree += if self.corners[corner].vertex == a {
                        -1
                    } else {
                        1
                    };
                }
            }
        }
        if agree < 0 {
            cycle.reverse();
        }
        let face = self.add_face(&cycle).map_err(|error| match error {
            MeshError::FaceExists(_) => MeshError::CannotFill,
            other => other,
        })?;
        self.select_only_faces(&[face], mode)
    }

    /// The selected vertices in order, if selected edges with room for a face join them
    /// into exactly one closed loop.
    fn selected_edge_loop(&self, selected: &[VertexId]) -> Option<Vec<VertexId>> {
        let chosen: HashSet<VertexId> = selected.iter().copied().collect();
        let mut neighbors: HashMap<VertexId, Vec<VertexId>> = HashMap::new();
        for (id, edge) in self.edges() {
            let [a, b] = edge.vertices;
            if chosen.contains(&a)
                && chosen.contains(&b)
                && self.radial_walk(id).take(2).count() < 2
            {
                neighbors.entry(a).or_default().push(b);
                neighbors.entry(b).or_default().push(a);
            }
        }
        if selected
            .iter()
            .any(|v| neighbors.get(v).is_none_or(|n| n.len() != 2))
        {
            return None;
        }
        let start = selected[0];
        let mut cycle = vec![start];
        let mut previous = start;
        let mut current = neighbors[&start][0];
        while current != start {
            cycle.push(current);
            if cycle.len() > selected.len() {
                return None;
            }
            let next = neighbors[&current]
                .iter()
                .copied()
                .find(|&v| v != previous)?;
            previous = current;
            current = next;
        }
        (cycle.len() == selected.len()).then_some(cycle)
    }

    fn ordered_around_center(&self, selected: &[VertexId]) -> Vec<VertexId> {
        let points: Vec<V3> = selected.iter().map(|&v| self.position_of(v)).collect();
        let center = average(points.iter().copied());
        // The widest spread gives one direction. A second, across it, completes a plane.
        let offset = |p: V3| [p[0] - center[0], p[1] - center[1], p[2] - center[2]];
        let dot = |a: V3, b: V3| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        let u = points
            .iter()
            .map(|&p| offset(p))
            .max_by(|&a, &b| dot(a, a).total_cmp(&dot(b, b)))
            .unwrap_or([1.0, 0.0, 0.0]);
        let across = |p: V3| {
            let d = dot(p, u) / dot(u, u).max(1e-12);
            [p[0] - u[0] * d, p[1] - u[1] * d, p[2] - u[2] * d]
        };
        let v = points
            .iter()
            .map(|&p| across(offset(p)))
            .max_by(|&a, &b| dot(a, a).total_cmp(&dot(b, b)))
            .unwrap_or([0.0, 1.0, 0.0]);
        let (lu, lv) = (dot(u, u).sqrt().max(1e-9), dot(v, v).sqrt().max(1e-9));
        let mut ordered: Vec<(f32, VertexId)> = selected
            .iter()
            .zip(&points)
            .map(|(&id, &p)| {
                let q = offset(p);
                ((dot(q, v) / lv).atan2(dot(q, u) / lu), id)
            })
            .collect();
        ordered.sort_by(|a, b| a.0.total_cmp(&b.0));
        ordered.into_iter().map(|(_, id)| id).collect()
    }

    /// Cuts selected edges in half, and divides selected faces: triangles into four,
    /// and larger faces into one quad per corner around a new center vertex. Unselected
    /// neighbors keep their shape, and gain a corner on each cut edge.
    pub fn subdivide_selected(&mut self, mode: SelectMode) -> Result<(), MeshError> {
        let edges: Vec<EdgeId> = self
            .edges()
            .filter(|(_, e)| e.selected)
            .map(|(id, _)| id)
            .collect();
        if edges.is_empty() {
            return Err(MeshError::NothingSelected);
        }
        let faces = self.selected_faces();
        let originals: Vec<Vec<VertexId>> = faces.iter().map(|&f| self.cycle(f)).collect();
        let mut middle: HashMap<(VertexId, VertexId), VertexId> = HashMap::new();
        for edge in edges {
            let [a, b] = self.edges[edge].vertices;
            let (mid, _) = self.split_edge(edge, 0.5)?;
            middle.insert((a.min(b), a.max(b)), mid);
        }

        let mut new_faces = Vec::new();
        for (face, cycle) in faces.into_iter().zip(originals) {
            let n = cycle.len();
            let mids: Vec<VertexId> = (0..n)
                .map(|i| {
                    let (a, b) = (cycle[i], cycle[(i + 1) % n]);
                    middle[&(a.min(b), a.max(b))]
                })
                .collect();
            self.remove_face(face)?;
            if n == 3 {
                for i in 0..3 {
                    new_faces.push(self.add_face(&[cycle[i], mids[i], mids[(i + 2) % 3]])?);
                }
                new_faces.push(self.add_face(&mids)?);
            } else {
                let center = average(cycle.iter().map(|&v| self.position_of(v)));
                let hub = self.add_vertex(center)?;
                for i in 0..n {
                    let before = mids[(i + n - 1) % n];
                    new_faces.push(self.add_face(&[before, cycle[i], mids[i], hub])?);
                }
            }
        }

        // New faces join the selection. Each mode then tidies its own flags.
        for face in new_faces {
            self.faces[face].selected = true;
            for corner in self.face_walk(face).collect::<Vec<_>>() {
                let data = self.corners[corner];
                self.edges[data.edge].selected = true;
                self.vertices[data.vertex].selected = true;
            }
        }
        self.flush_selection(mode);
        Ok(())
    }
}
