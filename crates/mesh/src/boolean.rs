//! Union, subtract and intersect of closed parts.
//!
//! The method is the classic one of csg.js: each solid becomes a binary space
//! partitioning tree of its polygons, each tree clips away the other's polygons that
//! lie inside or outside it, and what is left is put together. It needs closed
//! surfaces, which Roblox needs anyway. Splitting leaves new points in the middle
//! of neighbors' edges, so afterwards those edges get the points too, which keeps
//! the result closed, and faces lying in one plane are joined again so the result
//! has few faces.
//!
//! The trees are stored in flat arrays and walked with explicit stacks, never by
//! recursion: a deep tree must not run out of stack on Windows, whose main thread
//! has only 1 MB.

use std::collections::{HashMap, HashSet};

use crate::error::MeshError;
use crate::id::{FaceId, VertexId};
use crate::mesh::Mesh;
use crate::selection::SelectMode;

type P = [f64; 3];
/// Numbered points, and faces as lists of their numbers.
type Built = (Vec<P>, Vec<Vec<usize>>);

/// Points closer than this to a plane count as on it.
const PLANE_EPS: f64 = 1e-5;
/// Points closer than this are the same point.
/// Points closer than this are the same point, when cleaning up. Welding tries the
/// distances in [`WELDS`] in turn.
const WELD: f64 = 5e-5;
/// Weld distances to try, until one gives a closed surface. Near-touching shapes
/// can leave a crack at one distance and not at another.
const WELDS: [f64; 4] = [5e-5, 2e-5, 1e-4, 1e-5];
/// More polygons than this and the operation stops, rather than take minutes.
const MAX_POLYGONS: usize = 400_000;

/// Which combination to make.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BooleanOp {
    /// Everything inside any of the parts.
    Union,
    /// The other parts, with the cutter part cut out of each.
    Subtract,
    /// Only what is inside all of the parts.
    Intersect,
}

fn sub(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: P, b: P) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: P, b: P) -> P {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn lerp(a: P, b: P, t: f64) -> P {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}
fn length(a: P) -> f64 {
    dot(a, a).sqrt()
}

/// Newell's normal: handles any polygon, and its length is twice the area.
fn newell(points: &[P]) -> P {
    let mut n = [0.0; 3];
    for (i, p) in points.iter().enumerate() {
        let q = points[(i + 1) % points.len()];
        n[0] += (p[1] - q[1]) * (p[2] + q[2]);
        n[1] += (p[2] - q[2]) * (p[0] + q[0]);
        n[2] += (p[0] - q[0]) * (p[1] + q[1]);
    }
    n
}

#[derive(Debug, Clone, Copy)]
struct Plane {
    normal: P,
    w: f64,
}

impl Plane {
    fn of(points: &[P]) -> Option<Plane> {
        let n = newell(points);
        let len = length(n);
        if len.is_nan() || len <= 1e-12 {
            return None;
        }
        let normal = [n[0] / len, n[1] / len, n[2] / len];
        let count = points.len() as f64;
        let center = points.iter().fold([0.0; 3], |s, p| {
            [
                s[0] + p[0] / count,
                s[1] + p[1] / count,
                s[2] + p[2] / count,
            ]
        });
        Some(Plane {
            normal,
            w: dot(normal, center),
        })
    }

    fn flipped(self) -> Plane {
        Plane {
            normal: self.normal.map(|c| -c),
            w: -self.w,
        }
    }
}

#[derive(Debug, Clone)]
struct Polygon {
    points: Vec<P>,
    plane: Plane,
}

impl Polygon {
    fn new(points: Vec<P>) -> Option<Polygon> {
        let plane = Plane::of(&points)?;
        Some(Polygon { points, plane })
    }

    fn flip(&mut self) {
        self.points.reverse();
        self.plane = self.plane.flipped();
    }
}

const COPLANAR: u8 = 0;
const FRONT: u8 = 1;
const BACK: u8 = 2;
const SPANNING: u8 = 3;

/// Sorts a polygon by a plane, splitting it if it crosses.
fn split(
    plane: Plane,
    polygon: Polygon,
    coplanar_front: &mut Vec<Polygon>,
    coplanar_back: &mut Vec<Polygon>,
    front: &mut Vec<Polygon>,
    back: &mut Vec<Polygon>,
) {
    let kinds: Vec<u8> = polygon
        .points
        .iter()
        .map(|&p| {
            let t = dot(plane.normal, p) - plane.w;
            if t < -PLANE_EPS {
                BACK
            } else if t > PLANE_EPS {
                FRONT
            } else {
                COPLANAR
            }
        })
        .collect();
    let kind = kinds.iter().fold(0, |a, &k| a | k);
    match kind {
        COPLANAR => {
            if dot(plane.normal, polygon.plane.normal) > 0.0 {
                coplanar_front.push(polygon);
            } else {
                coplanar_back.push(polygon);
            }
        }
        FRONT => front.push(polygon),
        BACK => back.push(polygon),
        _ => {
            let n = polygon.points.len();
            let mut f = Vec::with_capacity(n + 1);
            let mut b = Vec::with_capacity(n + 1);
            for i in 0..n {
                let j = (i + 1) % n;
                let (ki, kj) = (kinds[i], kinds[j]);
                let (vi, vj) = (polygon.points[i], polygon.points[j]);
                if ki != BACK {
                    f.push(vi);
                }
                if ki != FRONT {
                    b.push(vi);
                }
                if ki | kj == SPANNING {
                    let t = (plane.w - dot(plane.normal, vi)) / dot(plane.normal, sub(vj, vi));
                    let v = lerp(vi, vj, t);
                    f.push(v);
                    b.push(v);
                }
            }
            if f.len() >= 3 {
                front.push(Polygon {
                    points: f,
                    plane: polygon.plane,
                });
            }
            if b.len() >= 3 {
                back.push(Polygon {
                    points: b,
                    plane: polygon.plane,
                });
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
struct Node {
    plane: Option<Plane>,
    front: Option<usize>,
    back: Option<usize>,
    polygons: Vec<Polygon>,
}

/// A solid's BSP tree, in a flat array. Node 0 is the root.
#[derive(Debug, Clone)]
struct Tree {
    nodes: Vec<Node>,
}

impl Tree {
    fn new(polygons: Vec<Polygon>) -> Result<Tree, MeshError> {
        let mut tree = Tree {
            nodes: vec![Node::default()],
        };
        tree.build(polygons)?;
        Ok(tree)
    }

    fn polygon_count(&self) -> usize {
        self.nodes.iter().map(|n| n.polygons.len()).sum()
    }

    /// Adds polygons to the tree, making new nodes where they are needed.
    fn build(&mut self, polygons: Vec<Polygon>) -> Result<(), MeshError> {
        if polygons.is_empty() {
            return Ok(());
        }
        let mut stack = vec![(0usize, polygons)];
        let mut total = self.polygon_count();
        while let Some((index, polygons)) = stack.pop() {
            let plane = match self.nodes[index].plane {
                Some(plane) => plane,
                None => {
                    let plane = polygons[0].plane;
                    self.nodes[index].plane = Some(plane);
                    plane
                }
            };
            let (mut front, mut back) = (Vec::new(), Vec::new());
            let (mut cf, mut cb) = (Vec::new(), Vec::new());
            for polygon in polygons {
                split(plane, polygon, &mut cf, &mut cb, &mut front, &mut back);
            }
            total += cf.len() + cb.len();
            if total > MAX_POLYGONS {
                return Err(MeshError::TooComplex);
            }
            self.nodes[index].polygons.extend(cf);
            self.nodes[index].polygons.extend(cb);
            for (list, is_front) in [(front, true), (back, false)] {
                if list.is_empty() {
                    continue;
                }
                let child = if is_front {
                    self.nodes[index].front
                } else {
                    self.nodes[index].back
                };
                let child = match child {
                    Some(child) => child,
                    None => {
                        self.nodes.push(Node::default());
                        let child = self.nodes.len() - 1;
                        if is_front {
                            self.nodes[index].front = Some(child);
                        } else {
                            self.nodes[index].back = Some(child);
                        }
                        child
                    }
                };
                stack.push((child, list));
            }
        }
        Ok(())
    }

    /// Turns the solid inside out.
    fn invert(&mut self) {
        for node in &mut self.nodes {
            for polygon in &mut node.polygons {
                polygon.flip();
            }
            node.plane = node.plane.map(Plane::flipped);
            std::mem::swap(&mut node.front, &mut node.back);
        }
    }

    /// Removes the parts of these polygons that are inside this solid.
    fn clip_polygons(&self, polygons: Vec<Polygon>) -> Result<Vec<Polygon>, MeshError> {
        let mut kept = Vec::new();
        let mut stack = vec![(0usize, polygons)];
        while let Some((index, polygons)) = stack.pop() {
            let node = &self.nodes[index];
            let Some(plane) = node.plane else {
                kept.extend(polygons);
                continue;
            };
            let (mut front, mut back) = (Vec::new(), Vec::new());
            let (mut cf, mut cb) = (Vec::new(), Vec::new());
            for polygon in polygons {
                split(plane, polygon, &mut cf, &mut cb, &mut front, &mut back);
            }
            front.extend(cf);
            back.extend(cb);
            match node.front {
                Some(child) if !front.is_empty() => stack.push((child, front)),
                _ => kept.extend(front),
            }
            // Behind a leaf is inside the solid, so those pieces are dropped.
            if let Some(child) = node.back
                && !back.is_empty()
            {
                stack.push((child, back));
            }
            if kept.len() > MAX_POLYGONS {
                return Err(MeshError::TooComplex);
            }
        }
        Ok(kept)
    }

    /// Removes the parts of this tree's polygons that are inside `other`.
    fn clip_to(&mut self, other: &Tree) -> Result<(), MeshError> {
        for i in 0..self.nodes.len() {
            let polygons = std::mem::take(&mut self.nodes[i].polygons);
            self.nodes[i].polygons = other.clip_polygons(polygons)?;
        }
        Ok(())
    }

    fn all_polygons(&self) -> Vec<Polygon> {
        self.nodes
            .iter()
            .flat_map(|n| n.polygons.iter().cloned())
            .collect()
    }
}

fn union(a: Vec<Polygon>, b: Vec<Polygon>) -> Result<Vec<Polygon>, MeshError> {
    let mut a = Tree::new(a)?;
    let mut b = Tree::new(b)?;
    a.clip_to(&b)?;
    b.clip_to(&a)?;
    b.invert();
    b.clip_to(&a)?;
    b.invert();
    a.build(b.all_polygons())?;
    Ok(a.all_polygons())
}

fn subtract(a: Vec<Polygon>, b: Vec<Polygon>) -> Result<Vec<Polygon>, MeshError> {
    let mut a = Tree::new(a)?;
    let mut b = Tree::new(b)?;
    a.invert();
    a.clip_to(&b)?;
    b.clip_to(&a)?;
    b.invert();
    b.clip_to(&a)?;
    b.invert();
    a.build(b.all_polygons())?;
    a.invert();
    Ok(a.all_polygons())
}

fn intersect(a: Vec<Polygon>, b: Vec<Polygon>) -> Result<Vec<Polygon>, MeshError> {
    let mut a = Tree::new(a)?;
    let mut b = Tree::new(b)?;
    a.invert();
    b.clip_to(&a)?;
    b.invert();
    a.clip_to(&b)?;
    b.clip_to(&a)?;
    a.build(b.all_polygons())?;
    a.invert();
    Ok(a.all_polygons())
}

/// Gives every point a number, with points closer than `tol` sharing one. Closeness
/// is followed through chains, so if A is near B and B near C, all three are one
/// point even when A and C are a little further apart. Otherwise two neighbors
/// could each round a shared corner to a different point and leave a crack.
fn weld_points(all: &[P], tol: f64) -> (Vec<P>, Vec<usize>) {
    let mut parent: Vec<usize> = (0..all.len()).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    let cell = |p: P| p.map(|c| (c / tol).floor() as i64);
    let mut cells: HashMap<[i64; 3], Vec<usize>> = HashMap::new();
    for (i, &p) in all.iter().enumerate() {
        let c = cell(p);
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    if let Some(list) = cells.get(&[c[0] + dx, c[1] + dy, c[2] + dz]) {
                        for &j in list {
                            if length(sub(all[j], p)) < tol {
                                let (ri, rj) = (root(&mut parent, i), root(&mut parent, j));
                                parent[ri] = rj;
                            }
                        }
                    }
                }
            }
        }
        cells.entry(c).or_default().push(i);
    }
    let mut number: HashMap<usize, usize> = HashMap::new();
    let mut points = Vec::new();
    let mut index = Vec::with_capacity(all.len());
    for i in 0..all.len() {
        let r = root(&mut parent, i);
        let n = *number.entry(r).or_insert_with(|| {
            points.push(all[r]);
            points.len() - 1
        });
        index.push(n);
    }
    (points, index)
}

/// Turns polygons into faces of shared, numbered points, with every point that lies
/// on an edge added to that edge, so that neighbors meet exactly.
fn weld(polygons: &[Polygon]) -> (Vec<P>, Vec<Vec<usize>>) {
    let mut best: Option<(usize, Built)> = None;
    for tol in WELDS {
        let made = weld_with(polygons, tol);
        let open = open_edge_count(&made.1);
        if open == 0 {
            return made;
        }
        if best.as_ref().is_none_or(|(fewest, _)| open < *fewest) {
            best = Some((open, made));
        }
    }
    best.map(|(_, made)| made).unwrap_or_default()
}

/// How many edges of these faces are not matched by a neighbor running the other way.
fn open_edge_count(faces: &[Vec<usize>]) -> usize {
    let mut directed: HashMap<(usize, usize), i32> = HashMap::new();
    for face in faces {
        for k in 0..face.len() {
            let (a, b) = (face[k], face[(k + 1) % face.len()]);
            *directed.entry((a, b)).or_default() += 1;
        }
    }
    directed
        .iter()
        .filter(|&(&(a, b), &n)| directed.get(&(b, a)).copied().unwrap_or(0) != n)
        .count()
}

fn weld_with(polygons: &[Polygon], tol: f64) -> (Vec<P>, Vec<Vec<usize>>) {
    let all: Vec<P> = polygons
        .iter()
        .flat_map(|p| p.points.iter().copied())
        .collect();
    let (points, index) = weld_points(&all, tol);
    let mut faces: Vec<Vec<usize>> = Vec::new();
    let mut at = 0;
    for polygon in polygons {
        let count = polygon.points.len();
        faces.extend(split_repeats(index[at..at + count].to_vec()));
        at += count;
    }
    let faces: Vec<Vec<usize>> = faces
        .into_iter()
        .filter(|face| {
            let corners: Vec<P> = face.iter().map(|&i| points[i]).collect();
            length(newell(&corners)) > 1e-9
        })
        .collect();

    // Points sorted along X, to find the ones near an edge quickly.

    let mut by_x: Vec<usize> = (0..points.len()).collect();
    by_x.sort_by(|&a, &b| points[a][0].total_cmp(&points[b][0]));
    let xs: Vec<f64> = by_x.iter().map(|&i| points[i][0]).collect();
    let on_edge = |a: usize, b: usize| -> Vec<usize> {
        let (pa, pb) = (points[a], points[b]);
        let d = sub(pb, pa);
        let len2 = dot(d, d);
        if len2 < tol * tol {
            return Vec::new();
        }
        let lo = pa[0].min(pb[0]) - tol;
        let hi = pa[0].max(pb[0]) + tol;
        let start = xs.partition_point(|&x| x < lo);
        let mut found: Vec<(f64, usize)> = Vec::new();
        for &i in &by_x[start..] {
            let p = points[i];
            if p[0] > hi {
                break;
            }
            if i == a || i == b {
                continue;
            }
            let t = dot(sub(p, pa), d) / len2;
            if t <= 0.0 || t >= 1.0 {
                continue;
            }
            if length(sub(p, lerp(pa, pb, t))) < tol {
                found.push((t, i));
            }
        }
        found.sort_by(|x, y| x.0.total_cmp(&y.0));
        found.into_iter().map(|(_, i)| i).collect()
    };
    let faces = faces
        .into_iter()
        .map(|face| {
            let mut out = Vec::with_capacity(face.len());
            for k in 0..face.len() {
                let (a, b) = (face[k], face[(k + 1) % face.len()]);
                out.push(a);
                out.extend(on_edge(a, b));
            }
            out
        })
        .flat_map(split_repeats)
        .collect();
    (points, faces)
}

/// Removes repeated points from a face. Neighbors that are the same are merged, and
/// a point that comes back later splits the face into two loops there. Loops of
/// fewer than three points are slivers with no area, and are dropped: their two
/// sides lie on one edge, so the faces beside them still meet.
fn split_repeats(face: Vec<usize>) -> Vec<Vec<usize>> {
    let mut out = Vec::new();
    let mut stack = vec![face];
    while let Some(mut face) = stack.pop() {
        face.dedup();
        while face.len() > 1 && face.first() == face.last() {
            face.pop();
        }
        let repeat = (0..face.len()).find_map(|i| {
            (i + 1..face.len())
                .find(|&j| face[j] == face[i])
                .map(|j| (i, j))
        });
        match repeat {
            Some((i, j)) => {
                let inner = face[i..j].to_vec();
                let mut outer = face[j..].to_vec();
                outer.extend_from_slice(&face[..i]);
                stack.push(inner);
                stack.push(outer);
            }
            None if face.len() >= 3 => out.push(face),
            None => {}
        }
    }
    out
}

impl Mesh {
    fn point(&self, v: VertexId) -> P {
        self.vertices[v].position.map(f64::from)
    }

    /// A part's faces as polygons for the trees: convex flat faces whole, others as
    /// triangles. Refuses parts that are not closed, and turns inside-out parts the
    /// right way.
    fn solid(&self, faces: &[FaceId]) -> Result<Vec<Polygon>, MeshError> {
        let set: HashSet<FaceId> = faces.iter().copied().collect();
        let mut edges = HashSet::new();
        for &face in faces {
            for corner in self.face_walk(face) {
                edges.insert(self.corners[corner].edge);
            }
        }
        for edge in edges {
            let corners: Vec<_> = self.radial_walk(edge).collect();
            let [a, b] = corners.as_slice() else {
                return Err(MeshError::NotClosed);
            };
            let (a, b) = (self.corners[*a], self.corners[*b]);
            if a.vertex == b.vertex || !set.contains(&a.face) || !set.contains(&b.face) {
                return Err(MeshError::NotClosed);
            }
        }

        let mut polygons = Vec::new();
        let mut volume = 0.0;
        for &face in faces {
            let points: Vec<P> = self
                .face_walk(face)
                .map(|c| self.point(self.corners[c].vertex))
                .collect();
            for t in self.face_triangles(face)? {
                let [a, b, c] = t.map(|v| self.point(v));
                volume += dot(a, cross(b, c)) / 6.0;
            }
            match Polygon::new(points.clone()) {
                Some(polygon) if is_flat_and_convex(&polygon) => polygons.push(polygon),
                _ => {
                    for t in self.face_triangles(face)? {
                        if let Some(polygon) = Polygon::new(t.map(|v| self.point(v)).to_vec()) {
                            polygons.push(polygon);
                        }
                    }
                }
            }
        }
        if volume < 0.0 {
            for polygon in &mut polygons {
                polygon.flip();
            }
        }
        Ok(polygons)
    }

    /// Combines the selected parts. For [`BooleanOp::Subtract`], `cutter` is a face of
    /// the part that is cut out of the others; the others stay separate parts. Union
    /// and intersect make one part. The parts must be closed. Returns how many faces
    /// the result has.
    pub fn boolean_selected(
        &mut self,
        op: BooleanOp,
        cutter: Option<FaceId>,
        mode: SelectMode,
    ) -> Result<usize, MeshError> {
        let parts: Vec<Vec<FaceId>> = self
            .parts()
            .into_iter()
            .filter(|part| part.iter().any(|&f| self.faces[f].selected))
            .collect();
        if parts.len() < 2 {
            return Err(MeshError::NeedsTwoParts);
        }
        let mut solids = Vec::with_capacity(parts.len());
        for part in &parts {
            solids.push(self.solid(part)?);
        }

        // Each result is one set of polygons that becomes one part.
        let results: Vec<Vec<Polygon>> = match op {
            BooleanOp::Union | BooleanOp::Intersect => {
                let mut solids = solids.into_iter();
                let mut result = solids.next().unwrap_or_default();
                for next in solids {
                    result = if op == BooleanOp::Union {
                        union(result, next)?
                    } else {
                        intersect(result, next)?
                    };
                }
                vec![result]
            }
            BooleanOp::Subtract => {
                let cutter = cutter
                    .and_then(|f| parts.iter().position(|part| part.contains(&f)))
                    .ok_or(MeshError::NoCutter)?;
                let tool = solids[cutter].clone();
                let mut results = Vec::new();
                for (i, solid) in solids.into_iter().enumerate() {
                    if i != cutter {
                        results.push(subtract(solid, tool.clone())?);
                    }
                }
                results
            }
        };
        let built: Vec<(Vec<P>, Vec<Vec<usize>>)> = results.iter().map(|r| weld(r)).collect();
        if built.iter().all(|(_, faces)| faces.is_empty()) {
            return Err(MeshError::EmptyResult);
        }

        // Try it on a copy first, so that a failure leaves this mesh untouched.
        let mut trial = self.clone();
        trial.replace_parts(&parts, &built, mode)?;
        self.replace_parts(&parts, &built, mode)
    }

    fn replace_parts(
        &mut self,
        parts: &[Vec<FaceId>],
        built: &[(Vec<P>, Vec<Vec<usize>>)],
        mode: SelectMode,
    ) -> Result<usize, MeshError> {
        let mut doomed: HashSet<VertexId> = HashSet::new();
        for &face in parts.iter().flatten() {
            for corner in self.face_walk(face) {
                doomed.insert(self.corners[corner].vertex);
            }
        }
        for vertex in doomed {
            if self.vertices.contains(vertex) {
                self.remove_vertex(vertex)?;
            }
        }
        self.deselect_all();
        let mut next_group = self.next_group()?;
        let mut total = 0;
        for (points, faces) in built {
            let mut ids: HashMap<usize, VertexId> = HashMap::new();
            let mut made: Vec<FaceId> = Vec::new();
            for face in faces {
                let mut cycle = Vec::with_capacity(face.len());
                for &i in face {
                    let id = match ids.get(&i) {
                        Some(&id) => id,
                        None => {
                            let p = points[i];
                            let id = self.add_vertex([p[0] as f32, p[1] as f32, p[2] as f32])?;
                            ids.insert(i, id);
                            id
                        }
                    };
                    cycle.push(id);
                }
                match self.add_face(&cycle) {
                    Ok(face) => made.push(face),
                    Err(MeshError::FaceExists(_) | MeshError::RepeatedVertex(_)) => {}
                    Err(error) => return Err(error),
                }
            }
            let made = self.join_flat_neighbors(made);
            // Pieces of one result stay one part.
            if self.count_pieces(&made) > 1 {
                for &face in &made {
                    self.faces[face].group = next_group;
                }
                next_group = next_group.checked_add(1).ok_or(MeshError::IdsExhausted)?;
            }
            for &face in &made {
                self.faces[face].selected = true;
            }
            total += made.len();
        }
        self.flush_selection(SelectMode::Face);
        self.flush_selection(mode);
        Ok(total)
    }

    /// Joins neighboring faces that lie in one plane, which the trees cut apart.
    /// Returns the faces left.
    fn join_flat_neighbors(&mut self, faces: Vec<FaceId>) -> Vec<FaceId> {
        let mut alive: HashSet<FaceId> = faces.into_iter().collect();
        // Removing leftover points lets more faces join, so take turns until neither
        // finds anything more to do.
        for _ in 0..8 {
            self.join_pass(&mut alive);
            if !self.dissolve_straight_points(&mut alive) {
                break;
            }
        }
        alive.into_iter().collect()
    }

    fn join_pass(&mut self, alive: &mut HashSet<FaceId>) {
        loop {
            let mut joined = false;
            let edges: Vec<_> = {
                let mut edges = HashSet::new();
                for &face in alive.iter() {
                    for corner in self.face_walk(face) {
                        edges.insert(self.corners[corner].edge);
                    }
                }
                edges.into_iter().collect()
            };
            for edge in edges {
                if !self.edges.contains(edge) {
                    continue;
                }
                let corners: Vec<_> = self.radial_walk(edge).collect();
                let [a, b] = corners.as_slice() else {
                    continue;
                };
                let (fa, fb) = (self.corners[*a].face, self.corners[*b].face);
                if fa == fb || !alive.contains(&fa) || !alive.contains(&fb) {
                    continue;
                }
                let (Ok(Some(na)), Ok(Some(nb))) = (self.face_normal(fa), self.face_normal(fb))
                else {
                    continue;
                };
                let na = na.map(f64::from);
                let nb = nb.map(f64::from);
                if dot(na, nb) < 1.0 - 1e-6 {
                    continue;
                }
                // The same plane, not just parallel.
                let pa = self.point(self.corners[self.faces[fa].first_corner].vertex);
                let pb = self.point(self.corners[self.faces[fb].first_corner].vertex);
                if dot(na, sub(pb, pa)).abs() > PLANE_EPS * 10.0 {
                    continue;
                }
                if let Ok(kept) = self.join_faces(edge) {
                    let removed = if kept == fa { fb } else { fa };
                    alive.remove(&removed);
                    joined = true;
                } else if let Some(merged) = self.merge_along_chain(fa, fb) {
                    alive.remove(&fa);
                    alive.remove(&fb);
                    alive.insert(merged);
                    joined = true;
                }
            }
            if !joined {
                break;
            }
        }
    }

    /// Joins two faces in one plane that share several edges in a row, which
    /// `join_faces` cannot do. The points in the middle of the shared run belong to
    /// nothing else, so they go too. Returns the new face, or `None` if the two do not
    /// join into one simple outline.
    fn merge_along_chain(&mut self, fa: FaceId, fb: FaceId) -> Option<FaceId> {
        let cycle = |mesh: &Mesh, f: FaceId| -> Vec<VertexId> {
            mesh.face_walk(f).map(|c| mesh.corners[c].vertex).collect()
        };
        let (ca, cb) = (cycle(self, fa), cycle(self, fb));
        let directed = |c: &[VertexId]| -> Vec<(VertexId, VertexId)> {
            (0..c.len()).map(|i| (c[i], c[(i + 1) % c.len()])).collect()
        };
        let (da, db) = (directed(&ca), directed(&cb));
        let shared: HashSet<(VertexId, VertexId)> = da
            .iter()
            .copied()
            .filter(|&(x, y)| db.contains(&(y, x)))
            .collect();
        if shared.is_empty() {
            return None;
        }
        let kept: Vec<(VertexId, VertexId)> = da
            .iter()
            .copied()
            .filter(|e| !shared.contains(e))
            .chain(
                db.iter()
                    .copied()
                    .filter(|&(x, y)| !shared.contains(&(y, x))),
            )
            .collect();
        // Walk the edges left into one loop.
        let mut next: HashMap<VertexId, VertexId> = HashMap::new();
        for &(x, y) in &kept {
            if next.insert(x, y).is_some() {
                return None;
            }
        }
        let start = kept.first()?.0;
        let mut outline = vec![start];
        let mut at = start;
        loop {
            at = *next.get(&at)?;
            if at == start {
                break;
            }
            if outline.len() > kept.len() {
                return None;
            }
            outline.push(at);
        }
        if outline.len() != kept.len() || outline.len() < 3 {
            return None;
        }
        // Points only on the shared run, touched by nothing but these two faces.
        let on_outline: HashSet<VertexId> = outline.iter().copied().collect();
        let inner: Vec<VertexId> = ca
            .iter()
            .copied()
            .filter(|v| !on_outline.contains(v))
            .collect();
        for &v in &inner {
            let faces: HashSet<FaceId> = self.corners_at(v).map(|c| self.corners[c].face).collect();
            let edges = self.disk_walk(v).count();
            if faces.len() != 2 || edges != 2 {
                return None;
            }
        }
        if self.find_face(&outline).is_some() {
            return None;
        }
        let (selected, group) = (self.faces[fa].selected, self.faces[fa].group);
        self.remove_face(fa).ok()?;
        self.remove_face(fb).ok()?;
        for v in inner {
            self.remove_vertex(v).ok()?;
        }
        for (x, y) in shared {
            if !self.vertices.contains(x) || !self.vertices.contains(y) {
                continue;
            }
            if let Some(e) = self.find_edge(x, y)
                && self.edges[e].first_corner.is_none()
                && self.remove_edge(e).is_err()
            {
                return None;
            }
        }
        let merged = self.add_face(&outline).ok()?;
        self.faces[merged].selected = selected;
        self.faces[merged].group = group;
        Some(merged)
    }

    /// Removes points that have only two edges in a straight line, left over from
    /// splitting, by rebuilding the faces around them without them.
    fn dissolve_straight_points(&mut self, faces: &mut HashSet<FaceId>) -> bool {
        let mut changed = false;
        let mut candidates: Vec<VertexId> = Vec::new();
        for &face in faces.iter() {
            if !self.faces.contains(face) {
                continue;
            }
            for corner in self.face_walk(face) {
                candidates.push(self.corners[corner].vertex);
            }
        }
        candidates.sort_unstable();
        candidates.dedup();
        for v in candidates {
            if !self.vertices.contains(v) {
                continue;
            }
            let edges: Vec<_> = self.disk_walk(v).collect();
            let [e1, e2] = edges.as_slice() else {
                continue;
            };
            let (Some(a), Some(b)) = (
                self.edges[*e1].other_vertex(v),
                self.edges[*e2].other_vertex(v),
            ) else {
                continue;
            };
            let (pa, pv, pb) = (self.point(a), self.point(v), self.point(b));
            let d = sub(pb, pa);
            let len2 = dot(d, d);
            if len2 < WELD * WELD {
                continue;
            }
            let t = dot(sub(pv, pa), d) / len2;
            if length(sub(pv, lerp(pa, pb, t))) > WELD {
                continue;
            }
            if self.find_edge(a, b).is_some() {
                continue;
            }
            let around: Vec<FaceId> = self.corners_at(v).map(|c| self.corners[c].face).collect();
            let mut cycles = Vec::new();
            let mut ok = true;
            for &face in &around {
                let cycle: Vec<VertexId> = self
                    .face_walk(face)
                    .map(|c| self.corners[c].vertex)
                    .filter(|&x| x != v)
                    .collect();
                if cycle.len() < 3 {
                    ok = false;
                }
                cycles.push((cycle, self.faces[face].selected, self.faces[face].group));
            }
            if !ok {
                continue;
            }
            // Rebuilt faces must not repeat each other or anything else.
            let mut keys = HashSet::new();
            for (cycle, _, _) in &cycles {
                let mut key = cycle.clone();
                key.sort_unstable();
                if !keys.insert(key) || self.find_face(cycle).is_some() {
                    ok = false;
                }
            }
            if !ok || self.remove_vertex(v).is_err() {
                continue;
            }
            changed = true;
            for face in &around {
                faces.remove(face);
            }
            for (cycle, selected, group) in cycles {
                if let Ok(face) = self.add_face(&cycle) {
                    self.faces[face].selected = selected;
                    self.faces[face].group = group;
                    faces.insert(face);
                }
            }
        }
        changed
    }

    /// How many connected pieces these faces make.
    fn count_pieces(&self, faces: &[FaceId]) -> usize {
        let mut seen: HashSet<VertexId> = HashSet::new();
        let set: HashSet<FaceId> = faces.iter().copied().collect();
        let mut pieces = 0;
        for &face in faces {
            if !self.faces.contains(face) {
                continue;
            }
            let start = self.corners[self.faces[face].first_corner].vertex;
            if !seen.insert(start) {
                continue;
            }
            pieces += 1;
            let mut stack = vec![start];
            while let Some(v) = stack.pop() {
                for c in self.corners_at(v).collect::<Vec<_>>() {
                    if !set.contains(&self.corners[c].face) {
                        continue;
                    }
                    for c2 in self.face_walk(self.corners[c].face) {
                        let w = self.corners[c2].vertex;
                        if seen.insert(w) {
                            stack.push(w);
                        }
                    }
                }
            }
        }
        pieces
    }
}

/// True if a polygon is flat and bulges outward at every corner.
fn is_flat_and_convex(polygon: &Polygon) -> bool {
    let n = polygon.plane.normal;
    let points = &polygon.points;
    let count = points.len();
    if points
        .iter()
        .any(|&p| (dot(n, p) - polygon.plane.w).abs() > PLANE_EPS)
    {
        return false;
    }
    (0..count).all(|i| {
        let a = points[i];
        let b = points[(i + 1) % count];
        let c = points[(i + 2) % count];
        dot(cross(sub(b, a), sub(c, b)), n) > -1e-12
    })
}
