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
//! grid of four-sided faces, as in Blender; see `corner.rs`. Where only two points
//! surround it, the bands meet directly, as where a beveled loop of edges turns a
//! corner.
//!
//! Every new point lies at its vertex plus the width times a fixed direction, so a
//! drag can change the width by moving points, without rebuilding anything.
//!
//! As in `tools.rs`, run this inside a `Document` edit.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};

use crate::corner::{self, Node, Pipe, Side};
use crate::error::MeshError;
use crate::id::{CornerId, EdgeId, FaceId, MaterialId, VertexId};
use crate::mesh::{Look, Mesh};
use crate::profile::CustomProfile;
use crate::selection::{Element, SelectMode, SelectOp};

type V3 = [f32; 3];

/// The most segments one bevel can have.
pub const MAX_BEVEL_SEGMENTS: usize = 16;

/// The width limit when overlap is not clamped: in effect none.
const UNCLAMPED: f32 = 1.0e6;

/// The most spread a miter can have, as a share of the width.
pub const MAX_SPREAD: f32 = 1.0;

/// What the width measures, as in Blender.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidthType {
    /// From the original edge to each new edge, across the faces.
    Offset,
    /// Between the two new edges, straight across the bevel.
    Width,
    /// From the original edge straight in to the new flat face.
    Depth,
    /// A percentage of the length of the edges the new points move along.
    Percent,
    /// How far the new points move along the edges next to the bevel.
    Absolute,
}

impl WidthType {
    /// Every kind, in Blender's order.
    pub const ALL: [WidthType; 5] = [
        WidthType::Offset,
        WidthType::Width,
        WidthType::Depth,
        WidthType::Percent,
        WidthType::Absolute,
    ];

    /// Its name in the panel.
    pub fn name(self) -> &'static str {
        match self {
            WidthType::Offset => "Offset",
            WidthType::Width => "Width",
            WidthType::Depth => "Depth",
            WidthType::Percent => "Percent",
            WidthType::Absolute => "Absolute",
        }
    }
}

/// The profile kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileType {
    /// Round, flat or pointed, set by `shape`.
    Superellipse,
    /// Any shape, drawn as a line through points; see `CustomProfile`.
    Custom,
}

/// How bands meet where two beveled edges turn a corner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Miter {
    /// In one line.
    Sharp,
    /// With a small patch between them.
    Patch,
    /// With a rounded arc between them.
    Arc,
}

/// How bands meet where three or more beveled edges meet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intersection {
    /// A corner patch following the bands' profiles.
    GridFill,
    /// Each band's end cut off flat, with a flat face between the cuts.
    Cutoff,
}

/// The choices for a bevel, as in Blender's bevel panel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BevelSettings {
    /// Bevel the selected vertices on their own, instead of the selected edges.
    pub vertices: bool,
    /// How far the new points move, measured as `width_type` says.
    pub width: f32,
    /// What `width` measures.
    pub width_type: WidthType,
    /// From 1 to `MAX_BEVEL_SEGMENTS`.
    pub segments: usize,
    /// Superellipse or custom.
    pub profile_type: ProfileType,
    /// The superellipse's shape, from 0 to 1: 0.5 is round, 0.25 straight, 1 a sharp
    /// corner, and below 0.25 it curves inward. Matters only with more than one segment.
    pub shape: f32,
    /// The custom profile, used when `profile_type` is `Custom`.
    pub custom: CustomProfile,
    /// Stop the width before new points pass each other.
    pub clamp: bool,
    /// New points slide along the edges next to the bevel, rather than keeping an even
    /// width, so those edges keep their direction.
    pub loop_slide: bool,
    /// The edges between the bevel and the faces around it become sharp, so smooth
    /// shading stops there.
    pub mark_sharp: bool,
    /// Where two beveled edges turn a corner: the outer side, then the inner side.
    /// The inner side takes `Sharp` or `Arc`.
    pub miter_outer: Miter,
    /// See `miter_outer`.
    pub miter_inner: Miter,
    /// How far a patch or arc spreads along the edges, as a share of the width.
    pub spread: f32,
    /// Where three or more beveled edges meet.
    pub intersection: Intersection,
    /// The material for the bevel's new faces, as Blender's Material Index; `None`
    /// takes the material of the faces around.
    pub material: Option<MaterialId>,
}

impl Default for BevelSettings {
    fn default() -> Self {
        Self {
            vertices: false,
            width: 0.0,
            width_type: WidthType::Offset,
            segments: 1,
            profile_type: ProfileType::Superellipse,
            shape: 0.5,
            custom: CustomProfile::default(),
            clamp: true,
            loop_slide: true,
            mark_sharp: false,
            miter_outer: Miter::Sharp,
            miter_inner: Miter::Sharp,
            spread: 0.1,
            intersection: Intersection::GridFill,
            material: None,
        }
    }
}

/// The profile used by one bevel: its inner points, each as `[x, y]` in the frame
/// where `[1, 0]` and `[0, 1]` are the ends and `[1, 1]` the original sharp edge.
struct ProfileShape {
    inner: Vec<[f32; 2]>,
    /// Where the profile crosses the frame's diagonal, from 0 at the center to 1 at the
    /// sharp edge: 0.71 for a round, 0.5 for a chamfer.
    diagonal: f32,
    /// A superellipse's exponent. It is the same from both ends, so its middle is on
    /// the diagonal. `None` for a custom profile.
    exponent: Option<f32>,
}

impl ProfileShape {
    fn new(settings: &BevelSettings) -> Self {
        let segments = settings.segments;
        match settings.profile_type {
            ProfileType::Superellipse => {
                let r = exponent(settings.shape);
                let points = corner::even_superellipse(r, segments);
                Self {
                    inner: points[1..segments].to_vec(),
                    diagonal: 0.5f32.powf(1.0 / r),
                    exponent: Some(r),
                }
            }
            ProfileType::Custom => {
                let samples = settings.custom.sample(segments);
                // The profile's square has the sharp edge at its (0, 0).
                let frame = |p: [f32; 2]| [1.0 - p[1], 1.0 - p[0]];
                Self {
                    inner: samples[1..segments].iter().map(|&p| frame(p)).collect(),
                    diagonal: 1.0 - settings.custom.diagonal(),
                    exponent: None,
                }
            }
        }
    }

    /// The inner points of a profile from `a` to `c` around the corner point `k`, all
    /// as steps per unit of width: the frame runs from `o = a + c - k` to `a` and `c`.
    /// With the round shape, an exact quarter circle when `k` is a right angle.
    fn steps(&self, a: V3, c: V3, k: V3) -> Vec<V3> {
        let o = sub(add(a, c), k);
        let (oa, oc) = (sub(a, o), sub(c, o));
        self.inner
            .iter()
            .map(|&[x, y]| add(o, add(scale(oa, x), scale(oc, y))))
            .collect()
    }

    /// The middle of the profile from `a` to `c` around `k`, where that is known. With
    /// an odd number of segments no point sits there, but a corner's grid needs it.
    fn middle(&self, a: V3, c: V3, k: V3) -> Option<V3> {
        self.exponent.map(|_| {
            let o = sub(add(a, c), k);
            add(o, scale(add(sub(a, o), sub(c, o)), self.diagonal))
        })
    }
}

/// One side of a corner's outline for `corner::fill`: its points' steps, and the
/// middle of its curve, which is half way along the middle piece unless given.
fn outline_side(points: Vec<V3>, middle: Option<V3>) -> Side {
    let h = (points.len() - 1) / 2;
    let middle = if points.len() % 2 == 1 {
        points[h]
    } else {
        middle.unwrap_or_else(|| scale(add(points[h], points[h + 1]), 0.5))
    };
    Side { points, middle }
}

/// Which end a one-sided profile starts from: the one on the face turned more
/// upward, judged by how its point moves, so molding presets come out the same way
/// on every edge. True if `a` goes first.
fn first_is_a(a: V3, c: V3) -> bool {
    for axis in [2, 0, 1] {
        if (a[axis] - c[axis]).abs() > 1e-5 {
            return a[axis] > c[axis];
        }
    }
    true
}

/// The superellipse exponent for a shape, measured as Blender's Shape is: the profile
/// crosses its frame's diagonal at the square root of the shape. So 0.5 gives 2 (a
/// circle), 0.25 gives 1 (a straight line), toward 1 the corner grows sharp, and below
/// 0.25 the profile curves inward.
fn exponent(shape: f32) -> f32 {
    if is_round(shape) {
        return 2.0;
    }
    if (shape - 0.25).abs() < 1e-4 {
        return 1.0;
    }
    if shape <= 0.0 {
        return 0.25;
    }
    if shape >= 1.0 {
        return 64.0;
    }
    (-2.0 * std::f32::consts::LN_2 / shape.ln()).clamp(0.25, 64.0)
}

fn is_round(shape: f32) -> bool {
    (shape - 0.5).abs() < 1e-4
}

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
    /// How far points move per unit of the width as its type measures it, so that
    /// `width` here is the setting's width times this.
    pub per_unit: f32,
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
    /// Where a band ends at a patch or arc miter: its vertex, the beveled edge, and
    /// whether on the outer side.
    Miter(VertexId, EdgeId, bool),
}

/// A vertex where two beveled edges turn a corner, with a patch or arc on a side.
struct MiterAt {
    /// For each fan face: whether it is on the outer side.
    outer_faces: Vec<bool>,
    /// The point on each side before mitering.
    inner_point: Spot,
    outer_point: Spot,
    /// The two beveled edges, and the directions along them away from the vertex.
    edges: [EdgeId; 2],
    outer: Miter,
    inner_arc: bool,
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
        // A face with no area of its own (all its points in a line, as a bevel's cut
        // end can be) borrows the way the faces around it face.
        let mut sum = [0.0f32; 3];
        for &c in &ordered {
            if let Ok(Some(n)) = self.face_normal(self.corners[c].face) {
                sum = add(sum, n);
            }
        }
        let around = unit(sum);
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
                .or_else(|| unit(cross(u_out, u_in)))
                .or(around)?;
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
        let settings = BevelSettings {
            width,
            segments,
            ..BevelSettings::default()
        };
        self.bevel_selected(&settings, mode)
    }

    /// Bevels the selected edges, or with `settings.vertices` the selected vertices.
    pub fn bevel_selected(
        &mut self,
        settings: &BevelSettings,
        mode: SelectMode,
    ) -> Result<Bevel, MeshError> {
        // Points sitting on top of each other, as a bevel kept at no width leaves,
        // cannot be bevelled; saying so points to the fix.
        let stacked = self.edges().any(|(_, e)| {
            let [a, b] = e.vertices;
            (self.vertices[a].selected || self.vertices[b].selected)
                && length(sub(self.vertices[a].position, self.vertices[b].position)) < 1e-6
        });
        match self.bevel_selected_as_is(settings, mode) {
            Err(MeshError::CannotBevel) if stacked => Err(MeshError::StackedPoints),
            other => other,
        }
    }

    fn bevel_selected_as_is(
        &mut self,
        settings: &BevelSettings,
        mode: SelectMode,
    ) -> Result<Bevel, MeshError> {
        let BevelSettings {
            width: value,
            segments,
            shape,
            clamp,
            vertices,
            spread,
            ..
        } = *settings;
        if !value.is_finite() || value < 0.0 {
            return Err(MeshError::InvalidDistance);
        }
        if !(1..=MAX_BEVEL_SEGMENTS).contains(&segments) {
            return Err(MeshError::InvalidSegments);
        }
        if !shape.is_finite() || !(0.0..=1.0).contains(&shape) {
            return Err(MeshError::InvalidShape);
        }
        if !spread.is_finite() || !(0.0..=MAX_SPREAD).contains(&spread) {
            return Err(MeshError::InvalidShape);
        }
        if settings.profile_type == ProfileType::Custom
            && settings
                .custom
                .points()
                .iter()
                .any(|p| !p.x.is_finite() || !p.y.is_finite())
        {
            return Err(MeshError::InvalidShape);
        }
        let profile_shape = ProfileShape::new(settings);
        if vertices {
            return self.bevel_vertices(settings, &profile_shape, mode);
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
                        let step = if settings.loop_slide {
                            // Along the edge, as far as gives the width across the face.
                            let share = distances
                                .iter()
                                .map(|&(u, inward)| 1.0 / dot(u, inward).max(0.2))
                                .sum::<f32>()
                                / distances.len() as f32;
                            scale(dir, share)
                        } else {
                            // Straight across from the beveled edge, for an even width.
                            let mut sum = [0.0; 3];
                            for &(_, inward) in &distances {
                                sum = add(sum, inward);
                            }
                            scale(sum, 1.0 / distances.len() as f32)
                        };
                        let spot = Spot::OnEdge(v, edge);
                        steps.insert(spot, step);
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

        // Where two beveled edges turn a corner, with a patch or arc on a side.
        let mut miters: HashMap<VertexId, MiterAt> = HashMap::new();
        let mut extra_spots: HashMap<VertexId, Vec<Spot>> = HashMap::new();
        let wants_miter =
            settings.miter_outer != Miter::Sharp || settings.miter_inner == Miter::Arc;
        for &v in &hubs {
            if !wants_miter {
                break;
            }
            let fan = &fans[&v];
            let ring = &rings[&v];
            let beveled_at: Vec<usize> = (0..fan.edges.len())
                .filter(|&i| beveled.contains(&fan.edges[i]))
                .collect();
            let points: Vec<Spot> = ring
                .iter()
                .filter_map(|r| match r {
                    Around::Point(spot) => Some(*spot),
                    Around::Beveled(_) => None,
                })
                .collect();
            let ([i1, i2], true, 2) = (
                <[usize; 2]>::try_from(beveled_at).unwrap_or([0, 0]),
                fan.closed,
                points.len(),
            ) else {
                continue;
            };
            if i1 == i2 {
                continue;
            }
            // Faces i1 .. i2 lie on one side, the rest on the other.
            let n = fan.corners.len();
            let side_a: Vec<bool> = (0..n).map(|f| f >= i1 && f < i2).collect();
            let angle = |f: usize| {
                dot(fan.along[f][0], fan.along[f][1])
                    .clamp(-1.0, 1.0)
                    .acos()
            };
            let (mut sum_a, mut sum_b) = (0.0, 0.0);
            for (f, &on_a) in side_a.iter().enumerate() {
                if on_a {
                    sum_a += angle(f);
                } else {
                    sum_b += angle(f);
                }
            }
            // A real turn only; a straight run of edges needs no miter.
            if (sum_a - sum_b).abs() < 0.1 {
                continue;
            }
            let a_is_outer = sum_a > sum_b;
            let outer_faces: Vec<bool> = side_a.iter().map(|&a| a == a_is_outer).collect();
            // The point on each side: the one some face of that side uses.
            let side_point = |outer: bool| {
                (0..n)
                    .filter(|&f| outer_faces[f] == outer)
                    .flat_map(|f| [fan.edges[f], fan.next_edge(f)].map(|e| (f, e)))
                    .map(|(f, e)| base_spot(fan, v, f, e, &beveled, &steps))
                    .find(|spot| points.contains(spot))
            };
            let (Some(inner_point), Some(outer_point)) = (side_point(false), side_point(true))
            else {
                continue;
            };
            if inner_point == outer_point {
                continue;
            }
            let edges = [fan.edges[i1], fan.edges[i2]];
            let here = self.vertices[v].position;
            let mut extra = Vec::new();
            for (k, &edge) in edges.iter().enumerate() {
                let other = self.edges[edge].other_vertex(v).unwrap_or(v);
                let u = unit(sub(self.vertices[other].position, here)).unwrap_or([0.0; 3]);
                let _ = k;
                for (outer, point, on) in [
                    (true, outer_point, settings.miter_outer != Miter::Sharp),
                    (false, inner_point, settings.miter_inner == Miter::Arc),
                ] {
                    if on {
                        let spot = Spot::Miter(v, edge, outer);
                        let base = steps.get(&point).copied().unwrap_or([0.0; 3]);
                        steps.insert(spot, add(base, scale(u, spread)));
                        extra.push(spot);
                    }
                }
            }
            extra_spots.insert(v, extra);
            miters.insert(
                v,
                MiterAt {
                    outer_faces,
                    inner_point,
                    outer_point,
                    edges,
                    outer: settings.miter_outer,
                    inner_arc: settings.miter_inner == Miter::Arc,
                },
            );
        }

        // Corners that get a grid of faces: more than one segment, and at least two
        // beveled edges with three or more points around. There, a straight side
        // between two points is cut into segments as well, as in Blender, and the face
        // beside it takes the new points.
        let grid_hubs: HashSet<VertexId> = hubs
            .iter()
            .copied()
            .filter(|v| {
                let ring = &rings[v];
                let count = |point: bool| {
                    ring.iter()
                        .filter(|r| matches!(r, Around::Point(_)) == point)
                        .count()
                };
                segments > 1
                    && settings.intersection == Intersection::GridFill
                    && count(true) >= 3
                    && count(false) >= 2
            })
            .collect();

        // How far points move per unit of the width, as its type measures it.
        let per_unit =
            self.width_per_unit(settings.width_type, &beveled, &sides, &fans, &rings, &steps);
        let width = value * per_unit;

        // The width must not push points past the middle of an edge they move along.
        let mut max_width = f32::MAX;
        for &v in &hubs {
            let here = self.vertices[v].position;
            let half = |e: EdgeId| {
                let [a, b] = self.edges[e].vertices;
                length(sub(self.vertices[a].position, self.vertices[b].position)) / 2.0
            };
            let miter_spots = extra_spots.get(&v).cloned().unwrap_or_default();
            let ring_spots = rings[&v].iter().filter_map(|r| match r {
                Around::Point(spot) => Some(*spot),
                Around::Beveled(_) => None,
            });
            for spot in ring_spots.chain(miter_spots) {
                let spot = &spot;
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
        let max_width = if clamp { max_width * 0.999 } else { UNCLAMPED };
        let width = width.min(max_width);

        // New vertices for every point.
        let mut moving: Vec<(VertexId, V3, V3)> = Vec::new();
        let mut made: HashMap<Spot, VertexId> = HashMap::new();
        let mut step_of: HashMap<VertexId, V3> = HashMap::new();
        for &v in &hubs {
            let here = self.vertices[v].position;
            let miter_spots = extra_spots.get(&v).cloned().unwrap_or_default();
            let around_points: Vec<Around> = rings[&v]
                .iter()
                .copied()
                .chain(miter_spots.into_iter().map(Around::Point))
                .collect();
            for around in &around_points {
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
                // One-sided custom profiles start from the end on the upper face.
                let mut points = if first_is_a(a, c) {
                    profile_shape.steps(a, c, k)
                } else {
                    let mut back = profile_shape.steps(c, a, k);
                    back.reverse();
                    back
                };
                let mut inner = Vec::with_capacity(points.len());
                for step in points.drain(..) {
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

        // Where an edge of a face meets a fan vertex, as a point. At a miter, a band
        // ends at its own point on a patched or arced side.
        let spot_for = |fan: &Fan, v: VertexId, face: usize, edge: EdgeId| {
            let spot = base_spot(fan, v, face, edge, &beveled, &steps);
            match miters.get(&v) {
                Some(m) if beveled.contains(&edge) => {
                    let outer = m.outer_faces[face];
                    let miter = Spot::Miter(v, edge, outer);
                    if made.contains_key(&miter) {
                        miter
                    } else {
                        spot
                    }
                }
                _ => spot,
            }
        };
        // Arcs for arc miters: a round curve from `from` to `to` bending toward the
        // point the miter replaces.
        let arc_shape = ProfileShape::new(&BevelSettings {
            segments,
            ..BevelSettings::default()
        });
        let mut arcs: HashMap<(VertexId, VertexId, VertexId), Vec<VertexId>> = HashMap::new();
        let mut arc = |mesh: &mut Mesh,
                       moving: &mut Vec<(VertexId, V3, V3)>,
                       inner_steps: &mut HashMap<VertexId, V3>,
                       v: VertexId,
                       from: VertexId,
                       to: VertexId,
                       toward: V3|
         -> Result<Vec<VertexId>, MeshError> {
            let key = (v, from.min(to), from.max(to));
            if let Entry::Vacant(slot) = arcs.entry(key) {
                let here = mesh.vertices[v].position;
                let mut inner = Vec::new();
                for step in arc_shape.steps(step_of[&key.1], step_of[&key.2], toward) {
                    let id = mesh.add_vertex(add(here, scale(step, width)))?;
                    moving.push((id, here, step));
                    inner_steps.insert(id, step);
                    inner.push(id);
                }
                slot.insert(inner);
            }
            let mut inner = arcs[&key].clone();
            if from > to {
                inner.reverse();
            }
            Ok(inner)
        };

        // Straight lines between two points, cut into the bevel's segments: the flat
        // sides of a corner's grid.
        let mut lines: HashMap<(VertexId, VertexId, VertexId), Vec<VertexId>> = HashMap::new();
        let mut line = |mesh: &mut Mesh,
                        moving: &mut Vec<(VertexId, V3, V3)>,
                        inner_steps: &mut HashMap<VertexId, V3>,
                        v: VertexId,
                        from: VertexId,
                        to: VertexId|
         -> Result<Vec<VertexId>, MeshError> {
            let key = (v, from.min(to), from.max(to));
            if let Entry::Vacant(slot) = lines.entry(key) {
                let here = mesh.vertices[v].position;
                let (a, c) = (step_of[&key.1], step_of[&key.2]);
                let mut inner = Vec::with_capacity(segments.saturating_sub(1));
                for j in 1..segments {
                    let t = j as f32 / segments as f32;
                    let step = add(scale(a, 1.0 - t), scale(c, t));
                    let id = mesh.add_vertex(add(here, scale(step, width)))?;
                    moving.push((id, here, step));
                    inner_steps.insert(id, step);
                    inner.push(id);
                }
                slot.insert(inner);
            }
            let mut inner = lines[&key].clone();
            if from > to {
                inner.reverse();
            }
            Ok(inner)
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
                    if let Some(m) = miters.get(&v) {
                        let old = made[&m.outer_point];
                        if !m.outer_faces[i] {
                            // The inner side of an arc miter: the face's corner
                            // becomes an arc.
                            if m.inner_arc && segments > 1 {
                                let toward = steps[&m.inner_point];
                                outline.extend(arc(
                                    self,
                                    &mut moving,
                                    &mut inner_steps,
                                    v,
                                    a,
                                    b,
                                    toward,
                                )?);
                            }
                        } else if a != old && b != old {
                            // One face holds the whole outer side: it runs along the
                            // patch's two flat sides, or along the arc.
                            match m.outer {
                                Miter::Sharp => {}
                                Miter::Patch => {
                                    outline.extend(line(
                                        self,
                                        &mut moving,
                                        &mut inner_steps,
                                        v,
                                        a,
                                        old,
                                    )?);
                                    outline.push(old);
                                    outline.extend(line(
                                        self,
                                        &mut moving,
                                        &mut inner_steps,
                                        v,
                                        old,
                                        b,
                                    )?);
                                }
                                Miter::Arc => {
                                    let toward = steps[&m.outer_point];
                                    outline.extend(arc(
                                        self,
                                        &mut moving,
                                        &mut inner_steps,
                                        v,
                                        a,
                                        b,
                                        toward,
                                    )?);
                                }
                            }
                        } else if m.outer == Miter::Patch {
                            // Along one flat side of the patch.
                            outline.extend(line(self, &mut moving, &mut inner_steps, v, a, b)?);
                        }
                        outline.push(b);
                        continue;
                    }
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
                    } else if grid_hubs.contains(&v) {
                        // A flat side of the corner's grid.
                        outline.extend(line(self, &mut moving, &mut inner_steps, v, a, b)?);
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

        // Faces whose winding is settled at the end, by the faces they share edges with.
        let mut pending: Vec<Vec<VertexId>> = Vec::new();

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
            // Each band's end, as a flat face, for the cutoff kind of corner.
            let mut cuts: Vec<Vec<VertexId>> = Vec::new();
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
                            if !inner.is_empty() {
                                let mut cut = vec![made[&a]];
                                cut.extend(inner.iter().copied());
                                cut.push(made[&b]);
                                cuts.push(cut);
                            }
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
            if curved && settings.intersection == Intersection::Cutoff {
                // A flat face across the corner points, and each band's end cut off flat.
                let mut center: Vec<VertexId> = ring
                    .iter()
                    .filter_map(|r| match r {
                        Around::Point(spot) => Some(made[spot]),
                        Around::Beveled(_) => None,
                    })
                    .collect();
                center.dedup();
                center.reverse();
                pending.push(center);
                pending.extend(cuts);
                continue;
            }
            if curved && grid_hubs.contains(&v) {
                // A grid of four-sided faces, as in Blender. Its outline has one side
                // per pair of neighboring points: a band's end, or a straight line.
                let here = self.vertices[v].position;
                let at: Vec<(usize, VertexId)> = ring
                    .iter()
                    .enumerate()
                    .filter_map(|(k, r)| match r {
                        Around::Point(spot) => Some((k, made[spot])),
                        Around::Beveled(_) => None,
                    })
                    .collect();
                let mut ids: Vec<Vec<VertexId>> = Vec::with_capacity(at.len());
                let mut sides: Vec<Side> = Vec::with_capacity(at.len());
                // Each band's end: its edge's direction and its two points' steps.
                let mut ends: Vec<(V3, V3, V3)> = Vec::new();
                for (j, &(k, from)) in at.iter().enumerate() {
                    let to = at[(j + 1) % at.len()].1;
                    let (a, c) = (step_of[&from], step_of[&to]);
                    let (inner, middle) = match ring[(k + 1) % count] {
                        Around::Beveled(edge) => {
                            let along = self.edges[edge].other_vertex(v).and_then(|other| {
                                unit(sub(
                                    self.vertices[other].position,
                                    self.vertices[v].position,
                                ))
                            });
                            if let Some(u) = along {
                                ends.push((u, a, c));
                            }
                            let frame =
                                along.map_or([0.0; 3], |u| scale(u, (dot(a, u) + dot(c, u)) / 2.0));
                            (
                                profile(self, &mut moving, &mut inner_steps, v, from, to, along)?,
                                profile_shape.middle(a, c, frame),
                            )
                        }
                        Around::Point(_) => (
                            line(self, &mut moving, &mut inner_steps, v, from, to)?,
                            None,
                        ),
                    };
                    let mut side = vec![from];
                    side.extend(inner);
                    side.push(to);
                    let points = side
                        .iter()
                        .map(|&p| step_of_point(&step_of, &inner_steps, p))
                        .collect();
                    sides.push(outline_side(points, middle));
                    ids.push(side);
                }
                // Two beveled edges in one line, with every side a band's end: a
                // straight run of bevel passing through, as Blender's "pipe".
                let pipe = (ends.len() == sides.len() && sides.len() <= 4)
                    .then(|| {
                        let exponent = profile_shape.exponent?;
                        let (u, a, c) = ends.iter().copied().find(|&(u, _, _)| {
                            ends.iter().any(|&(other, _, _)| dot(u, other) < -0.9999)
                        })?;
                        // The band's profile straight across its edge.
                        let flat = |p: V3| sub(p, scale(u, dot(p, u)));
                        let (a, c) = (flat(a), flat(c));
                        Some(Pipe {
                            along: u,
                            origin: add(a, c),
                            across: [scale(c, -1.0), scale(a, -1.0)],
                            exponent,
                        })
                    })
                    .flatten();
                let grid = corner::fill(&sides, profile_shape.diagonal, pipe.as_ref());
                let mut inside = Vec::with_capacity(grid.inner.len());
                for &step in &grid.inner {
                    let id = self.add_vertex(add(here, scale(step, width)))?;
                    moving.push((id, here, step));
                    inside.push(id);
                }
                for face in &grid.faces {
                    // Faces run the other way round the vertex from the fan's order.
                    bevel_faces.push(
                        face.iter()
                            .rev()
                            .map(|&node| match node {
                                Node::Side(i, k) => ids[i][k],
                                Node::Inner(k) => inside[k],
                            })
                            .collect(),
                    );
                }
            } else if curved {
                // The end of one beveled edge where other edges go on: triangles from
                // the vertex, which stays, to the pieces of the band's end.
                let keep = ring.iter().find_map(|r| match r {
                    Around::Point(Spot::Keep(kept)) => outline.iter().position(|p| p == kept),
                    _ => None,
                });
                match keep {
                    Some(at) => {
                        let n = outline.len();
                        for t in 1..n - 1 {
                            bevel_faces.push(vec![
                                outline[at],
                                outline[(at + t) % n],
                                outline[(at + t + 1) % n],
                            ]);
                        }
                    }
                    None => bevel_faces.push(outline),
                }
            } else {
                bevel_faces.push(outline);
            }
        }

        // Miters: the patch between two bands where they turn a corner, and the arcs.
        let mut miter_list: Vec<VertexId> = miters.keys().copied().collect();
        miter_list.sort_unstable();
        for v in miter_list {
            let m = &miters[&v];
            let end = |edge: EdgeId, outer: bool| {
                let spot = Spot::Miter(v, edge, outer);
                if made.contains_key(&spot) {
                    made[&spot]
                } else if outer {
                    made[&m.outer_point]
                } else {
                    made[&m.inner_point]
                }
            };
            let [e0, e1] = m.edges;
            let (x0, y0, x1, y1) = (end(e0, false), end(e0, true), end(e1, false), end(e1, true));
            let y = made[&m.outer_point];
            // The patch's outline, side by side: the first band's end, the outer
            // side, the second band's end, and the inner side.
            let mut ids: Vec<Vec<VertexId>> = Vec::new();
            let mut sides: Vec<Side> = Vec::new();
            let mut side = |from: VertexId,
                            inner: Vec<VertexId>,
                            to: VertexId,
                            middle: Option<V3>,
                            inner_steps: &HashMap<VertexId, V3>| {
                let mut all = vec![from];
                all.extend(inner);
                all.push(to);
                let points = all
                    .iter()
                    .map(|&p| step_of_point(&step_of, inner_steps, p))
                    .collect();
                sides.push(outline_side(points, middle));
                ids.push(all);
            };
            let col0 = profile(self, &mut moving, &mut inner_steps, v, x0, y0, None)?;
            let middle = profile_shape.middle(step_of[&x0], step_of[&y0], [0.0; 3]);
            side(x0, col0, y0, middle, &inner_steps);
            match m.outer {
                Miter::Sharp => {}
                Miter::Patch => {
                    let first = line(self, &mut moving, &mut inner_steps, v, y0, y)?;
                    side(y0, first, y, None, &inner_steps);
                    let second = line(self, &mut moving, &mut inner_steps, v, y, y1)?;
                    side(y, second, y1, None, &inner_steps);
                }
                Miter::Arc => {
                    let toward = steps[&m.outer_point];
                    let points = arc(self, &mut moving, &mut inner_steps, v, y0, y1, toward)?;
                    // Where faces still reach the old corner point, a fan from it
                    // fills in behind the arc. A single face there follows the arc.
                    if matches!(m.outer_point, Spot::OnEdge(..)) {
                        let mut curve = vec![y0];
                        curve.extend(points.iter().copied());
                        curve.push(y1);
                        for w in curve.windows(2) {
                            pending.push(vec![y, w[0], w[1]]);
                        }
                    }
                    let middle = arc_shape.middle(step_of[&y0], step_of[&y1], toward);
                    side(y0, points, y1, middle, &inner_steps);
                }
            }
            let col1 = profile(self, &mut moving, &mut inner_steps, v, y1, x1, None)?;
            let middle = profile_shape.middle(step_of[&y1], step_of[&x1], [0.0; 3]);
            side(y1, col1, x1, middle, &inner_steps);
            if x1 != x0 {
                let toward = steps[&m.inner_point];
                let points = if m.inner_arc && segments > 1 {
                    arc(self, &mut moving, &mut inner_steps, v, x1, x0, toward)?
                } else {
                    Vec::new()
                };
                let middle = arc_shape.middle(step_of[&x1], step_of[&x0], toward);
                let straight = points.is_empty() && segments > 1;
                side(x1, points, x0, middle, &inner_steps);
                if straight {
                    // Cannot happen with the options as they are: an inner side is
                    // an arc or is not there. Kept so the outline always closes.
                    ids.pop();
                    sides.pop();
                }
            }
            let whole = sides.len() >= 3
                && segments > 1
                && sides.iter().all(|side| side.points.len() == segments + 1);
            if whole {
                // A grid of four-sided faces, as Blender fills a miter.
                let here = self.vertices[v].position;
                let grid = corner::fill(&sides, profile_shape.diagonal, None);
                let mut inside = Vec::with_capacity(grid.inner.len());
                for &step in &grid.inner {
                    let id = self.add_vertex(add(here, scale(step, width)))?;
                    moving.push((id, here, step));
                    inside.push(id);
                }
                for face in &grid.faces {
                    pending.push(
                        face.iter()
                            .map(|&node| match node {
                                Node::Side(i, k) => ids[i][k],
                                Node::Inner(k) => inside[k],
                            })
                            .collect(),
                    );
                }
            } else {
                // One face.
                let mut ring: Vec<VertexId> = ids
                    .iter()
                    .flat_map(|side| side[..side.len() - 1].iter().copied())
                    .collect();
                ring.dedup();
                if ring.len() > 1 && ring.first() == ring.last() {
                    ring.pop();
                }
                if ring.len() >= 3 {
                    pending.push(ring);
                }
            }
        }
        let fixed: Vec<&Vec<VertexId>> = new_faces.iter().chain(bevel_faces.iter()).collect();
        bevel_faces.extend(orient_by_neighbors(&fixed, pending));

        // Out with the old, keeping each face's shading and group for its new outline;
        // the bevel's own faces take the smoothest shading around them.
        let kept: Vec<(Look, u16)> = affected
            .iter()
            .map(|&f| (self.faces[f].look(), self.faces[f].group))
            .collect();
        let bevel_smooth = kept.iter().map(|k| k.0.smooth).max().unwrap_or(0);
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
        for (outline, (smooth, group)) in new_faces.into_iter().zip(kept) {
            let face = self.add_face(&outline).map_err(refused)?;
            self.faces[face].set_look(smooth);
            self.faces[face].group = group;
        }
        let mut selected = Vec::with_capacity(bevel_faces.len());
        for outline in bevel_faces {
            let face = self.add_face(&outline).map_err(refused)?;
            self.faces[face].smooth = bevel_smooth;
            // Blender's Material Index: the bevel's own faces can take a material;
            // otherwise they take a neighbor's when the edit ends.
            self.faces[face].material = settings.material;
            selected.push(Element::Face(face));
        }
        if settings.mark_sharp {
            self.mark_bevel_border_sharp(&selected);
        }
        self.drop_unused(&mut moving)?;
        self.select_elements(SelectMode::Face, &selected, SelectOp::Replace)?;
        self.flush_selection(mode);
        Ok(Bevel {
            moving,
            width,
            max_width,
            per_unit,
        })
    }

    /// The edges between the bevel's faces and the faces around it become sharp.
    fn mark_bevel_border_sharp(&mut self, bevel: &[Element]) {
        let faces: HashSet<FaceId> = bevel
            .iter()
            .filter_map(|e| match e {
                Element::Face(f) => Some(*f),
                _ => None,
            })
            .collect();
        let mut border = Vec::new();
        for &face in &faces {
            for corner in self.face_walk(face) {
                let edge = self.corners[corner].edge;
                if self
                    .radial_walk(edge)
                    .any(|c| !faces.contains(&self.corners[c].face))
                {
                    border.push(edge);
                }
            }
        }
        for edge in border {
            self.edges[edge].sharp = true;
        }
    }

    /// Removes new points that no face ended up using, such as the corner point an
    /// arc miter replaced.
    fn drop_unused(&mut self, moving: &mut Vec<(VertexId, V3, V3)>) -> Result<(), MeshError> {
        let unused: HashSet<VertexId> = moving
            .iter()
            .map(|m| m.0)
            .filter(|&v| self.disk_walk(v).next().is_none())
            .collect();
        for &v in &unused {
            self.remove_vertex(v)?;
        }
        moving.retain(|m| !unused.contains(&m.0));
        Ok(())
    }

    /// How far points move per unit of the width, as its type measures it.
    fn width_per_unit(
        &self,
        width_type: WidthType,
        beveled: &HashSet<EdgeId>,
        sides: &HashMap<EdgeId, [CornerId; 2]>,
        fans: &HashMap<VertexId, Fan>,
        rings: &HashMap<VertexId, Vec<Around>>,
        steps: &HashMap<Spot, V3>,
    ) -> f32 {
        let mean = |values: &[f32]| {
            let good: Vec<f32> = values
                .iter()
                .copied()
                .filter(|v| v.is_finite() && *v > 1e-6)
                .collect();
            (!good.is_empty()).then(|| good.iter().sum::<f32>() / good.len() as f32)
        };
        let mut sorted: Vec<EdgeId> = beveled.iter().copied().collect();
        sorted.sort_unstable();
        match width_type {
            WidthType::Offset => 1.0,
            WidthType::Width | WidthType::Depth => {
                let mut across = Vec::new();
                for &edge in &sorted {
                    for v in self.edges[edge].vertices {
                        let fan = &fans[&v];
                        let [s0, s1] = sides[&edge].map(|corner| {
                            let i = fan
                                .corners
                                .iter()
                                .position(|&c| self.corners[c].face == self.corners[corner].face)
                                .unwrap_or(0);
                            steps
                                .get(&base_spot(fan, v, i, edge, beveled, steps))
                                .copied()
                                .unwrap_or([0.0; 3])
                        });
                        if width_type == WidthType::Width {
                            across.push(length(sub(s0, s1)));
                        } else {
                            let other = self.edges[edge].other_vertex(v).unwrap_or(v);
                            let u = unit(sub(
                                self.vertices[other].position,
                                self.vertices[v].position,
                            ))
                            .unwrap_or([0.0; 3]);
                            let flat = |p: V3| sub(p, scale(u, dot(p, u)));
                            let (p0, p1) = (flat(s0), flat(s1));
                            let chord = sub(p1, p0);
                            let l = length(chord);
                            if l > 1e-9 {
                                across.push(length(cross(p0, chord)) / l);
                            }
                        }
                    }
                }
                mean(&across).map_or(1.0, |m| 1.0 / m)
            }
            WidthType::Percent | WidthType::Absolute => {
                // How far each point travels along the edges next to it, per unit.
                let mut per = Vec::new();
                let mut hubs: Vec<VertexId> = rings.keys().copied().collect();
                hubs.sort_unstable();
                for v in hubs {
                    let fan = &fans[&v];
                    let here = self.vertices[v].position;
                    for around in &rings[&v] {
                        let Around::Point(spot) = around else {
                            continue;
                        };
                        let Some(&step) = steps.get(spot) else {
                            continue;
                        };
                        let edges: Vec<EdgeId> = match *spot {
                            Spot::OnEdge(_, e) => vec![e],
                            Spot::Corner(_, i) => vec![fan.edges[i], fan.next_edge(i)],
                            _ => Vec::new(),
                        };
                        for e in edges {
                            let other = self.edges[e].other_vertex(v).unwrap_or(v);
                            let along = sub(self.vertices[other].position, here);
                            let Some(u) = unit(along) else {
                                continue;
                            };
                            let travel = dot(step, u);
                            if travel > 1e-6 {
                                per.push(if width_type == WidthType::Percent {
                                    length(along) / 100.0 / travel
                                } else {
                                    1.0 / travel
                                });
                            }
                        }
                    }
                }
                mean(&per).unwrap_or(1.0)
            }
        }
    }
}

/// The point where an edge of fan face `face` meets the fan's vertex, before miters.
fn base_spot(
    fan: &Fan,
    v: VertexId,
    face: usize,
    edge: EdgeId,
    beveled: &HashSet<EdgeId>,
    steps: &HashMap<Spot, V3>,
) -> Spot {
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
}

/// Winds each pending face to match the faces it shares an edge with: a shared edge
/// must run opposite ways in the two faces. Faces are settled one after another, so a
/// face can follow one settled just before it.
fn orient_by_neighbors(
    fixed: &[&Vec<VertexId>],
    pending: Vec<Vec<VertexId>>,
) -> Vec<Vec<VertexId>> {
    let mut directed: HashSet<(VertexId, VertexId)> = HashSet::new();
    let add_edges = |directed: &mut HashSet<(VertexId, VertexId)>, face: &[VertexId]| {
        for k in 0..face.len() {
            directed.insert((face[k], face[(k + 1) % face.len()]));
        }
    };
    for face in fixed {
        add_edges(&mut directed, face);
    }
    let mut left = pending;
    let mut done = Vec::with_capacity(left.len());
    loop {
        let before = left.len();
        let mut still = Vec::new();
        for mut face in left {
            let n = face.len();
            let mut verdict = None;
            for k in 0..n {
                let (p, q) = (face[k], face[(k + 1) % n]);
                if directed.contains(&(p, q)) {
                    verdict = Some(true);
                    break;
                }
                if directed.contains(&(q, p)) {
                    verdict = Some(false);
                    break;
                }
            }
            match verdict {
                Some(flip) => {
                    if flip {
                        face.reverse();
                    }
                    add_edges(&mut directed, &face);
                    done.push(face);
                }
                None => still.push(face),
            }
        }
        left = still;
        if left.is_empty() || left.len() == before {
            break;
        }
    }
    done.extend(left);
    done
}

impl Mesh {
    /// Bevels each selected vertex on its own, as Blender's vertex bevel: a point on
    /// every edge at the width from the vertex, joined across each face by a straight
    /// cut or, with more segments, a curve. Where faces surround the vertex, a corner
    /// face fills the gap. At a border, the faces' corners are simply cut off.
    fn bevel_vertices(
        &mut self,
        settings: &BevelSettings,
        profile_shape: &ProfileShape,
        mode: SelectMode,
    ) -> Result<Bevel, MeshError> {
        let BevelSettings {
            width: value,
            segments,
            clamp,
            ..
        } = *settings;
        let hubs = self.selected_vertices();
        if hubs.is_empty() {
            return Err(MeshError::NothingSelected);
        }
        let hub_set: HashSet<VertexId> = hubs.iter().copied().collect();
        let mut fans: HashMap<VertexId, Fan> = HashMap::with_capacity(hubs.len());
        for &v in &hubs {
            fans.insert(v, self.fan(v).ok_or(MeshError::CannotBevel)?);
        }

        // A point may go half way along an edge shared with another beveled vertex,
        // and nearly all the way otherwise.
        let mut max_width = f32::MAX;
        let mut directions: HashMap<(VertexId, EdgeId), V3> = HashMap::new();
        for &v in &hubs {
            let here = self.vertices[v].position;
            for &e in &fans[&v].edges {
                let other = self.edges[e]
                    .other_vertex(v)
                    .ok_or(MeshError::CannotBevel)?;
                let along = sub(self.vertices[other].position, here);
                let dir = unit(along).ok_or(MeshError::CannotBevel)?;
                let share = if hub_set.contains(&other) { 0.5 } else { 1.0 };
                max_width = max_width.min(length(along) * share);
                directions.insert((v, e), dir);
            }
        }
        let max_width = if clamp { max_width * 0.999 } else { UNCLAMPED };
        // How far points move per unit of the width, as its type measures it. Each
        // point moves straight along its edge.
        let mean = |values: Vec<f32>| {
            let good: Vec<f32> = values
                .into_iter()
                .filter(|v| v.is_finite() && *v > 1e-6)
                .collect();
            (!good.is_empty()).then(|| good.iter().sum::<f32>() / good.len() as f32)
        };
        let mut lengths = Vec::new();
        let mut across = Vec::new();
        let mut depths = Vec::new();
        for &v in &hubs {
            let fan = &fans[&v];
            let here = self.vertices[v].position;
            for &e in &fan.edges {
                if let Some(other) = self.edges[e].other_vertex(v) {
                    lengths.push(length(sub(self.vertices[other].position, here)));
                }
            }
            for i in 0..fan.corners.len() {
                let (a, b) = (
                    directions[&(v, fan.edges[i])],
                    directions[&(v, fan.next_edge(i))],
                );
                let chord = sub(b, a);
                across.push(length(chord));
                if length(chord) > 1e-9 {
                    depths.push(length(cross(a, chord)) / length(chord));
                }
            }
        }
        let per_unit = match settings.width_type {
            WidthType::Offset | WidthType::Absolute => 1.0,
            WidthType::Percent => mean(lengths).map_or(1.0, |l| l / 100.0),
            WidthType::Width => mean(across).map_or(1.0, |w| 1.0 / w),
            WidthType::Depth => mean(depths).map_or(1.0, |d| 1.0 / d),
        };
        let width = (value * per_unit).min(max_width);

        let mut moving: Vec<(VertexId, V3, V3)> = Vec::new();
        let mut made: HashMap<(VertexId, EdgeId), VertexId> = HashMap::new();
        let mut step_of: HashMap<VertexId, V3> = HashMap::new();
        for &v in &hubs {
            let here = self.vertices[v].position;
            for &e in &fans[&v].edges {
                let step = directions[&(v, e)];
                let id = self.add_vertex(add(here, scale(step, width)))?;
                made.insert((v, e), id);
                step_of.insert(id, step);
                moving.push((id, here, step));
            }
        }

        // Curves across each face's corner, bending toward the vertex.
        let mut profiles: HashMap<(VertexId, VertexId, VertexId), Vec<VertexId>> = HashMap::new();
        // Steps of the curves' inner points, for filling corners.
        let mut inner_steps: HashMap<VertexId, V3> = HashMap::new();
        let mut curve = |mesh: &mut Mesh,
                         moving: &mut Vec<(VertexId, V3, V3)>,
                         inner_steps: &mut HashMap<VertexId, V3>,
                         v: VertexId,
                         from: VertexId,
                         to: VertexId|
         -> Result<Vec<VertexId>, MeshError> {
            let key = (v, from.min(to), from.max(to));
            if let Entry::Vacant(slot) = profiles.entry(key) {
                let here = mesh.vertices[v].position;
                let mut inner = Vec::new();
                let (a, c) = (step_of[&key.1], step_of[&key.2]);
                let points = if first_is_a(a, c) {
                    profile_shape.steps(a, c, [0.0; 3])
                } else {
                    let mut back = profile_shape.steps(c, a, [0.0; 3]);
                    back.reverse();
                    back
                };
                for step in points {
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

        let mut affected: Vec<FaceId> = hubs
            .iter()
            .flat_map(|v| fans[v].corners.iter().map(|&c| self.corners[c].face))
            .collect();
        affected.sort_unstable();
        affected.dedup();
        let mut new_faces: Vec<Vec<VertexId>> = Vec::with_capacity(affected.len());
        for &face in &affected {
            let corners: Vec<CornerId> = self.face_walk(face).collect();
            let mut outline = Vec::with_capacity(corners.len() * 2);
            for c in corners {
                let data = self.corners[c];
                let v = data.vertex;
                if !hub_set.contains(&v) {
                    outline.push(v);
                    continue;
                }
                let a = made[&(v, self.corners[data.prev].edge)];
                let b = made[&(v, data.edge)];
                outline.push(a);
                outline.extend(curve(self, &mut moving, &mut inner_steps, v, a, b)?);
                outline.push(b);
            }
            new_faces.push(outline);
        }

        // Corner faces, where faces go all the way round.
        let mut cap_faces: Vec<Vec<VertexId>> = Vec::new();
        for &v in &hubs {
            let fan = &fans[&v];
            if !fan.closed {
                continue;
            }
            let here = self.vertices[v].position;
            let n = fan.edges.len();
            let mut outline = Vec::new();
            let mut ids: Vec<Vec<VertexId>> = Vec::with_capacity(n);
            let mut sides: Vec<Side> = Vec::with_capacity(n);
            for i in 0..n {
                let a = made[&(v, fan.edges[i])];
                let b = made[&(v, fan.edges[(i + 1) % n])];
                let inner = curve(self, &mut moving, &mut inner_steps, v, a, b)?;
                outline.push(a);
                outline.extend(inner.iter().copied());
                let mut side = vec![a];
                side.extend(inner);
                side.push(b);
                let points = side
                    .iter()
                    .map(|&p| step_of_point(&step_of, &inner_steps, p))
                    .collect();
                let middle = profile_shape.middle(step_of[&a], step_of[&b], [0.0; 3]);
                sides.push(outline_side(points, middle));
                ids.push(side);
            }
            if segments > 1 {
                // A grid of four-sided faces, as in Blender.
                let grid = corner::fill(&sides, profile_shape.diagonal, None);
                let mut inside = Vec::with_capacity(grid.inner.len());
                for &step in &grid.inner {
                    let id = self.add_vertex(add(here, scale(step, width)))?;
                    moving.push((id, here, step));
                    inside.push(id);
                }
                for face in &grid.faces {
                    // Faces run the other way round the vertex from the fan's order.
                    cap_faces.push(
                        face.iter()
                            .rev()
                            .map(|&node| match node {
                                Node::Side(i, k) => ids[i][k],
                                Node::Inner(k) => inside[k],
                            })
                            .collect(),
                    );
                }
            } else {
                // Faces run the other way round the vertex from the fan's order.
                outline.reverse();
                cap_faces.push(outline);
            }
        }

        let kept: Vec<(Look, u16)> = affected
            .iter()
            .map(|&f| (self.faces[f].look(), self.faces[f].group))
            .collect();
        let bevel_smooth = kept.iter().map(|k| k.0.smooth).max().unwrap_or(0);
        for &face in &affected {
            self.remove_face(face)?;
        }
        for &v in &hubs {
            self.remove_vertex(v)?;
        }
        let refused = |error: MeshError| match error {
            MeshError::FaceExists(_)
            | MeshError::RepeatedVertex(_)
            | MeshError::FaceTooSmall { .. } => MeshError::CannotBevel,
            other => other,
        };
        for (outline, (smooth, group)) in new_faces.into_iter().zip(kept) {
            let face = self.add_face(&outline).map_err(refused)?;
            self.faces[face].set_look(smooth);
            self.faces[face].group = group;
        }
        let mut selected = Vec::with_capacity(cap_faces.len());
        for outline in cap_faces {
            let face = self.add_face(&outline).map_err(refused)?;
            self.faces[face].smooth = bevel_smooth;
            // Blender's Material Index: the bevel's own faces can take a material;
            // otherwise they take a neighbor's when the edit ends.
            self.faces[face].material = settings.material;
            selected.push(Element::Face(face));
        }
        if settings.mark_sharp {
            self.mark_bevel_border_sharp(&selected);
        }
        self.select_elements(SelectMode::Face, &selected, SelectOp::Replace)?;
        self.flush_selection(mode);
        Ok(Bevel {
            moving,
            width,
            max_width,
            per_unit,
        })
    }
}
