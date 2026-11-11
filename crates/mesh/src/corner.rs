//! The grid of faces that fills a bevel's corner.
//!
//! Where three or more of a bevel's points surround a vertex, the gap between them is
//! an outline with one side per pair of neighboring points, every side cut into the
//! bevel's number of segments. This module fills such an outline the way Blender's
//! bevel does: with four-sided faces only, laid out as one small grid per corner
//! point.
//!
//! - With an even number of segments, the grids meet at one point in the middle.
//! - With an odd number, they stop short of the middle, where one face with as many
//!   corners as the outline has sides closes the gap, with a strip of faces between
//!   each two neighboring grids.
//!
//! # Where the points go
//!
//! The outline is split into one four-cornered *sector* per corner point: from the
//! corner point along its side to the side's middle, in to the patch's center, out to
//! the previous side's middle, and back along that side. Each sector is filled as a
//! Coons patch, which blends its four edges. The spokes from the sides' middles to
//! the center are curved the way the bevel's profile is, so a sharp profile's corner
//! keeps its straight edges and a round one's swells evenly.
//!
//! Done with plain positions, that makes a corner that is too flat for a round bevel.
//! So the patch is also worked out around a *center of curvature*: every point is
//! taken as a direction and a distance from it, and the distances are blended instead.
//! When the outline lies on a sphere, as on a cube's corner beveled round, so does
//! every point inside. How much of each answer is used depends on the profile: all
//! sphere for a round one, all plain for a straight or a sharp one, whose corners are
//! flat.
//!
//! Everything here is in the bevel's own units: positions are steps from the vertex
//! per unit of width, so the vertex is at the origin.

use std::collections::HashMap;
use std::sync::Mutex;

type V3 = [f32; 3];

/// One side of the outline, from one corner point to the next.
#[derive(Debug, Clone)]
pub(crate) struct Side {
    /// Its points in order, both ends included: one more than the bevel has segments.
    pub points: Vec<V3>,
    /// The middle of the curve the points lie on. With an even number of segments
    /// this is its middle point; with an odd number no point sits there.
    pub middle: V3,
}

/// A straight run of bevel passing through a corner: two of the corner's beveled
/// edges lie in one line, as where a rounded edge is crossed by another. The corner is
/// then a piece of that run, a "pipe", and its points are put on the pipe's surface.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Pipe {
    /// The direction of the two edges.
    pub along: V3,
    /// The profile across the pipe: the points `origin + x * across[0] + y * across[1]`
    /// with `x^r + y^r = 1`, for the exponent `r`.
    pub origin: V3,
    pub across: [V3; 2],
    pub exponent: f32,
}

impl Pipe {
    /// The point of the pipe's surface level with `p`: on the same ray from the
    /// pipe's middle line. `None` where there is no such ray.
    fn onto(&self, p: V3) -> Option<V3> {
        let [a, b] = self.across;
        let cross = |u: V3, v: V3| {
            [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ]
        };
        let dot = |u: V3, v: V3| u[0] * v[0] + u[1] * v[1] + u[2] * v[2];
        // `p - origin` as `x * a + y * b + t * along`.
        let det = dot(a, cross(b, self.along));
        if det.abs() < 1e-9 {
            return None;
        }
        let q = sub(p, self.origin);
        let x = dot(q, cross(b, self.along)) / det;
        let y = dot(a, cross(q, self.along)) / det;
        let t = dot(a, cross(b, q)) / det;
        let r = self.exponent;
        let size = (x.abs().powf(r) + y.abs().powf(r)).powf(1.0 / r);
        if !size.is_finite() || size < 1e-6 {
            return None;
        }
        Some(add(
            self.origin,
            add(
                add(scale(a, x / size), scale(b, y / size)),
                scale(self.along, t),
            ),
        ))
    }
}

/// A point of the filled corner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Node {
    /// On the outline: the side, and which of its points. A side's last point is
    /// always given as the next side's first.
    Side(usize, usize),
    /// A new point inside, by its index in `Grid::inner`.
    Inner(usize),
}

/// A filled corner: its new points and its faces.
#[derive(Debug, Clone)]
pub(crate) struct Grid {
    pub inner: Vec<V3>,
    /// Each face's corners in order. Faces wind the way the outline runs: a face on a
    /// side runs along it from that side's first point toward its last.
    pub faces: Vec<Vec<Node>>,
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
fn length(a: V3) -> f32 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}
fn lerp(a: V3, b: V3, t: f32) -> V3 {
    add(scale(a, 1.0 - t), scale(b, t))
}

/// How much of the sphere-like answer to use: all of it for a round profile, and
/// none once the profile is some way from round, where blending distances would
/// bend what should be flat.
fn roundness(diagonal: f32) -> f32 {
    let t = (diagonal - std::f32::consts::FRAC_1_SQRT_2) / 0.1;
    (1.0 - t * t).clamp(0.0, 1.0)
}

/// Fills the outline made by `sides`. All sides have the same number of points, at
/// least three (two segments), and there are at least three sides. `diagonal` says
/// where the bevel's profile crosses its frame's diagonal: 0.5 for a straight
/// profile, 0.71 for a round one, 1 for a sharp one.
///
/// With a `pipe`, the points inside are put on its surface, but only if the outline
/// already lies on it. Otherwise the pipe is left out, since the fill would not meet
/// its own outline.
pub(crate) fn fill(sides: &[Side], diagonal: f32, pipe: Option<&Pipe>) -> Grid {
    let n = sides.len();
    let s = sides[0].points.len() - 1;
    debug_assert!(n >= 3 && s >= 2);
    debug_assert!(sides.iter().all(|side| side.points.len() == s + 1));
    let h = s / 2;
    let odd = s % 2 == 1;
    let before = |i: usize| (i + n - 1) % n;

    let corner = |i: usize| sides[i].points[0];
    let middle = |i: usize| {
        if odd {
            sides[i].middle
        } else {
            sides[i].points[h]
        }
    };

    // The patch's center: from the middle of the corner points toward the vertex.
    let mut centroid = [0.0; 3];
    for i in 0..n {
        centroid = add(centroid, corner(i));
    }
    let centroid = scale(centroid, 1.0 / n as f32);
    // The profile as the curve `x^r + y^r = 1` crossing the diagonal where it does,
    // kept away from the two ends where the sums below would divide by nothing.
    let d = diagonal.clamp(0.07, 0.989);
    let r = -std::f32::consts::LN_2 / d.ln();
    // Where the surface `x^r + y^r + z^r = 1` crosses a cube's diagonal. On a cube's
    // corner the corner points' middle is at 1/3 of the diagonal and the vertex at 1,
    // which gives how far the patch's center sits from that middle toward the vertex.
    let c = d.powf(3f32.log2());
    let full = (3.0 * c - 1.0) / 2.0;
    let center = scale(centroid, 1.0 - full);

    // A spoke runs from a side's middle `m` to the center. On that surface it is the
    // curve `2 t^r + z^r = 1` in the plane through the side's middle `(d, 0)`, the
    // center `(c, c)` and the vertex `(1, 1)`. Here it is the same curve in the plane
    // through the real three, so it starts out along the side's own edge and bends
    // in to the center as the profile does. `w` runs from 0 at `m` to 1.
    let spoke_at = |m: V3, w: f32| -> V3 {
        let z = c * w.clamp(0.0, 1.0);
        let t = ((1.0 - z.powf(r)) / 2.0).max(0.0).powf(1.0 / r);
        let on_m = (t - z) / d;
        let on_center = (1.0 - on_m - z) / (1.0 - c);
        // The rest is on the vertex, which is the origin.
        add(scale(m, on_m), scale(center, on_center))
    };

    // The center of curvature: of the sphere through the corner points (taken as a
    // circle around their middle) and the patch's center.
    let rise = full * length(centroid);
    let mut around = 0.0;
    for i in 0..n {
        around += length(sub(corner(i), centroid)) / n as f32;
    }
    let share = roundness(diagonal);
    let pole = (share > 1e-4 && rise.abs() > 1e-3 * around.max(1e-6) && length(centroid) > 1e-6)
        .then(|| {
            let radius = (around * around + rise * rise) / (2.0 * rise);
            // Toward the vertex, which is at the origin.
            let toward = scale(centroid, -1.0 / length(centroid));
            sub(center, scale(toward, radius))
        });

    // One point of sector `i`: `a` steps from its corner along its side, `b` steps
    // along the side before it.
    let place = |i: usize, a: usize, b: usize| -> V3 {
        let (u, v) = (2.0 * a as f32 / s as f32, 2.0 * b as f32 / s as f32);
        let p = corner(i);
        let (m, m_before) = (middle(i), middle(before(i)));
        // With an even count, the last step along a side is its middle.
        let bottom = if !odd && a == h {
            m
        } else {
            sides[i].points[a]
        };
        let left = if !odd && b == h {
            m_before
        } else {
            sides[before(i)].points[s - b]
        };
        // Each term of the Coons patch with its weight. The spokes run from the
        // sides' middles to the center.
        let spoke = spoke_at(m, v);
        let spoke_before = spoke_at(m_before, u);
        let plain = {
            let edges = add(
                add(scale(bottom, 1.0 - v), scale(spoke_before, v)),
                add(scale(left, 1.0 - u), scale(spoke, u)),
            );
            let corners = add(
                add(scale(p, (1.0 - u) * (1.0 - v)), scale(m, u * (1.0 - v))),
                add(scale(m_before, (1.0 - u) * v), scale(center, u * v)),
            );
            sub(edges, corners)
        };
        let Some(pole) = pole else {
            return plain;
        };
        // The same blend of the distances from the center of curvature.
        let far = |x: V3| length(sub(x, pole));
        let (far_m, far_before, far_center) = (far(m), far(m_before), far(center));
        let distance = far(bottom) * (1.0 - v)
            + far(spoke_before) * v
            + far(left) * (1.0 - u)
            + far(spoke) * u
            - (far(p) * (1.0 - u) * (1.0 - v)
                + far_m * u * (1.0 - v)
                + far_before * (1.0 - u) * v
                + far_center * u * v);
        let out = sub(plain, pole);
        let l = length(out);
        if l < 1e-9 || !distance.is_finite() {
            return plain;
        }
        let round = add(pole, scale(out, distance / l));
        lerp(plain, round, share)
    };

    let pipe = pipe.filter(|pipe| {
        let size = sides
            .iter()
            .flat_map(|side| side.points.iter())
            .map(|p| length(sub(*p, pipe.origin)))
            .fold(0.0f32, f32::max);
        sides.iter().flat_map(|side| side.points.iter()).all(|&p| {
            pipe.onto(p)
                .is_some_and(|on| length(sub(on, p)) < 2e-3 * size.max(1e-6))
        })
    });
    let place = |i: usize, a: usize, b: usize| -> V3 {
        let p = place(i, a, b);
        pipe.and_then(|pipe| pipe.onto(p)).unwrap_or(p)
    };

    let mut inner: Vec<V3> = Vec::new();
    let mut made: HashMap<(usize, usize, usize), usize> = HashMap::new();
    let mut node = |i: usize, a: usize, b: usize| -> Node {
        if b == 0 {
            return Node::Side(i, a);
        }
        if a == 0 {
            return Node::Side(before(i), s - b);
        }
        // With an even count the last column of a sector is the next one's last row,
        // and every sector ends at the one center point.
        let key = if !odd && a == h {
            if b == h {
                (0, h, h)
            } else {
                ((i + 1) % n, b, h)
            }
        } else {
            (i, a, b)
        };
        let index = *made.entry(key).or_insert_with(|| {
            inner.push(place(key.0, key.1, key.2));
            inner.len() - 1
        });
        Node::Inner(index)
    };

    let mut faces: Vec<Vec<Node>> = Vec::new();
    for i in 0..n {
        for b in 0..h {
            for a in 0..h {
                faces.push(vec![
                    node(i, a, b),
                    node(i, a + 1, b),
                    node(i, a + 1, b + 1),
                    node(i, a, b + 1),
                ]);
            }
        }
        if odd {
            // The strip between this sector and the next.
            let next = (i + 1) % n;
            for b in 0..h {
                faces.push(vec![
                    node(i, h, b),
                    node(next, b, h),
                    node(next, b + 1, h),
                    node(i, h, b + 1),
                ]);
            }
        }
    }
    if odd {
        if h == 0 {
            // One segment: the outline itself.
            faces.push((0..n).map(|i| Node::Side(i, 0)).collect());
        } else {
            faces.push((0..n).map(|i| node(i, h, h)).collect());
        }
    }
    Grid { inner, faces }
}

/// Points along the curve `x^r + y^r = 1` from `(1, 0)` to `(0, 1)`, `segments + 1` of
/// them with both ends, spaced so that the straight pieces between them are all the
/// same length, as Blender spaces a bevel's profile.
pub(crate) fn even_superellipse(r: f32, segments: usize) -> Vec<[f32; 2]> {
    // Finding the spacing takes a search, and a drag asks for the same one again and
    // again, so the answers are kept.
    type Spacings = HashMap<(u32, usize), Vec<[f32; 2]>>;
    static KEPT: Mutex<Option<Spacings>> = Mutex::new(None);
    let key = (r.to_bits(), segments);
    if let Ok(kept) = KEPT.lock()
        && let Some(points) = kept.as_ref().and_then(|map| map.get(&key))
    {
        return points.clone();
    }
    let points = spaced_superellipse(r, segments);
    if let Ok(mut kept) = KEPT.lock() {
        let map = kept.get_or_insert_with(HashMap::new);
        if map.len() >= 512 {
            map.clear();
        }
        map.insert(key, points.clone());
    }
    points
}

fn spaced_superellipse(r: f32, segments: usize) -> Vec<[f32; 2]> {
    let n = segments.max(1);
    let r = r as f64;
    let with_ends = |mut points: Vec<[f32; 2]>| {
        points[0] = [1.0, 0.0];
        points[n] = [0.0, 1.0];
        points
    };
    if (r - 2.0).abs() < 1e-6 {
        // A circle: equal angles.
        return with_ends(
            (0..=n)
                .map(|j| {
                    let angle = j as f64 / n as f64 * std::f64::consts::FRAC_PI_2;
                    [angle.cos() as f32, angle.sin() as f32]
                })
                .collect(),
        );
    }
    if (r - 1.0).abs() < 1e-6 {
        return with_ends(
            (0..=n)
                .map(|j| {
                    let t = j as f32 / n as f32;
                    [1.0 - t, t]
                })
                .collect(),
        );
    }
    // The curve from 0 to 1, by whichever coordinate changes steadily: near its ends
    // a bulging curve runs straight up or across, and a hollow one the other way, so
    // going by angle would squeeze nearly all of a sharp profile into no room at all.
    let d = 0.5f64.powf(1.0 / r);
    let other = |c: f64| (1.0 - c.clamp(0.0, 1.0).powf(r)).max(0.0).powf(1.0 / r);
    let at = |t: f64| {
        let t = t.clamp(0.0, 1.0);
        let first = t <= 0.5;
        // How far along its half, from the end toward the diagonal.
        let along = if first { 2.0 * t } else { 2.0 * (1.0 - t) };
        let (near, far) = if r >= 1.0 {
            let small = along * d;
            (other(small), small)
        } else {
            let large = 1.0 - along * (1.0 - d);
            (large, other(large))
        };
        if first { [near, far] } else { [far, near] }
    };
    let apart = |p: [f64; 2], q: [f64; 2]| ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt();
    // Steps of one length along the curve. Returns where each lands, and how far the
    // last one is from the end: negative if the steps were too long to all fit.
    let walk = |step: f64| -> (Vec<f64>, f64) {
        let mut stops = vec![0.0f64];
        let mut t = 0.0f64;
        for _ in 0..n {
            let from = at(t);
            if apart(from, at(1.0)) < step {
                return (stops, -1.0);
            }
            // The curve keeps moving away from `from`, so the distance only grows.
            let (mut low, mut high) = (t, 1.0);
            for _ in 0..48 {
                let mid = (low + high) / 2.0;
                if apart(from, at(mid)) < step {
                    low = mid;
                } else {
                    high = mid;
                }
            }
            t = high;
            stops.push(t);
        }
        (stops, 1.0 - t)
    };
    let (mut low, mut high) = (0.0f64, 2.0f64);
    for _ in 0..56 {
        let mid = (low + high) / 2.0;
        if walk(mid).1 < 0.0 {
            high = mid;
        } else {
            low = mid;
        }
    }
    let (stops, _) = walk(low);
    with_ends(
        (0..=n)
            .map(|j| {
                let p = at(stops.get(j).copied().unwrap_or(1.0));
                [p[0] as f32, p[1] as f32]
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// The outline of a cube's corner beveled round by 1: three quarter circles on the
    /// sphere of radius 1 around (-1, -1, -1), with the vertex at the origin.
    fn cube_corner(s: usize) -> Vec<Side> {
        let o = [-1.0f32, -1.0, -1.0];
        let axis = |i: usize| {
            let mut a = [0.0f32; 3];
            a[i] = 1.0;
            a
        };
        (0..3)
            .map(|i| {
                let (from, to) = (axis(i), axis((i + 1) % 3));
                let at = |t: f32| {
                    let angle = t * std::f32::consts::FRAC_PI_2;
                    add(o, add(scale(from, angle.cos()), scale(to, angle.sin())))
                };
                Side {
                    points: (0..=s).map(|k| at(k as f32 / s as f32)).collect(),
                    middle: at(0.5),
                }
            })
            .collect()
    }

    fn position(sides: &[Side], grid: &Grid, node: Node) -> V3 {
        match node {
            Node::Side(i, k) => sides[i].points[k],
            Node::Inner(k) => grid.inner[k],
        }
    }

    #[test]
    fn a_round_cube_corner_is_quads_on_its_sphere() {
        for s in 2..=9 {
            let sides = cube_corner(s);
            let grid = fill(&sides, std::f32::consts::FRAC_1_SQRT_2, None);
            let h = s / 2;
            if s % 2 == 0 {
                assert_eq!(grid.faces.len(), 3 * h * h, "{s}");
                assert!(grid.faces.iter().all(|f| f.len() == 4));
            } else {
                assert_eq!(grid.faces.len(), 3 * h * (h + 1) + 1, "{s}");
                assert_eq!(grid.faces.iter().filter(|f| f.len() == 3).count(), 1);
                assert_eq!(
                    grid.faces.iter().filter(|f| f.len() == 4).count(),
                    3 * h * (h + 1)
                );
            }
            for p in &grid.inner {
                let r = length(sub(*p, [-1.0, -1.0, -1.0]));
                assert!((r - 1.0).abs() < 1e-4, "{s}: {p:?} is {r} from the center");
            }
            // Every edge inside is used once each way; outline edges once, forward.
            let mut directed: HashSet<(Node, Node)> = HashSet::new();
            for face in &grid.faces {
                for k in 0..face.len() {
                    assert!(directed.insert((face[k], face[(k + 1) % face.len()])));
                }
            }
            let on_outline = |n: Node| matches!(n, Node::Side(..));
            let mut outline = 0;
            for &(p, q) in &directed {
                if directed.contains(&(q, p)) {
                    continue;
                }
                assert!(on_outline(p) && on_outline(q), "{s}: a hole inside");
                let (Node::Side(i, k), Node::Side(j, l)) = (p, q) else {
                    unreachable!()
                };
                // Forward along a side: to its next point, or on to the next side.
                assert!((i == j && l == k + 1) || (j == (i + 1) % 3 && l == 0 && k == s - 1));
                outline += 1;
            }
            assert_eq!(outline, 3 * s);
            // Faces face outward: away from the sphere's center.
            for face in &grid.faces {
                let p: Vec<V3> = face.iter().map(|&n| position(&sides, &grid, n)).collect();
                let mut normal = [0.0f32; 3];
                let mut middle = [0.0f32; 3];
                for k in 0..p.len() {
                    let (a, b) = (p[k], p[(k + 1) % p.len()]);
                    normal[0] += (a[1] - b[1]) * (a[2] + b[2]);
                    normal[1] += (a[2] - b[2]) * (a[0] + b[0]);
                    normal[2] += (a[0] - b[0]) * (a[1] + b[1]);
                    middle = add(middle, scale(a, 1.0 / p.len() as f32));
                }
                let out = sub(middle, [-1.0, -1.0, -1.0]);
                let facing = normal[0] * out[0] + normal[1] * out[1] + normal[2] * out[2];
                // The outline runs x, y, z, which is counterclockwise seen from outside.
                assert!(facing > 0.0, "{s}: a face is turned over");
            }
        }
    }

    /// A cube's corner beveled by 1 with the profile `x^r + y^r = 1`: its true surface
    /// is `x^r + y^r + z^r = 1` around (-1, -1, -1).
    fn shaped_corner(r: f32, s: usize) -> Vec<Side> {
        let profile = even_superellipse(r, s);
        let d = 0.5f32.powf(1.0 / r);
        (0..3)
            .map(|i| {
                let at = |p: [f32; 2]| {
                    let mut q = [-1.0f32; 3];
                    q[i] += p[0];
                    q[(i + 1) % 3] += p[1];
                    q
                };
                Side {
                    points: profile.iter().map(|&p| at(p)).collect(),
                    middle: at([d, d]),
                }
            })
            .collect()
    }

    #[test]
    fn other_shapes_stay_close_to_their_true_surface() {
        for (r, slack) in [
            (1.0f32, 1e-5),
            (2.0, 1e-4),
            (1.5, 0.01),
            (3.0, 0.03),
            (6.0, 0.03),
            (40.0, 0.01),
            (0.6, 0.03),
        ] {
            for s in [2, 3, 4, 5, 8] {
                let d = 0.5f32.powf(1.0 / r);
                let grid = fill(&shaped_corner(r, s), d, None);
                let mut worst = 0.0f32;
                for p in &grid.inner {
                    let q = p.map(|c| (c + 1.0).max(0.0));
                    let on = (q[0].powf(r) + q[1].powf(r) + q[2].powf(r)).powf(1.0 / r);
                    // How far along its own ray the point is from the surface.
                    worst = worst.max((on - 1.0).abs() * length(q) / on);
                }
                assert!(worst < slack, "r {r}, {s} segments: off by {worst}");
            }
        }
    }

    #[test]
    fn a_crossing_on_a_round_edge_stays_on_the_edges_cylinder() {
        // A round edge along z, radius 1 around the line x = y = -1, crossed at the
        // vertex by flat edges: the outline is two quarter circles, at z = 1 and
        // z = -1, joined by two straight lines.
        let arc = |z: f32, t: f32| {
            let angle = t * std::f32::consts::FRAC_PI_2;
            [-1.0 + angle.cos(), -1.0 + angle.sin(), z]
        };
        for s in [2, 3, 4, 6] {
            let line = |from: V3, to: V3| Side {
                points: (0..=s)
                    .map(|k| lerp(from, to, k as f32 / s as f32))
                    .collect(),
                middle: lerp(from, to, 0.5),
            };
            let curve = |z: f32, back: bool| {
                let at = |k: usize| {
                    let t = k as f32 / s as f32;
                    arc(z, if back { 1.0 - t } else { t })
                };
                Side {
                    points: (0..=s).map(at).collect(),
                    middle: arc(z, 0.5),
                }
            };
            let sides = vec![
                curve(1.0, false),
                line(arc(1.0, 1.0), arc(-1.0, 1.0)),
                curve(-1.0, true),
                line(arc(-1.0, 0.0), arc(1.0, 0.0)),
            ];
            let pipe = Pipe {
                along: [0.0, 0.0, 1.0],
                origin: [-1.0, -1.0, 0.0],
                across: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                exponent: 2.0,
            };
            let on_it = |grid: &Grid| {
                grid.inner
                    .iter()
                    .map(|p| (((p[0] + 1.0).powi(2) + (p[1] + 1.0).powi(2)).sqrt() - 1.0).abs())
                    .fold(0.0f32, f32::max)
            };
            let piped = fill(&sides, std::f32::consts::FRAC_1_SQRT_2, Some(&pipe));
            assert!(on_it(&piped) < 1e-5, "{s}: {}", on_it(&piped));
            assert!(piped.inner.iter().all(|p| p[2].abs() < 1.0));
            // A pipe the outline is not on is left out.
            let wrong = Pipe {
                origin: [-1.0, -1.5, 0.0],
                ..pipe
            };
            let plain = fill(&sides, std::f32::consts::FRAC_1_SQRT_2, None);
            let ignored = fill(&sides, std::f32::consts::FRAC_1_SQRT_2, Some(&wrong));
            assert_eq!(plain.inner, ignored.inner);
        }
    }

    #[test]
    fn a_flat_outline_stays_flat_and_inside_itself() {
        // A square in the plane z = 0 with straight sides and the vertex at its middle.
        let corners = [
            [-1.0f32, -1.0, 0.0],
            [1.0, -1.0, 0.0],
            [1.0, 1.0, 0.0],
            [-1.0, 1.0, 0.0],
        ];
        for s in [2, 3, 4, 7] {
            let sides: Vec<Side> = (0..4)
                .map(|i| {
                    let (a, b) = (corners[i], corners[(i + 1) % 4]);
                    Side {
                        points: (0..=s).map(|k| lerp(a, b, k as f32 / s as f32)).collect(),
                        middle: lerp(a, b, 0.5),
                    }
                })
                .collect();
            let grid = fill(&sides, 0.5, None);
            for p in &grid.inner {
                assert!(p[2].abs() < 1e-6 && p[0].abs() < 1.0 && p[1].abs() < 1.0);
            }
            // An even count on a square is a plain grid.
            if s == 4 {
                assert_eq!(grid.faces.len(), 16);
                let h = 2.0 / s as f32;
                for p in &grid.inner {
                    for c in &p[..2] {
                        let steps = (c + 1.0) / h;
                        assert!((steps - steps.round()).abs() < 1e-5, "{p:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn profiles_are_cut_into_pieces_of_one_length() {
        for (r, n) in [
            (2.0f32, 4),
            (1.0, 3),
            (4.0, 5),
            (0.5, 6),
            (64.0, 4),
            (0.25, 3),
            (8.0, 7),
        ] {
            let points = even_superellipse(r, n);
            assert_eq!(points.len(), n + 1);
            assert_eq!((points[0], points[n]), ([1.0, 0.0], [0.0, 1.0]));
            let lengths: Vec<f32> = points
                .windows(2)
                .map(|w| ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt())
                .collect();
            for l in &lengths {
                assert!((l - lengths[0]).abs() < 2e-4, "{r} {n}: {lengths:?}");
            }
            for p in &points[1..n] {
                let on = p[0].powf(r) + p[1].powf(r);
                assert!((on - 1.0).abs() < 2e-3, "{r} {n}: {p:?} gives {on}");
            }
            // The same from either end.
            for j in 0..=n {
                let (p, q) = (points[j], points[n - j]);
                assert!((p[0] - q[1]).abs() < 2e-4 && (p[1] - q[0]).abs() < 2e-4);
            }
        }
    }
}
