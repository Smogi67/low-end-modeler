//! Edge bevel: cuts selected edges off with a flat or rounded band of faces.
//!
//! # How it works
//!
//! Around each vertex touched by a beveled edge, the faces form a *fan*: the faces
//! in order around the vertex, each between two of its edges. The vertex is replaced
//! by new points:
//!
//! - In a face between two beveled edges, a *corner point* inside the face, at the
//!   bevel width from both edges.
//! - On an unbeveled edge next to a beveled one, a point *on that edge*, as far along
//!   it as needed to lie at the bevel width from the beveled edge.
//! - Unbeveled edges with no beveled neighbor stay on the original vertex, which is
//!   then kept.
//!
//! Each beveled edge becomes a band of `segments` faces between the points on its two
//! sides. With more than one segment the band follows a round profile: a quarter
//! circle on a cube edge. The profile at a vertex runs between the band's two side
//! points, curving toward the original vertex, so neighboring bands share it.
//!
//! Where three or more points surround a vertex, a corner face fills the gap, as on a
//! cube corner with all three edges beveled. With more than one segment it becomes a
//! fan of triangles around a raised center. Where only two points surround it, the
//! bands meet directly, as where a beveled loop of edges turns a corner.
//!
//! Every new point lies at its vertex plus the width times a fixed direction, so a
//! drag can change the width by moving points, without rebuilding anything.
//!
//! As in `tools.rs`, run this inside a `Document` edit.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};

use crate::error::MeshError;
use crate::id::{CornerId, EdgeId, FaceId, VertexId};
use crate::mesh::Mesh;
use crate::selection::{Element, SelectMode, SelectOp};

type V3 = [f32; 3];

/// The most segments one bevel can have.
pub const MAX_BEVEL_SEGMENTS: usize = 16;

/// How far a raised center sits from its vertex, as a share of the way to the middle
/// of the corner's points. This puts it on the sphere of a rounded cube corner.
const CAP_CENTER_SHARE: f32 = 0.634;

/// What a bevel made, for adjusting its width afterwards.
#[derive(Debug, Clone, PartialEq)]
pub struct Bevel {
    /// Each new vertex, the original vertex position it grew from, and how far it
    /// moves from there per unit of width.
    pub moving: Vec<(VertexId, V3, V3)>,
    /// The width used, after limiting it to `max_width`.
    pub width: f32,
    /// The widest bevel that keeps the new points from passing each other.
    pub max_width: f32,
}

impl Bevel {
    /// Where each new vertex goes for this width, limited to `max_width`.
    pub fn positions(&self, width: f32) -> impl Iterator<Item = (VertexId, V3)> + '_ {
        let width = width.clamp(0.0, self.max_width);
        self.moving
            .iter()
            .map(move |&(id, origin, step)| (id, add(origin, scale(step, width))))
    }
}

fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn scale(a: V3, s: f32) -> V3 {
    a.map(|c| c * s)
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
fn length(a: V3) -> f32 {
    dot(a, a).sqrt()
}
fn unit(a: V3) -> Option<V3> {
    let l = length(a);
    (l > 1e-9).then(|| scale(a, 1.0 / l))
}

/// A new point around a vertex.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Spot {
    /// Inside the fan's face with this index, between two beveled edges.
    Corner(VertexId, usize),
    /// On this unbeveled edge.
    OnEdge(VertexId, EdgeId),
    /// The original vertex, kept.
    Keep(VertexId),
}

/// The faces around a vertex, in order. Face `i` lies between `edges[i]`, where it
/// arrives at the vertex, and the next edge, where it leaves.
struct Fan {
    corners: Vec<CornerId>,
    edges: Vec<EdgeId>,
    closed: bool,
    /// For face `i`: the inward directions across its arriving and leaving edges, and
    /// the directions along them away from the vertex.
    across: Vec<[V3; 2]>,
    along: Vec<[V3; 2]>,
}

impl Fan {
    fn next_edge(&self, face: usize) -> EdgeId {
        self.edges[(face + 1) % self.edges.len()]
    }
}

/// The ring of points and beveled edges around a vertex, in fan order.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Around {
    Point(Spot),
    Beveled(EdgeId),
}

/// A polygon's normal, scaled by its area.
fn newell(points: &[V3]) -> V3 {
    let mut n = [0.0; 3];
    for (i, p) in points.iter().enumerate() {
        let q = points[(i + 1) % points.len()];
        n[0] += (p[1] - q[1]) * (p[2] + q[2]);
        n[1] += (p[2] - q[2]) * (p[0] + q[0]);
        n[2] += (p[0] - q[0]) * (p[1] + q[1]);
    }
    n
}

/// The step of any point made so far: a corner or edge point, or a profile's.
fn step_of_point(
    points: &HashMap<VertexId, V3>,
    inner: &HashMap<VertexId, V3>,
    vertex: VertexId,
) -> V3 {
    points
        .get(&vertex)
        .or_else(|| inner.get(&vertex))
        .copied()
        .unwrap_or([0.0; 3])
}

impl Mesh {
    /// Where every edge at a vertex is beveled, its rounded corner is part of a sphere:
    /// the one touching each face at that face's corner point. Returns the sphere's
    /// center and radius, per unit of width and measured from the vertex.
    fn corner_sphere(
        &self,
        v: VertexId,
        fan: &Fan,
        ring: &[Around],
        steps: &HashMap<Spot, V3>,
        made: &HashMap<Spot, VertexId>,
        step_of: &HashMap<VertexId, V3>,
    ) -> Option<(V3, f32)> {
        let all_beveled = fan.closed
            && ring
                .iter()
                .all(|r| matches!(r, Around::Beveled(_) | Around::Point(Spot::Corner(..))));
        if !all_beveled {
            return None;
        }
        // Each face's corner point, moved one width back into the shape.
        let mut sum = [0.0; 3];
        let mut count = 0.0;
        for (i, &c) in fan.corners.iter().enumerate() {
            let face = self.corners[c].face;
            let (Some(&step), Ok(Some(normal))) =
                (steps.get(&Spot::Corner(v, i)), self.face_normal(face))
            else {
                return None;
            };
            sum = add(sum, sub(step, normal));
            count += 1.0;
        }
        let middle = scale(sum, 1.0 / count);
        let mut radius = 0.0;
        for r in ring {
            if let Around::Point(spot) = r {
                radius += length(sub(step_of[&made[spot]], middle)) / count;
            }
        }
        (radius > 1e-6).then_some((middle, radius))
    }

    /// The faces around `vertex` in order, or `None` if they do not form one simple fan
    /// with matching winding, or if the vertex has loose edges.
    fn fan(&self, vertex: VertexId) -> Option<Fan> {
        let corners: Vec<CornerId> = self.corners_at(vertex).collect();
        let arriving = |c: CornerId| self.corners[self.corners[c].prev].edge;
        let mut by_arriving: HashMap<EdgeId, CornerId> = HashMap::new();
        let mut leaving: HashSet<EdgeId> = HashSet::new();
        for &c in &corners {
            if by_arriving.insert(arriving(c), c).is_some() || !leaving.insert(self.corners[c].edge)
            {
                return None;
            }
        }
        // An open fan starts at the face whose arriving edge no face leaves by.
        let starts: Vec<CornerId> = corners
            .iter()
            .copied()
            .filter(|&c| !leaving.contains(&arriving(c)))
            .collect();
        let first = match starts.as_slice() {
            [] => *corners.first()?,
            [only] => *only,
            _ => return None,
        };
        let mut ordered = vec![first];
        let mut current = first;
        let closed = loop {
            match by_arriving.get(&self.corners[current].edge) {
                Some(&next) if next == first => break true,
                Some(&next) => {
                    if ordered.len() > corners.len() {
                        return None;
                    }
                    ordered.push(next);
                    current = next;
                }
                None => break false,
            }
        };
        if ordered.len() != corners.len() || (closed && ordered.len() < 3) {
            return None;
        }
        let mut edges: Vec<EdgeId> = ordered.iter().map(|&c| arriving(c)).collect();
        if !closed {
            edges.push(self.corners[current].edge);
        }
        if self.disk_walk(vertex).count() != edges.len() {
            return None;
        }

        let here = self.vertices[vertex].position;
        let toward = |e: EdgeId| {
            let other = self.edges[e].other_vertex(vertex)?;
            unit(sub(self.vertices[other].position, here))
        };
        let mut across = Vec::with_capacity(ordered.len());
        let mut along = Vec::with_capacity(ordered.len());
        for (i, &c) in ordered.iter().enumerate() {
            let face = self.corners[c].face;
            let (into, out_of) = (edges[i], edges[(i + 1) % edges.len()]);
            let (u_in, u_out) = (toward(into)?, toward(out_of)?);
            let normal = self
                .face_normal(face)
                .ok()
                .flatten()
                .or_else(|| unit(cross(u_out, u_in)))?;
            // Faces wind counterclockwise about their normal, so the inside of an edge
            // lies to the left of the way the face runs along it.
            let inward_in = unit(cross(normal, scale(u_in, -1.0)))?;
            let inward_out = unit(cross(normal, u_out))?;
            across.push([inward_in, inward_out]);
            along.push([u_in, u_out]);
        }
        Some(Fan {
            corners: ordered,
            edges,
            closed,
            across,
            along,
        })
    }

    /// Bevels the selected edges. Each must have one face on each side, with both
    /// faces winding the same way around, and the faces around each end must form a
    /// simple fan. The width is limited so new points cannot pass each other. The new
    /// faces become the selection.
    pub fn bevel_selected_edges(
        &mut self,
        width: f32,
        segments: usize,
        mode: SelectMode,
    ) -> Result<Bevel, MeshError> {
        if !width.is_finite() || width < 0.0 {
            return Err(MeshError::InvalidDistance);
        }
        if !(1..=MAX_BEVEL_SEGMENTS).contains(&segments) {
            return Err(MeshError::InvalidSegments);
        }
        let beveled: HashSet<EdgeId> = self
            .edges()
            .filter(|(_, e)| e.selected)
            .map(|(id, _)| id)
            .collect();
        if beveled.is_empty() {
            return Err(MeshError::NothingSelected);
        }

        // Each beveled edge needs a face on each side, running opposite ways.
        let mut sides: HashMap<EdgeId, [CornerId; 2]> = HashMap::new();
        for &edge in &beveled {
            let corners: Vec<CornerId> = self.radial_walk(edge).collect();
            let [a, b] = <[CornerId; 2]>::try_from(corners).map_err(|_| MeshError::CannotBevel)?;
            if self.corners[a].vertex == self.corners[b].vertex {
                return Err(MeshError::CannotBevel);
            }
            let first = self.edges[edge].vertices[0];
            // Side 0 runs from the edge's first vertex to its second.
            sides.insert(
                edge,
                if self.corners[a].vertex == first {
                    [a, b]
                } else {
                    [b, a]
                },
            );
        }
        let mut hubs: Vec<VertexId> = beveled
            .iter()
            .flat_map(|&e| self.edges[e].vertices)
            .collect();
        hubs.sort_unstable();
        hubs.dedup();
        let mut fans: HashMap<VertexId, Fan> = HashMap::with_capacity(hubs.len());
        for &v in &hubs {
            fans.insert(v, self.fan(v).ok_or(MeshError::CannotBevel)?);
        }

        // The direction each new point moves in, per unit of width.
        let mut steps: HashMap<Spot, V3> = HashMap::new();
        let mut rings: HashMap<VertexId, Vec<Around>> = HashMap::new();
        for &v in &hubs {
            let fan = &fans[&v];
            let n = fan.corners.len();
            let count = fan.edges.len();
            let is_beveled = |i: usize| beveled.contains(&fan.edges[i]);
            let mut ring = Vec::with_capacity(count * 2);
            for i in 0..count {
                let edge = fan.edges[i];
                if is_beveled(i) {
                    ring.push(Around::Beveled(edge));
                } else {
                    // The face before this edge leaves by it; the face after arrives by it.
                    let mut distances = Vec::new();
                    if (fan.closed || i > 0) && is_beveled((i + count - 1) % count) {
                        let face = (i + n - 1) % n;
                        distances.push((fan.along[face][1], fan.across[face][0]));
                    }
                    if i < n && is_beveled((i + 1) % count) {
                        distances.push((fan.along[i][0], fan.across[i][1]));
                    }
                    if distances.is_empty() {
                        ring.push(Around::Point(Spot::Keep(v)));
                    } else {
                        let dir = distances[0].0;
                        let share = distances
                            .iter()
                            .map(|&(u, inward)| 1.0 / dot(u, inward).max(0.2))
                            .sum::<f32>()
                            / distances.len() as f32;
                        let spot = Spot::OnEdge(v, edge);
                        steps.insert(spot, scale(dir, share));
                        ring.push(Around::Point(spot));
                    }
                }
                if i < n && is_beveled(i) && beveled.contains(&fan.next_edge(i)) {
                    let [a, b] = fan.across[i];
                    let spot = Spot::Corner(v, i);
                    steps.insert(spot, scale(add(a, b), 1.0 / (1.0 + dot(a, b)).max(0.1)));
                    ring.push(Around::Point(spot));
                }
            }
            // Neighboring kept points are the same vertex.
            ring.dedup();
            if ring.len() > 1 && ring.first() == ring.last() {
                ring.pop();
            }
            rings.insert(v, ring);
        }

        // The width must not push points past the middle of an edge they move along.
        let mut max_width = f32::MAX;
        for &v in &hubs {
            let here = self.vertices[v].position;
            let half = |e: EdgeId| {
                let [a, b] = self.edges[e].vertices;
                length(sub(self.vertices[a].position, self.vertices[b].position)) / 2.0
            };
            for around in &rings[&v] {
                let Around::Point(spot) = around else {
                    continue;
                };
                let Some(&step) = steps.get(spot) else {
                    continue;
                };
                if let Spot::OnEdge(_, e) = spot {
                    max_width = max_width.min(half(*e) / length(step).max(1e-9));
                }
                for edge in self.disk_walk(v).filter(|e| beveled.contains(e)) {
                    let Some(other) = self.edges[edge].other_vertex(v) else {
                        continue;
                    };
                    let Some(dir) = unit(sub(self.vertices[other].position, here)) else {
                        continue;
                    };
                    let reach = dot(step, dir);
                    if reach > 1e-6 {
                        max_width = max_width.min(half(edge) / reach);
                    }
                }
            }
        }
        let max_width = max_width * 0.999;
        let width = width.min(max_width);

        // New vertices for every point.
        let mut moving: Vec<(VertexId, V3, V3)> = Vec::new();
        let mut made: HashMap<Spot, VertexId> = HashMap::new();
        let mut step_of: HashMap<VertexId, V3> = HashMap::new();
        for &v in &hubs {
            let here = self.vertices[v].position;
            for around in &rings[&v] {
                if let Around::Point(spot) = *around {
                    if let Spot::Keep(kept) = spot {
                        made.insert(spot, kept);
                        step_of.insert(kept, [0.0; 3]);
                    } else if let Entry::Vacant(slot) = made.entry(spot) {
                        let step = steps[&spot];
                        let id = self.add_vertex(add(here, scale(step, width)))?;
                        slot.insert(id);
                        step_of.insert(id, step);
                        moving.push((id, here, step));
                    }
                }
            }
        }

        // Profiles between two points around a vertex, shared by everything using them.
        let mut profiles: HashMap<(VertexId, VertexId, VertexId), Vec<VertexId>> = HashMap::new();
        // Steps of the profiles' inner points, for placing corner centers.
        let mut inner_steps: HashMap<VertexId, V3> = HashMap::new();
        let mut profile = |mesh: &mut Mesh,
                           moving: &mut Vec<(VertexId, V3, V3)>,
                           inner_steps: &mut HashMap<VertexId, V3>,
                           v: VertexId,
                           from: VertexId,
                           to: VertexId,
                           along: Option<V3>|
         -> Result<Vec<VertexId>, MeshError> {
            let key = (v, from.min(to), from.max(to));
            if let Entry::Vacant(slot) = profiles.entry(key) {
                let here = mesh.vertices[v].position;
                let (a, c) = (step_of[&key.1], step_of[&key.2]);
                // The curve bends toward where the faces' tangents meet. Where a corner
                // face is made, that is on the beveled edge, level with the ends.
                // Otherwise it is the vertex itself, as where two bands meet in a mitre.
                let k = along.map_or([0.0; 3], |u| scale(u, (dot(a, u) + dot(c, u)) / 2.0));
                // A rational quadratic curve from a to c, pulled toward k: a circular arc
                // when both ends are the same distance from k. The weight is the sine of
                // half the angle at k: 0.707 for a right angle, an exact quarter circle.
                let (ka, kc) = (sub(a, k), sub(c, k));
                let cosine = dot(ka, kc) / (length(ka) * length(kc)).max(1e-12);
                let pull = ((1.0 - cosine.clamp(-1.0, 1.0)) / 2.0)
                    .sqrt()
                    .clamp(0.01, 1.0);
                let mut inner = Vec::with_capacity(segments.saturating_sub(1));
                for j in 1..segments {
                    let t = j as f32 / segments as f32;
                    let (wa, wc, wk) = ((1.0 - t) * (1.0 - t), t * t, 2.0 * t * (1.0 - t) * pull);
                    let sum = add(add(scale(a, wa), scale(c, wc)), scale(k, wk));
                    let step = scale(sum, 1.0 / (wa + wc + wk));
                    let id = mesh.add_vertex(add(here, scale(step, width)))?;
                    moving.push((id, here, step));
                    inner_steps.insert(id, step);
                    inner.push(id);
                }
                slot.insert(inner);
            }
            let mut inner = profiles[&key].clone();
            if from > to {
                inner.reverse();
            }
            Ok(inner)
        };

        // Where an edge of a face meets a fan vertex, as a point.
        let spot_for = |fan: &Fan, v: VertexId, face: usize, edge: EdgeId| {
            let (into, out_of) = (fan.edges[face], fan.next_edge(face));
            if beveled.contains(&edge) {
                let other = if edge == into { out_of } else { into };
                if beveled.contains(&other) {
                    Spot::Corner(v, face)
                } else {
                    Spot::OnEdge(v, other)
                }
            } else if steps.contains_key(&Spot::OnEdge(v, edge)) {
                Spot::OnEdge(v, edge)
            } else {
                Spot::Keep(v)
            }
        };

        // New outlines for every face touching a beveled vertex.
        let mut fan_face: HashMap<(VertexId, FaceId), usize> = HashMap::new();
        let mut affected: Vec<FaceId> = Vec::new();
        for &v in &hubs {
            for (i, &c) in fans[&v].corners.iter().enumerate() {
                let face = self.corners[c].face;
                fan_face.insert((v, face), i);
                affected.push(face);
            }
        }
        affected.sort_unstable();
        affected.dedup();
        let mut new_faces: Vec<Vec<VertexId>> = Vec::new();
        for &face in &affected {
            let corners: Vec<CornerId> = self.face_walk(face).collect();
            let mut outline = Vec::new();
            for c in corners {
                let data = self.corners[c];
                let v = data.vertex;
                let Some(&i) = fan_face.get(&(v, face)) else {
                    outline.push(v);
                    continue;
                };
                let fan = &fans[&v];
                let into = self.corners[data.prev].edge;
                let a = made[&spot_for(fan, v, i, into)];
                let b = made[&spot_for(fan, v, i, data.edge)];
                outline.push(a);
                if a != b {
                    // With only two points around the vertex, a band's profile joins the
                    // same two points as this face's edge does, so the face follows the
                    // profile. This is where a single beveled edge ends at a corner
                    // with three edges.
                    let points = rings[&v]
                        .iter()
                        .filter(|r| matches!(r, Around::Point(_)))
                        .count();
                    if points == 2 && segments > 1 {
                        outline.extend(profile(
                            self,
                            &mut moving,
                            &mut inner_steps,
                            v,
                            a,
                            b,
                            None,
                        )?);
                    }
                    outline.push(b);
                }
            }
            new_faces.push(outline);
        }

        let points_around = |v: VertexId| {
            rings[&v]
                .iter()
                .filter(|r| matches!(r, Around::Point(_)))
                .count()
        };

        // Bands along the beveled edges.
        let mut bevel_faces: Vec<Vec<VertexId>> = Vec::new();
        let mut sorted_beveled: Vec<EdgeId> = beveled.iter().copied().collect();
        sorted_beveled.sort_unstable();
        for &edge in &sorted_beveled {
            let [p, q] = self.edges[edge].vertices;
            let mut columns = Vec::with_capacity(2);
            for v in [p, q] {
                let fan = &fans[&v];
                // From the side running p to q, to the side running q to p.
                let [start, end] = sides[&edge].map(|corner| {
                    let i = fan_face[&(v, self.corners[corner].face)];
                    made[&spot_for(fan, v, i, edge)]
                });
                let mut points = vec![start];
                let along = (points_around(v) >= 3)
                    .then(|| {
                        let other = self.edges[edge].other_vertex(v)?;
                        unit(sub(
                            self.vertices[other].position,
                            self.vertices[v].position,
                        ))
                    })
                    .flatten();
                points.extend(profile(
                    self,
                    &mut moving,
                    &mut inner_steps,
                    v,
                    start,
                    end,
                    along,
                )?);
                points.push(end);
                columns.push(points);
            }
            let (at_p, at_q) = (&columns[0], &columns[1]);
            for j in 0..segments {
                bevel_faces.push(vec![at_q[j], at_p[j], at_p[j + 1], at_q[j + 1]]);
            }
        }

        // Corner faces where three or more points surround a vertex.
        for &v in &hubs {
            let ring = &rings[&v];
            let points = ring
                .iter()
                .filter(|r| matches!(r, Around::Point(_)))
                .count();
            if points < 3 {
                continue;
            }
            let count = ring.len();
            let mut outline: Vec<VertexId> = Vec::new();
            let mut curved = false;
            for (k, around) in ring.iter().enumerate() {
                match *around {
                    Around::Point(spot) => outline.push(made[&spot]),
                    Around::Beveled(edge) => {
                        let before = ring[(k + count - 1) % count];
                        let after = ring[(k + 1) % count];
                        if let (Around::Point(a), Around::Point(b)) = (before, after) {
                            let along = self.edges[edge].other_vertex(v).and_then(|other| {
                                unit(sub(
                                    self.vertices[other].position,
                                    self.vertices[v].position,
                                ))
                            });
                            let inner = profile(
                                self,
                                &mut moving,
                                &mut inner_steps,
                                v,
                                made[&a],
                                made[&b],
                                along,
                            )?;
                            curved |= !inner.is_empty();
                            outline.extend(inner);
                        }
                    }
                }
            }
            outline.dedup();
            if outline.len() > 1 && outline.first() == outline.last() {
                outline.pop();
            }
            // Faces run the other way round the vertex from the fan's order.
            outline.reverse();
            if curved {
                let here = self.vertices[v].position;
                let corner_points: Vec<VertexId> = ring
                    .iter()
                    .filter_map(|r| match r {
                        Around::Point(spot) => Some(made[spot]),
                        Around::Beveled(_) => None,
                    })
                    .collect();
                let sphere = self.corner_sphere(v, &fans[&v], ring, &steps, &made, &step_of);
                // The three sides of a three-cornered patch, each from corner to corner.
                let mut sides3 = Vec::new();
                if sphere.is_some() && corner_points.len() == 3 {
                    for k in 0..3 {
                        let (from, to) = (corner_points[k], corner_points[(k + 1) % 3]);
                        let mut line = vec![from];
                        line.extend(profile(
                            self,
                            &mut moving,
                            &mut inner_steps,
                            v,
                            from,
                            to,
                            None,
                        )?);
                        line.push(to);
                        sides3.push(line);
                    }
                }
                let mut added: Vec<(VertexId, V3)> = Vec::new();
                let mut new_point = |mesh: &mut Mesh, step: V3| -> Result<VertexId, MeshError> {
                    let id = mesh.add_vertex(add(here, scale(step, width)))?;
                    added.push((id, step));
                    Ok(id)
                };
                match sphere {
                    // Three beveled edges meeting, as at a cube corner: a triangle of
                    // the corner's sphere, divided into a grid of small triangles.
                    Some((middle, radius)) if corner_points.len() == 3 => {
                        let s = segments;
                        let [a, b, c] = [corner_points[0], corner_points[1], corner_points[2]];
                        let (ab, bc, ca) = (&sides3[0], &sides3[1], &sides3[2]);
                        let reach =
                            |p: VertexId| sub(step_of_point(&step_of, &inner_steps, p), middle);
                        let (ra, rb, rc) = (reach(a), reach(b), reach(c));
                        // Points by how many steps they are from b and from c.
                        let mut grid: HashMap<(usize, usize), VertexId> = HashMap::new();
                        // j and k are grid coordinates, which only sometimes index a side.
                        #[allow(clippy::needless_range_loop)]
                        for j in 0..=s {
                            for k in 0..=(s - j) {
                                let i = s - j - k;
                                let id = if k == 0 {
                                    ab[j]
                                } else if i == 0 {
                                    bc[k]
                                } else if j == 0 {
                                    ca[i]
                                } else {
                                    let blend = add(
                                        add(scale(ra, i as f32), scale(rb, j as f32)),
                                        scale(rc, k as f32),
                                    );
                                    let dir = unit(blend).unwrap_or(ra);
                                    new_point(self, add(middle, scale(dir, radius)))?
                                };
                                grid.insert((j, k), id);
                            }
                        }
                        let mut triangles = Vec::with_capacity(s * s);
                        for j in 0..s {
                            for k in 0..(s - j) {
                                triangles.push([
                                    grid[&(j, k)],
                                    grid[&(j + 1, k)],
                                    grid[&(j, k + 1)],
                                ]);
                                if j + k + 2 <= s {
                                    triangles.push([
                                        grid[&(j + 1, k)],
                                        grid[&(j + 1, k + 1)],
                                        grid[&(j, k + 1)],
                                    ]);
                                }
                            }
                        }
                        // Turn each triangle to face the same way as the corner's outline.
                        let made_here: HashMap<VertexId, V3> = added.iter().copied().collect();
                        let at = |p: VertexId| {
                            made_here
                                .get(&p)
                                .copied()
                                .unwrap_or_else(|| step_of_point(&step_of, &inner_steps, p))
                        };
                        let facing = newell(&outline.iter().map(|&p| at(p)).collect::<Vec<_>>());
                        for [p, q, r] in triangles {
                            let normal = cross(sub(at(q), at(p)), sub(at(r), at(p)));
                            if dot(normal, facing) >= 0.0 {
                                bevel_faces.push(vec![p, q, r]);
                            } else {
                                bevel_faces.push(vec![p, r, q]);
                            }
                        }
                    }
                    _ => {
                        // A fan of triangles around a raised center.
                        let mut sum = [0.0; 3];
                        for &p in &corner_points {
                            sum = add(sum, step_of[&p]);
                        }
                        let average = scale(sum, 1.0 / corner_points.len() as f32);
                        let step = match sphere {
                            Some((middle, radius)) => add(
                                middle,
                                scale(unit(sub(average, middle)).unwrap_or([0.0; 3]), radius),
                            ),
                            // On a rounded cube corner, this share puts it on the sphere.
                            None => scale(average, CAP_CENTER_SHARE),
                        };
                        let center = new_point(self, step)?;
                        let n = outline.len();
                        for k in 0..n {
                            bevel_faces.push(vec![center, outline[k], outline[(k + 1) % n]]);
                        }
                    }
                }
                moving.extend(added.into_iter().map(|(id, step)| (id, here, step)));
            } else {
                bevel_faces.push(outline);
            }
        }

        // Out with the old.
        for &face in &affected {
            self.remove_face(face)?;
        }
        for &edge in &sorted_beveled {
            self.remove_edge(edge)?;
        }
        for &v in &hubs {
            let kept = rings[&v]
                .iter()
                .any(|r| matches!(r, Around::Point(Spot::Keep(_))));
            if kept {
                let fan = &fans[&v];
                let moved: Vec<EdgeId> = fan
                    .edges
                    .iter()
                    .copied()
                    .filter(|&e| steps.contains_key(&Spot::OnEdge(v, e)))
                    .collect();
                for edge in moved {
                    if self.edges.contains(edge) {
                        self.remove_edge(edge)?;
                    }
                }
            } else {
                self.remove_vertex(v)?;
            }
        }

        // In with the new.
        let refused = |error: MeshError| match error {
            MeshError::FaceExists(_)
            | MeshError::RepeatedVertex(_)
            | MeshError::FaceTooSmall { .. } => MeshError::CannotBevel,
            other => other,
        };
        for outline in new_faces {
            self.add_face(&outline).map_err(refused)?;
        }
        let mut selected = Vec::with_capacity(bevel_faces.len());
        for outline in bevel_faces {
            selected.push(Element::Face(self.add_face(&outline).map_err(refused)?));
        }
        self.select_elements(SelectMode::Face, &selected, SelectOp::Replace)?;
        self.flush_selection(mode);
        Ok(Bevel {
            moving,
            width,
            max_width,
        })
    }
}
