//! Face normals and triangulation. N-gons stay in the editing mesh, so this is only
//! used for drawing and export.

use crate::error::MeshError;
use crate::id::{FaceId, VertexId};
use crate::mesh::Mesh;

type V3 = [f32; 3];

/// Normal of a polygon by Newell's method, which also handles concave and slightly
/// bent polygons. Not normalized: its length is twice the polygon's area.
fn newell(points: &[V3]) -> [f64; 3] {
    let mut n = [0.0f64; 3];
    for (i, p) in points.iter().enumerate() {
        let q = points[(i + 1) % points.len()];
        let (px, py, pz) = (p[0] as f64, p[1] as f64, p[2] as f64);
        let (qx, qy, qz) = (q[0] as f64, q[1] as f64, q[2] as f64);
        n[0] += (py - qy) * (pz + qz);
        n[1] += (pz - qz) * (px + qx);
        n[2] += (px - qx) * (py + qy);
    }
    n
}

fn unit(n: [f64; 3]) -> Option<V3> {
    let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if length > 1e-12 && length.is_finite() {
        Some([
            (n[0] / length) as f32,
            (n[1] / length) as f32,
            (n[2] / length) as f32,
        ])
    } else {
        None
    }
}

/// Twice the signed area of triangle a, b, c seen in 2D. Positive means counterclockwise.
fn area2(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

/// Splits a polygon into `n - 2` triangles by ear clipping, keeping its winding.
/// Returns indices into `points`. Never fails: a polygon too damaged to clip properly
/// (crossing itself, or with no area) is finished as a fan.
pub(crate) fn triangulate_points(points: &[V3]) -> Vec<[usize; 3]> {
    let n = points.len();
    let mut out = Vec::with_capacity(n.saturating_sub(2));
    if n < 3 {
        return out;
    }
    if n == 3 {
        out.push([0, 1, 2]);
        return out;
    }

    // Project onto the plane by dropping the normal's largest axis, keeping the
    // polygon counterclockwise in 2D.
    let normal = newell(points);
    let drop = (0..3)
        .max_by(|&a, &b| normal[a].abs().total_cmp(&normal[b].abs()))
        .unwrap_or(2);
    let (u, v) = match drop {
        0 => (1, 2),
        1 => (2, 0),
        _ => (0, 1),
    };
    let flip = normal[drop] < 0.0;
    let flat: Vec<[f64; 2]> = points
        .iter()
        .map(|p| {
            let (x, y) = (p[u] as f64, p[v] as f64);
            if flip { [y, x] } else { [x, y] }
        })
        .collect();

    let mut remaining: Vec<usize> = (0..n).collect();
    while remaining.len() > 3 {
        let count = remaining.len();
        let ear = (0..count).find(|&i| {
            let a = remaining[(i + count - 1) % count];
            let b = remaining[i];
            let c = remaining[(i + 1) % count];
            if area2(flat[a], flat[b], flat[c]) <= 0.0 {
                return false;
            }
            !remaining.iter().any(|&p| {
                p != a
                    && p != b
                    && p != c
                    && area2(flat[a], flat[b], flat[p]) >= 0.0
                    && area2(flat[b], flat[c], flat[p]) >= 0.0
                    && area2(flat[c], flat[a], flat[p]) >= 0.0
            })
        });
        // A damaged polygon has no valid ear. Any corner keeps the count right.
        let i = ear.unwrap_or(1);
        let a = remaining[(i + count - 1) % count];
        let b = remaining[i];
        let c = remaining[(i + 1) % count];
        out.push([a, b, c]);
        remaining.remove(i);
    }
    out.push([remaining[0], remaining[1], remaining[2]]);
    out
}

impl Mesh {
    /// The face's unit normal, following its winding. `None` if the face has no area.
    pub fn face_normal(&self, face: FaceId) -> Result<Option<[f32; 3]>, MeshError> {
        self.require_face(face)?;
        Ok(unit(newell(&self.face_positions(face))))
    }

    /// The triangles of a face, in the face's winding.
    pub fn face_triangles(&self, face: FaceId) -> Result<Vec<[VertexId; 3]>, MeshError> {
        self.require_face(face)?;
        let vertices: Vec<VertexId> = self
            .face_walk(face)
            .map(|c| self.corners[c].vertex)
            .collect();
        let points: Vec<V3> = vertices
            .iter()
            .map(|&v| self.vertices[v].position)
            .collect();
        Ok(triangulate_points(&points)
            .into_iter()
            .map(|t| t.map(|i| vertices[i]))
            .collect())
    }

    /// How many triangles the mesh has when triangulated.
    pub fn triangle_count(&self) -> usize {
        self.faces().map(|(_, face)| face.corner_count() - 2).sum()
    }

    /// How many edges are not shared by exactly two faces. Zero on a closed surface.
    pub fn open_edge_count(&self) -> usize {
        self.edges()
            .filter(|&(id, _)| self.radial_walk(id).take(3).count() != 2)
            .count()
    }

    fn face_positions(&self, face: FaceId) -> Vec<V3> {
        self.face_walk(face)
            .map(|c| self.vertices[self.corners[c].vertex].position)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sub(a: V3, b: V3) -> V3 {
        [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
    }

    fn triangle_area(a: V3, b: V3, c: V3) -> f32 {
        let (u, v) = (sub(b, a), sub(c, a));
        let cross = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt() / 2.0
    }

    /// A correct triangulation covers exactly the polygon's area, with each triangle
    /// winding the same way as the polygon.
    fn assert_covers(points: &[V3], expected_area: f32) {
        let triangles = triangulate_points(points);
        assert_eq!(triangles.len(), points.len() - 2);
        let polygon_normal = newell(points);
        let mut total = 0.0;
        for t in &triangles {
            let tri = [points[t[0]], points[t[1]], points[t[2]]];
            total += triangle_area(tri[0], tri[1], tri[2]);
            let n = newell(&tri);
            let dot: f64 = (0..3).map(|i| n[i] * polygon_normal[i]).sum();
            assert!(dot > 0.0, "triangle {t:?} winds the wrong way");
        }
        assert!(
            (total - expected_area).abs() < 1e-3,
            "{total} vs {expected_area}"
        );
    }

    #[test]
    fn triangles_and_small_inputs() {
        assert!(triangulate_points(&[[0.0; 3], [1.0; 3]]).is_empty());
        assert_eq!(
            triangulate_points(&[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]),
            vec![[0, 1, 2]]
        );
    }

    #[test]
    fn convex_polygon() {
        let square = [
            [0.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [2.0, 2.0, 0.0],
            [0.0, 2.0, 0.0],
        ];
        assert_covers(&square, 4.0);
    }

    #[test]
    fn concave_polygons_in_every_rotation() {
        let shape = [
            [0.0, 0.0],
            [3.0, 0.0],
            [3.0, 1.0],
            [1.0, 1.0],
            [1.0, 3.0],
            [0.0, 3.0],
        ];
        for start in 0..shape.len() {
            let rotated: Vec<[f32; 2]> = (0..shape.len())
                .map(|i| shape[(i + start) % shape.len()])
                .collect();
            // On each axis plane, and with both windings.
            for axis in 0..3 {
                for reversed in [false, true] {
                    let mut points: Vec<V3> = rotated
                        .iter()
                        .map(|p| match axis {
                            0 => [p[0], p[1], 5.0],
                            1 => [p[1], 5.0, p[0]],
                            _ => [5.0, p[0], p[1]],
                        })
                        .collect();
                    if reversed {
                        points.reverse();
                    }
                    assert_covers(&points, 5.0);
                }
            }
        }
    }

    #[test]
    fn tilted_star_polygon() {
        let points: Vec<V3> = (0..10)
            .map(|i| {
                let angle = std::f32::consts::TAU * i as f32 / 10.0;
                let r = if i % 2 == 0 { 2.0 } else { 1.0 };
                let (x, y) = (r * angle.cos(), r * angle.sin());
                [x, y * 0.6, y * 0.8]
            })
            .collect();
        // Five-pointed star with outer radius 2 and inner radius 1.
        let area = 10.0 * 0.5 * 2.0 * 1.0 * (std::f32::consts::TAU / 10.0).sin();
        assert_covers(&points, area);
    }

    #[test]
    fn damaged_polygons_still_give_the_right_triangle_count() {
        let flat_line = [
            [0.0; 3],
            [1.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [3.0, 0.0, 0.0],
            [4.0, 0.0, 0.0],
        ];
        assert_eq!(triangulate_points(&flat_line).len(), 3);
        let crossing = [
            [0.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
        ];
        assert_eq!(triangulate_points(&crossing).len(), 2);
        let same_point = [[1.0; 3]; 6];
        assert_eq!(triangulate_points(&same_point).len(), 4);
    }

    #[test]
    fn repeated_positions_on_a_valid_outline_are_handled() {
        // Two corners at one spot, as a split edge at factor 0 creates.
        let points = [
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [2.0, 2.0, 0.0],
            [0.0, 2.0, 0.0],
        ];
        let triangles = triangulate_points(&points);
        assert_eq!(triangles.len(), 3);
        let total: f32 = triangles
            .iter()
            .map(|t| triangle_area(points[t[0]], points[t[1]], points[t[2]]))
            .sum();
        assert!((total - 4.0).abs() < 1e-3, "{total}");
    }

    #[test]
    fn normals_and_counts_on_a_mesh() {
        let mut mesh = Mesh::new();
        let v: Vec<_> = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ]
        .iter()
        .map(|&p| mesh.add_vertex(p).unwrap())
        .collect();
        let f = mesh.add_face(&v).unwrap();
        assert_eq!(mesh.face_normal(f), Ok(Some([0.0, 0.0, 1.0])));
        assert_eq!(mesh.triangle_count(), 2);
        assert_eq!(mesh.open_edge_count(), 4);
        let triangles = mesh.face_triangles(f).unwrap();
        assert_eq!(triangles.len(), 2);
        mesh.flip_face(f).unwrap();
        assert_eq!(mesh.face_normal(f), Ok(Some([0.0, 0.0, -1.0])));

        mesh.remove_face(f).unwrap();
        assert_eq!(mesh.face_normal(f), Err(MeshError::StaleFace(f)));
        assert_eq!(mesh.face_triangles(f), Err(MeshError::StaleFace(f)));
        assert_eq!(mesh.triangle_count(), 0);
    }

    #[test]
    fn a_face_with_no_area_has_no_normal() {
        let mut mesh = Mesh::new();
        let v: Vec<_> = (0..3)
            .map(|i| mesh.add_vertex([i as f32, 0.0, 0.0]).unwrap())
            .collect();
        let f = mesh.add_face(&v).unwrap();
        assert_eq!(mesh.face_normal(f), Ok(None));
    }
}
