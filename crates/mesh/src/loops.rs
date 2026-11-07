//! Edge loops and rings, and the tools built on them: loop cut, edge slide and bridge.
//!
//! - An **edge loop** runs along edges, end to end. In a grid of four-sided faces it
//!   goes straight through each vertex that has four edges, as in Blender. Along a
//!   hole's border it follows the border, along a face with more than four sides it
//!   follows that face's outline, and along loose edges it follows them.
//! - An **edge ring** runs across four-sided faces: from an edge to the opposite side
//!   of the face, then into the next face. A loop cut cuts every edge of a ring.
//!
//! As in `tools.rs`, run the editing tools inside a `Document` edit.

use std::collections::{HashMap, HashSet};

use crate::error::MeshError;
use crate::id::{CornerId, EdgeId, FaceId, VertexId};
use crate::mesh::Mesh;
use crate::selection::{Element, SelectMode, SelectOp};

type V3 = [f32; 3];

/// The most cuts one loop cut can make.
pub const MAX_LOOP_CUTS: usize = 32;

/// A walk across four-sided faces.
struct Ring {
    /// The ring's edges in order, each with the end that cut positions are measured
    /// from. These ends all lie on the same side of the ring.
    edges: Vec<(EdgeId, VertexId)>,
    /// `faces[i]` lies between `edges[i]` and the next edge. A closed ring has one
    /// face more than an open one: the face between its last and first edge.
    faces: Vec<FaceId>,
}

/// One direction of a ring walk: the edges reached, and the faces crossed to reach them.
type RingSide = (Vec<(EdgeId, VertexId)>, Vec<FaceId>);

/// One end of a slide: the vertex, where it starts, and where it would end up if slid
/// all the way to either side. A side with nothing to slide along is `None`.
pub type SlideRail = (VertexId, V3, [Option<V3>; 2]);

fn lerp(a: V3, b: V3, t: f32) -> V3 {
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
}

fn squared_distance(a: V3, b: V3) -> f32 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum()
}

impl Mesh {
    fn corners_on(&self, edge: EdgeId) -> Vec<CornerId> {
        self.radial_walk(edge).collect()
    }

    fn face_count_on(&self, edge: EdgeId) -> usize {
        self.radial_walk(edge).take(3).count()
    }

    /// The other edge of the corner's face that meets `vertex`, where the corner's own
    /// edge also meets `vertex`.
    fn neighbor_in_face(&self, corner: CornerId, vertex: VertexId) -> EdgeId {
        let data = self.corners[corner];
        if data.vertex == vertex {
            self.corners[data.prev].edge
        } else {
            self.corners[data.next].edge
        }
    }

    /// The edge that continues a loop through `at`, if there is exactly one.
    fn loop_continues(&self, edge: EdgeId, at: VertexId) -> Option<EdgeId> {
        let corners = self.corners_on(edge);
        let others: Vec<EdgeId> = self.disk_walk(at).filter(|&e| e != edge).collect();
        let pick = |candidates: Vec<EdgeId>| (candidates.len() == 1).then(|| candidates[0]);
        match corners.len() {
            // Straight across a vertex with four edges: not either edge sharing a face.
            2 => {
                if others.len() != 3 {
                    return None;
                }
                let beside: Vec<EdgeId> = corners
                    .iter()
                    .map(|&c| self.neighbor_in_face(c, at))
                    .collect();
                let next = pick(others.into_iter().filter(|e| !beside.contains(e)).collect())?;
                (self.face_count_on(next) == 2).then_some(next)
            }
            // Along the border of a hole.
            1 => pick(
                others
                    .into_iter()
                    .filter(|&e| self.face_count_on(e) == 1)
                    .collect(),
            ),
            // Along loose edges, through vertices where only two meet.
            0 => {
                let next = pick(others)?;
                (self.face_count_on(next) == 0).then_some(next)
            }
            _ => None,
        }
    }

    /// The edge loop through `edge`, in order along the loop.
    pub fn edge_loop(&self, edge: EdgeId) -> Result<Vec<EdgeId>, MeshError> {
        self.require_edge(edge)?;
        // An edge on a face with more than four sides, such as a cylinder's cap, follows
        // that face's outline, as in Blender.
        let corners = self.corners_on(edge);
        if corners.len() == 2
            && let Some(&hub) = corners
                .iter()
                .filter(|&&c| self.faces[self.corners[c].face].corner_count() > 4)
                .max_by_key(|&&c| self.faces[self.corners[c].face].corner_count())
        {
            let mut outline = Vec::new();
            let mut corner = hub;
            loop {
                outline.push(self.corners[corner].edge);
                corner = self.corners[corner].next;
                if corner == hub {
                    return Ok(outline);
                }
            }
        }
        let [a, b] = self.edges[edge].vertices;
        let mut seen = HashSet::from([edge]);
        let mut ahead = Vec::new();
        let (mut current, mut at) = (edge, b);
        while let Some(next) = self.loop_continues(current, at) {
            if next == edge {
                return Ok(std::iter::once(edge).chain(ahead).collect());
            }
            if !seen.insert(next) {
                break;
            }
            ahead.push(next);
            at = self.edges[next].other_vertex(at).unwrap_or(at);
            current = next;
        }
        let mut behind = Vec::new();
        let (mut current, mut at) = (edge, a);
        while let Some(next) = self.loop_continues(current, at) {
            if !seen.insert(next) {
                break;
            }
            behind.push(next);
            at = self.edges[next].other_vertex(at).unwrap_or(at);
            current = next;
        }
        behind.reverse();
        behind.push(edge);
        behind.extend(ahead);
        Ok(behind)
    }

    /// Walks across four-sided faces from `edge`, in both directions.
    fn ring_walk(&self, edge: EdgeId) -> Ring {
        let start_vertex = self.edges[edge].vertices[0];
        let mut seen_faces = HashSet::new();
        let mut seen_edges = HashSet::from([edge]);
        let starts = self.corners_on(edge);
        let mut sides: Vec<RingSide> = Vec::new();
        let mut closed = false;
        if starts.len() <= 2 {
            for &first in &starts {
                let (mut chain, mut faces) = (Vec::new(), Vec::new());
                let (mut corner, mut from) = (first, start_vertex);
                loop {
                    let data = self.corners[corner];
                    let face = data.face;
                    if self.faces[face].corner_count() != 4 || seen_faces.contains(&face) {
                        break;
                    }
                    // The quad runs p q r s; this corner runs p -> q, the opposite one
                    // r -> s. p lies across from s, and q across from r.
                    let p = data.vertex;
                    let opposite = self.corners[data.next].next;
                    let r = self.corners[opposite].vertex;
                    let s = self.corners[self.corners[opposite].next].vertex;
                    let next_from = if from == p { s } else { r };
                    let next_edge = self.corners[opposite].edge;
                    seen_faces.insert(face);
                    if next_edge == edge {
                        faces.push(face);
                        closed = true;
                        break;
                    }
                    if !seen_edges.insert(next_edge) {
                        break;
                    }
                    chain.push((next_edge, next_from));
                    faces.push(face);
                    let across = self.corners_on(next_edge);
                    if across.len() != 2 {
                        break;
                    }
                    corner = if across[0] == opposite {
                        across[1]
                    } else {
                        across[0]
                    };
                    from = next_from;
                }
                sides.push((chain, faces));
                if closed {
                    break;
                }
            }
        }
        let mut sides = sides.into_iter();
        let (ahead, ahead_faces) = sides.next().unwrap_or_default();
        let (mut edges, mut faces) = sides.next().unwrap_or_default();
        edges.reverse();
        faces.reverse();
        edges.push((edge, start_vertex));
        edges.extend(ahead);
        faces.extend(ahead_faces);
        Ring { edges, faces }
    }

    /// The edge ring through `edge`: the edges crossed by walking across four-sided faces.
    pub fn edge_ring(&self, edge: EdgeId) -> Result<Vec<EdgeId>, MeshError> {
        self.require_edge(edge)?;
        Ok(self
            .ring_walk(edge)
            .edges
            .into_iter()
            .map(|(e, _)| e)
            .collect())
    }

    /// The faces crossed by the edge ring through `edge`: a band of faces running
    /// across the edge, as Blender's face loop.
    pub fn face_loop(&self, edge: EdgeId) -> Result<Vec<FaceId>, MeshError> {
        self.require_edge(edge)?;
        Ok(self.ring_walk(edge).faces)
    }

    /// Selects the loop through `edge`: its edges in vertex and edge mode, and the band of
    /// faces across it in face mode.
    pub fn select_loop(
        &mut self,
        mode: SelectMode,
        edge: EdgeId,
        op: SelectOp,
    ) -> Result<(), MeshError> {
        let elements = match mode {
            SelectMode::Face => self
                .face_loop(edge)?
                .into_iter()
                .map(Element::Face)
                .collect(),
            _ => self.as_mode_elements(mode, &self.edge_loop(edge)?),
        };
        self.select_elements(mode, &elements, op)
    }

    fn as_mode_elements(&self, mode: SelectMode, edges: &[EdgeId]) -> Vec<Element> {
        match mode {
            SelectMode::Vertex => edges
                .iter()
                .flat_map(|&e| self.edges[e].vertices)
                .map(Element::Vertex)
                .collect(),
            _ => edges.iter().map(|&e| Element::Edge(e)).collect(),
        }
    }

    fn selected_edges(&self) -> Vec<EdgeId> {
        self.edges()
            .filter(|(_, e)| e.selected)
            .map(|(id, _)| id)
            .collect()
    }

    /// Grows each selected edge to its whole loop, or with `ring` to its whole ring.
    /// Works in vertex and edge mode.
    pub fn select_loops_of_selection(
        &mut self,
        mode: SelectMode,
        ring: bool,
    ) -> Result<(), MeshError> {
        if mode == SelectMode::Face {
            return Err(MeshError::NeedsEdgeMode);
        }
        let selected = self.selected_edges();
        if selected.is_empty() {
            return Err(MeshError::NothingSelected);
        }
        let mut grown = Vec::new();
        for edge in selected {
            if ring {
                grown.extend(self.edge_ring(edge)?);
            } else {
                grown.extend(self.edge_loop(edge)?);
            }
        }
        let elements = self.as_mode_elements(mode, &grown);
        self.select_elements(mode, &elements, SelectOp::Add)
    }

    /// Where each cut goes along a ring edge, as a share of the way from its start.
    fn cut_shares(cuts: usize) -> impl Iterator<Item = f32> {
        (1..=cuts).map(move |k| k as f32 / (cuts + 1) as f32)
    }

    fn check_cut(&self, edge: EdgeId, cuts: usize) -> Result<Ring, MeshError> {
        self.require_edge(edge)?;
        if !(1..=MAX_LOOP_CUTS).contains(&cuts) {
            return Err(MeshError::InvalidCutCount);
        }
        let ring = self.ring_walk(edge);
        if ring.faces.is_empty() {
            return Err(MeshError::CannotLoopCut);
        }
        Ok(ring)
    }

    /// The lines a loop cut through `edge` would add, for showing before cutting.
    pub fn loop_cut_preview(&self, edge: EdgeId, cuts: usize) -> Result<Vec<[V3; 2]>, MeshError> {
        let ring = self.check_cut(edge, cuts)?;
        let point = |(e, from): (EdgeId, VertexId), share: f32| {
            let end = self.edges[e].other_vertex(from).unwrap_or(from);
            lerp(
                self.vertices[from].position,
                self.vertices[end].position,
                share,
            )
        };
        let n = ring.edges.len();
        let mut lines = Vec::with_capacity(ring.faces.len() * cuts);
        for i in 0..ring.faces.len() {
            let (here, there) = (ring.edges[i], ring.edges[(i + 1) % n]);
            for share in Self::cut_shares(cuts) {
                lines.push([point(here, share), point(there, share)]);
            }
        }
        Ok(lines)
    }

    /// Cuts `cuts` evenly spaced loops across the ring through `edge`. Faces at the
    /// ends of an open ring gain corners where their edges were cut. The new edges become
    /// the selection, and are returned for sliding.
    pub fn loop_cut(
        &mut self,
        edge: EdgeId,
        cuts: usize,
        mode: SelectMode,
    ) -> Result<Vec<EdgeId>, MeshError> {
        let ring = self.check_cut(edge, cuts)?;
        let mut cut_vertices: Vec<Vec<VertexId>> = Vec::with_capacity(ring.edges.len());
        for &(ring_edge, start) in &ring.edges {
            let (mut rest, mut from) = (ring_edge, start);
            let mut made = Vec::with_capacity(cuts);
            for k in 1..=cuts {
                // Each cut takes an even share of what is left of the edge.
                let share = 1.0 / (cuts + 2 - k) as f32;
                let from_first_end = self.edges[rest].vertices[0] == from;
                let factor = if from_first_end { share } else { 1.0 - share };
                let (middle, new_edge) = self.split_edge(rest, factor)?;
                // split_edge keeps the first end on the old edge.
                if from_first_end {
                    rest = new_edge;
                }
                from = middle;
                made.push(middle);
            }
            cut_vertices.push(made);
        }

        let n = ring.edges.len();
        let mut new_edges = Vec::with_capacity(ring.faces.len() * cuts);
        for i in 0..ring.faces.len() {
            let j = (i + 1) % n;
            for (&a, &b) in cut_vertices[i].iter().zip(&cut_vertices[j]) {
                let face = self
                    .corners_at(a)
                    .map(|c| self.corners[c].face)
                    .find(|&f| self.face_walk(f).any(|c| self.corners[c].vertex == b))
                    .ok_or(MeshError::CannotLoopCut)?;
                let (_, made) = self.split_face(face, a, b)?;
                new_edges.push(made);
            }
        }

        self.deselect_all();
        for &made in &new_edges {
            self.edges[made].selected = true;
            for vertex in self.edges[made].vertices {
                self.vertices[vertex].selected = true;
            }
        }
        if mode != SelectMode::Face {
            self.flush_selection(mode);
        }
        Ok(new_edges)
    }

    /// The edges in `edges` as separate chains, each in order along itself, with
    /// whether it closes on itself. Refused if any vertex has more than two of them.
    fn chains(&self, edges: &[EdgeId]) -> Result<Vec<(Vec<VertexId>, bool)>, MeshError> {
        let mut at: HashMap<VertexId, Vec<EdgeId>> = HashMap::new();
        for &edge in edges {
            self.require_edge(edge)?;
            for vertex in self.edges[edge].vertices {
                at.entry(vertex).or_default().push(edge);
            }
        }
        if at.values().any(|list| list.len() > 2) {
            return Err(MeshError::NotLoops);
        }
        let mut used: HashSet<EdgeId> = HashSet::new();
        let mut chains = Vec::new();
        // Open chains start from an end; whatever is left over is closed.
        let mut starts: Vec<VertexId> = edges
            .iter()
            .flat_map(|&e| self.edges[e].vertices)
            .filter(|v| at[v].len() == 1)
            .collect();
        starts.extend(edges.iter().map(|&e| self.edges[e].vertices[0]));
        for start in starts {
            let Some(&first) = at[&start].iter().find(|e| !used.contains(*e)) else {
                continue;
            };
            let mut vertices = vec![start];
            let (mut edge, mut vertex) = (first, start);
            let closed = loop {
                used.insert(edge);
                vertex = self.edges[edge].other_vertex(vertex).unwrap_or(vertex);
                if vertex == start {
                    break true;
                }
                vertices.push(vertex);
                match at[&vertex].iter().find(|e| !used.contains(*e)) {
                    Some(&next) => edge = next,
                    None => break false,
                }
            };
            chains.push((vertices, closed));
        }
        Ok(chains)
    }

    /// For sliding `edges` along their neighbors: each vertex's start and its target on
    /// either side. Side 0 lies to the left of the chain's direction, where faces with
    /// the usual winding run along with it.
    pub fn edge_slide_rails(&self, edges: &[EdgeId]) -> Result<Vec<SlideRail>, MeshError> {
        if edges.is_empty() {
            return Err(MeshError::NothingSelected);
        }
        let chosen: HashSet<EdgeId> = edges.iter().copied().collect();
        let mut rails = Vec::new();
        for (vertices, closed) in self.chains(edges)? {
            let n = vertices.len();
            let steps = if closed { n } else { n - 1 };
            // For each step u -> w: the corner on each side.
            let mut sides: Vec<[Option<CornerId>; 2]> = Vec::with_capacity(steps);
            for i in 0..steps {
                let (u, w) = (vertices[i], vertices[(i + 1) % n]);
                let edge = self.find_edge(u, w).ok_or(MeshError::NotLoops)?;
                let mut side = [None, None];
                for corner in self.radial_walk(edge) {
                    let slot = usize::from(self.corners[corner].vertex != u);
                    side[slot].get_or_insert(corner);
                }
                sides.push(side);
            }
            for (i, &vertex) in vertices.iter().enumerate() {
                // The steps touching this vertex: the one arriving and the one leaving.
                let mut touching = Vec::with_capacity(2);
                if i < steps {
                    touching.push(i);
                }
                if i > 0 {
                    touching.push(i - 1);
                } else if closed {
                    touching.push(steps - 1);
                }
                let target = |slot: usize| {
                    touching.iter().find_map(|&step| {
                        let corner = sides[step][slot]?;
                        let rail = self.neighbor_in_face(corner, vertex);
                        if chosen.contains(&rail) {
                            return None;
                        }
                        let end = self.edges[rail].other_vertex(vertex)?;
                        Some(self.vertices[end].position)
                    })
                };
                rails.push((
                    vertex,
                    self.vertices[vertex].position,
                    [target(0), target(1)],
                ));
            }
        }
        if rails
            .iter()
            .all(|(_, _, [a, b])| a.is_none() && b.is_none())
        {
            return Err(MeshError::NothingToSlide);
        }
        Ok(rails)
    }

    /// Joins two edge loops with a band of four-sided faces. With faces selected, their
    /// outlines are bridged and the faces themselves removed, so two facing caps become
    /// a tube. The loops must be separate and have the same number of edges. The new
    /// faces become the selection.
    pub fn bridge_selected_loops(&mut self, mode: SelectMode) -> Result<(), MeshError> {
        let faces: Vec<FaceId> = self
            .faces()
            .filter(|(_, f)| f.selected)
            .map(|(id, _)| id)
            .collect();
        let (outline, inside): (Vec<EdgeId>, Vec<EdgeId>) = if faces.is_empty() {
            (self.selected_edges(), Vec::new())
        } else {
            let chosen: HashSet<FaceId> = faces.iter().copied().collect();
            let mut touched: HashSet<EdgeId> = HashSet::new();
            for &face in &faces {
                touched.extend(self.face_walk(face).map(|c| self.corners[c].edge));
            }
            touched.into_iter().partition(|&edge| {
                self.radial_walk(edge)
                    .filter(|&c| chosen.contains(&self.corners[c].face))
                    .count()
                    == 1
            })
        };
        let chains = self.chains(&outline)?;
        let [(first, closed), (second, other_closed)] =
            <[_; 2]>::try_from(chains).map_err(|_| MeshError::BridgeNeedsTwoLoops)?;
        if closed != other_closed
            || first.len() != second.len()
            || first.len() < 2
            || first.iter().any(|v| second.contains(v))
        {
            return Err(MeshError::BridgeNeedsTwoLoops);
        }

        // The selected faces go, with anything inside them left unused.
        let loop_vertices: HashSet<VertexId> = first.iter().chain(&second).copied().collect();
        for face in faces {
            self.remove_face(face)?;
        }
        for edge in inside {
            if self.edges[edge].first_corner.is_none() {
                let ends = self.edges[edge].vertices;
                self.remove_edge(edge)?;
                for vertex in ends {
                    if !loop_vertices.contains(&vertex)
                        && self.vertices[vertex].first_edge.is_none()
                    {
                        self.remove_vertex(vertex)?;
                    }
                }
            }
        }

        let (first, second) = self.line_up_loops(first, second, closed);
        let n = first.len();
        let steps = if closed { n } else { n - 1 };
        let mut new_faces = Vec::with_capacity(steps);
        for i in 0..steps {
            let j = (i + 1) % n;
            new_faces.push(self.add_face(&[first[i], first[j], second[j], second[i]])?);
        }
        let elements: Vec<Element> = new_faces.into_iter().map(Element::Face).collect();
        self.select_elements(SelectMode::Face, &elements, SelectOp::Replace)?;
        self.flush_selection(mode);
        Ok(())
    }

    /// How a run of a loop is used by faces already there: positive when they run
    /// along it in its order, negative when against it.
    fn winding_along(&self, vertices: &[VertexId], closed: bool) -> i32 {
        let n = vertices.len();
        let steps = if closed { n } else { n - 1 };
        let mut total = 0;
        for i in 0..steps {
            let (a, b) = (vertices[i], vertices[(i + 1) % n]);
            if let Some(edge) = self.find_edge(a, b) {
                for corner in self.radial_walk(edge) {
                    total += if self.corners[corner].vertex == a {
                        1
                    } else {
                        -1
                    };
                }
            }
        }
        total
    }

    /// Orders both loops so that the faces `first[i], first[i+1], second[i+1], second[i]`
    /// wind like their neighbors and do not twist.
    fn line_up_loops(
        &self,
        mut first: Vec<VertexId>,
        second: Vec<VertexId>,
        closed: bool,
    ) -> (Vec<VertexId>, Vec<VertexId>) {
        // The new faces run along the first loop, so neighbors must run against it.
        if self.winding_along(&first, closed) > 0 {
            first.reverse();
        }
        // ...and against the second loop, so neighbors must run along it.
        let votes = self.winding_along(&second, closed);
        let mut reversed = second.clone();
        reversed.reverse();
        let directions: Vec<&Vec<VertexId>> = match votes.signum() {
            1 => vec![&second],
            -1 => vec![&reversed],
            _ => vec![&second, &reversed],
        };
        let n = first.len();
        let points: Vec<V3> = first.iter().map(|&v| self.vertices[v].position).collect();
        let mut best: Option<(f32, Vec<VertexId>)> = None;
        for candidate in directions {
            let shifts = if closed { n } else { 1 };
            for shift in 0..shifts {
                let mut turned = candidate.clone();
                turned.rotate_left(shift);
                let cost: f32 = turned
                    .iter()
                    .zip(&points)
                    .map(|(&v, &p)| squared_distance(self.vertices[v].position, p))
                    .sum();
                if best.as_ref().is_none_or(|(lowest, _)| cost < *lowest) {
                    best = Some((cost, turned));
                }
            }
        }
        (first, best.map_or(second, |(_, order)| order))
    }
}
