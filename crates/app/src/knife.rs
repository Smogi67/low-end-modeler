//! Where a knife path drawn on screen crosses the mesh. No graphics code, so it is
//! tested without a window: what can be seen is passed in.

use std::collections::HashMap;

use modeler_mesh::{CutPoint, Mesh, VertexId};

use crate::picking::ViewInfo;

/// A click this close to a corner cuts through the corner.
pub const SNAP_PIXELS: f32 = 10.0;
/// A crossing this close to an edge's end counts as the corner.
const END_PIXELS: f32 = 3.0;

/// The path's points in the mesh, and where each is on screen.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct KnifeCut {
    pub points: Vec<CutPoint>,
    pub screen: Vec<[f32; 2]>,
}

fn screen(view: &ViewInfo, p: [f32; 3]) -> Option<[f32; 2]> {
    view.project(p).map(|s| [s[0], s[1]])
}

fn distance(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// Where two screen segments cross: how far along each, from 0 to 1.
fn crossing(p0: [f32; 2], p1: [f32; 2], q0: [f32; 2], q1: [f32; 2]) -> Option<(f32, f32)> {
    let r = [p1[0] - p0[0], p1[1] - p0[1]];
    let s = [q1[0] - q0[0], q1[1] - q0[1]];
    let denominator = r[0] * s[1] - r[1] * s[0];
    if denominator.abs() < 1e-9 {
        return None;
    }
    let d = [q0[0] - p0[0], q0[1] - p0[1]];
    let u = (d[0] * s[1] - d[1] * s[0]) / denominator;
    let v = (d[0] * r[1] - d[1] * r[0]) / denominator;
    ((0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v)).then_some((u, v))
}

/// The spot on a 3D edge that appears `along` of the way along it on screen. In
/// perspective these differ, so this searches for it.
fn factor_for(view: &ViewInfo, a: [f32; 3], b: [f32; 3], along: f32) -> f32 {
    let (Some(sa), Some(sb)) = (screen(view, a), screen(view, b)) else {
        return along;
    };
    let d = [sb[0] - sa[0], sb[1] - sa[1]];
    let squared = d[0] * d[0] + d[1] * d[1];
    if squared < 1e-9 {
        return along;
    }
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..32 {
        let mid = (lo + hi) / 2.0;
        let p = std::array::from_fn(|i| a[i] + (b[i] - a[i]) * mid);
        let Some(sp) = screen(view, p) else {
            return along;
        };
        let progress = ((sp[0] - sa[0]) * d[0] + (sp[1] - sa[1]) * d[1]) / squared;
        if progress < along {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (lo + hi) / 2.0
}

/// The corner nearest a click, if one is close enough and can be seen.
pub fn snapped_vertex(
    mesh: &Mesh,
    view: &ViewInfo,
    click: [f32; 2],
    visible: &dyn Fn([f32; 3]) -> bool,
) -> Option<(VertexId, [f32; 2])> {
    mesh.vertices()
        .filter_map(|(id, v)| {
            let s = screen(view, v.position())?;
            let d = distance(s, click);
            (d <= SNAP_PIXELS && visible(v.position())).then_some((id, s, d))
        })
        .min_by(|a, b| a.2.total_cmp(&b.2))
        .map(|(id, s, _)| (id, s))
}

/// Where a path of clicks crosses the edges that can be seen, in order along the path.
/// Clicks on corners cut through the corner. Clicks elsewhere only steer the path.
pub fn knife_cut(
    mesh: &Mesh,
    view: &ViewInfo,
    clicks: &[[f32; 2]],
    visible: &dyn Fn([f32; 3]) -> bool,
) -> KnifeCut {
    let mut cut = KnifeCut::default();
    let snapped: Vec<Option<(VertexId, [f32; 2])>> = clicks
        .iter()
        .map(|&c| snapped_vertex(mesh, view, c, visible))
        .collect();
    let path: Vec<[f32; 2]> = clicks
        .iter()
        .zip(&snapped)
        .map(|(&c, s)| s.map_or(c, |(_, at)| at))
        .collect();
    let projected: HashMap<VertexId, [f32; 2]> = mesh
        .vertices()
        .filter_map(|(id, v)| Some((id, screen(view, v.position())?)))
        .collect();

    let push = |cut: &mut KnifeCut, point: CutPoint, at: [f32; 2]| {
        if cut.points.last() != Some(&point) {
            cut.points.push(point);
            cut.screen.push(at);
        }
    };
    for (i, &click) in path.iter().enumerate() {
        if let Some((vertex, at)) = snapped[i] {
            push(&mut cut, CutPoint::Vertex(vertex), at);
        }
        let Some(&next) = path.get(i + 1) else {
            break;
        };
        let mut hits: Vec<(f32, CutPoint, [f32; 2])> = Vec::new();
        for (edge, e) in mesh.edges() {
            let [a, b] = e.vertices();
            let (Some(&sa), Some(&sb)) = (projected.get(&a), projected.get(&b)) else {
                continue;
            };
            let Some((u, v)) = crossing(click, next, sa, sb) else {
                continue;
            };
            let at = [
                click[0] + (next[0] - click[0]) * u,
                click[1] + (next[1] - click[1]) * u,
            ];
            let length = distance(sa, sb);
            let (pa, pb) = (
                mesh.vertex(a).map(|x| x.position()),
                mesh.vertex(b).map(|x| x.position()),
            );
            let (Some(pa), Some(pb)) = (pa, pb) else {
                continue;
            };
            let point = if v * length <= END_PIXELS {
                visible(pa).then_some((CutPoint::Vertex(a), sa))
            } else if (1.0 - v) * length <= END_PIXELS {
                visible(pb).then_some((CutPoint::Vertex(b), sb))
            } else {
                let factor = factor_for(view, pa, pb, v);
                let p = std::array::from_fn(|k| pa[k] + (pb[k] - pa[k]) * factor);
                visible(p).then_some((CutPoint::Edge { edge, factor }, at))
            };
            if let Some((point, at)) = point {
                hits.push((u, point, at));
            }
        }
        hits.sort_by(|x, y| x.0.total_cmp(&y.0));
        for (_, point, at) in hits {
            push(&mut cut, point, at);
        }
    }
    cut
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::Camera;
    use modeler_mesh::{Primitive, SelectMode};

    fn front_view() -> ViewInfo {
        let mut camera = Camera {
            target: [0.0; 3],
            distance: 6.0,
            ..Camera::default()
        };
        camera.set_view(0.0, 0.0);
        ViewInfo {
            matrix: camera.view_projection(1.0),
            width: 600.0,
            height: 600.0,
        }
    }

    fn cube() -> Mesh {
        let mut mesh = Mesh::new();
        mesh.add_primitive(Primitive::Cube { size: 2.0 }, [0.0; 3])
            .unwrap();
        mesh
    }

    #[test]
    fn a_line_across_the_front_face_cuts_it_in_two() {
        let mut mesh = cube();
        let view = front_view();
        // Seen from the front, the cube's face fills the middle of the screen. A line
        // from left to right through it crosses its two side edges.
        let everything = |_: [f32; 3]| true;
        let cut = knife_cut(&mesh, &view, &[[100.0, 300.0], [500.0, 300.0]], &everything);
        assert!(cut.points.len() >= 2, "{cut:?}");
        let faces = mesh.face_count();
        mesh.knife(&cut.points, SelectMode::Edge).unwrap();
        mesh.validate().unwrap();
        assert!(mesh.face_count() > faces);
    }

    #[test]
    fn hidden_edges_are_left_alone() {
        let mesh = cube();
        let view = front_view();
        // Only the front (y = -1) can be seen.
        let front = |p: [f32; 3]| p[1] < -0.99;
        let cut = knife_cut(&mesh, &view, &[[100.0, 300.0], [500.0, 300.0]], &front);
        let everything = |_: [f32; 3]| true;
        let through = knife_cut(&mesh, &view, &[[100.0, 300.0], [500.0, 300.0]], &everything);
        assert_eq!(cut.points.len(), 2);
        assert!(through.points.len() > cut.points.len());
    }

    #[test]
    fn clicks_on_corners_snap_to_them() {
        let mesh = cube();
        let view = front_view();
        let everything = |_: [f32; 3]| true;
        let corner = mesh
            .vertices()
            .map(|(id, v)| (id, view.project(v.position()).unwrap()))
            .find(|(_, s)| s[2] < 0.99)
            .unwrap();
        let near = [corner.1[0] + 4.0, corner.1[1] - 3.0];
        let (snapped, _) = snapped_vertex(&mesh, &view, near, &everything).unwrap();
        let at = mesh.vertex(snapped).unwrap().position();
        let s = view.project(at).unwrap();
        assert!(distance([s[0], s[1]], near) <= SNAP_PIXELS);
    }
}
