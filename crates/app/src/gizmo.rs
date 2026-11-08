//! Blender-style transform handles on the selection: arrows to move, red, green and
//! blue rings to rotate, and square-ended handles to scale. Red is X, green Y and
//! blue Z. The white ring and the middle circle work in the view's own plane.
//!
//! This file only works out where the handles are on screen and which one the mouse
//! is over. Dragging one starts an ordinary transform locked to that axis, and the
//! interface draws them.

use crate::camera::Camera;
use crate::transform::Kind;

type V3 = [f32; 3];

/// Handle size on screen, in pixels, whatever the zoom.
pub const RADIUS: f32 = 84.0;
/// How near the mouse must be to grab a handle.
const REACH: f32 = 9.0;
const RING_POINTS: usize = 72;

/// A part of the gizmo that can be dragged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    /// Along or around X, Y or Z.
    Axis(usize),
    /// In the view's plane: free move, rotation around the view, or even scale.
    View,
}

impl Handle {
    pub fn axis(self) -> Option<usize> {
        match self {
            Handle::Axis(a) => Some(a),
            Handle::View => None,
        }
    }
}

/// What to draw for one handle.
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    /// Lines through these points, on screen.
    Line(Vec<[f32; 2]>),
    /// A filled triangle, for an arrow's head.
    Triangle([[f32; 2]; 3]),
    /// A filled square of this half size, for a scale handle's end.
    Square([f32; 2], f32),
    /// A circle outline of this radius.
    Circle([f32; 2], f32),
}

/// One handle and how it looks.
#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    pub handle: Handle,
    pub shapes: Vec<Shape>,
}

/// The screen, for placing the gizmo.
pub struct Screen {
    pub camera: Camera,
    pub width: f32,
    pub height: f32,
}

impl Screen {
    pub fn project(&self, p: V3) -> Option<[f32; 2]> {
        let m = self
            .camera
            .view_projection(self.width / self.height.max(1.0));
        let at = |row: usize| m[row] * p[0] + m[4 + row] * p[1] + m[8 + row] * p[2] + m[12 + row];
        let w = at(3);
        (w > 1e-6).then(|| {
            [
                (at(0) / w + 1.0) / 2.0 * self.width,
                (1.0 - at(1) / w) / 2.0 * self.height,
            ]
        })
    }
}

fn axis(i: usize) -> V3 {
    let mut v = [0.0; 3];
    v[i] = 1.0;
    v
}

fn along(p: V3, d: V3, s: f32) -> V3 {
    [p[0] + d[0] * s, p[1] + d[1] * s, p[2] + d[2] * s]
}

fn distance(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

fn segment_distance(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let squared = dx * dx + dy * dy;
    let t = if squared > 0.0 {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / squared).clamp(0.0, 1.0)
    } else {
        0.0
    };
    distance(p, [a[0] + t * dx, a[1] + t * dy])
}

/// The gizmo for a transform kind, around `pivot`. Empty for kinds without one, or
/// if the pivot is behind the camera.
pub fn parts(kind: Kind, pivot: V3, screen: &Screen) -> Vec<Part> {
    let Some(center) = screen.project(pivot) else {
        return Vec::new();
    };
    let size = RADIUS * screen.camera.world_per_pixel_at(pivot, screen.height);
    let mut parts = Vec::new();
    match kind {
        Kind::Move | Kind::Scale => {
            for a in 0..3 {
                let dir = axis(a);
                let (Some(start), Some(end)) = (
                    screen.project(along(pivot, dir, size * 0.22)),
                    screen.project(along(pivot, dir, size)),
                ) else {
                    continue;
                };
                // An axis pointing at the viewer has no length to drag along.
                if distance(start, end) < 12.0 {
                    continue;
                }
                let mut shapes = vec![Shape::Line(vec![start, end])];
                let (dx, dy) = (end[0] - start[0], end[1] - start[1]);
                let length = (dx * dx + dy * dy).sqrt();
                let (ux, uy) = (dx / length, dy / length);
                if kind == Kind::Move {
                    let tip = [end[0] + ux * 16.0, end[1] + uy * 16.0];
                    let side = [-uy * 6.5, ux * 6.5];
                    shapes.push(Shape::Triangle([
                        tip,
                        [end[0] + side[0], end[1] + side[1]],
                        [end[0] - side[0], end[1] - side[1]],
                    ]));
                } else {
                    shapes.push(Shape::Square([end[0] + ux * 6.0, end[1] + uy * 6.0], 6.0));
                }
                parts.push(Part {
                    handle: Handle::Axis(a),
                    shapes,
                });
            }
            parts.push(Part {
                handle: Handle::View,
                shapes: vec![Shape::Circle(center, 10.0)],
            });
        }
        Kind::Rotate => {
            let toward = screen.camera.toward_camera();
            for a in 0..3 {
                // Two directions across the ring's plane.
                let (u, v) = match a {
                    0 => (axis(1), axis(2)),
                    1 => (axis(2), axis(0)),
                    _ => (axis(0), axis(1)),
                };
                // Only the half facing the viewer, as in Blender, split where it hides.
                let mut runs: Vec<Vec<[f32; 2]>> = vec![Vec::new()];
                for i in 0..=RING_POINTS {
                    let angle = i as f32 / RING_POINTS as f32 * std::f32::consts::TAU;
                    let offset: V3 =
                        std::array::from_fn(|k| (angle.cos() * u[k] + angle.sin() * v[k]) * size);
                    let facing =
                        offset[0] * toward[0] + offset[1] * toward[1] + offset[2] * toward[2];
                    match screen.project(along(pivot, offset, 1.0)) {
                        Some(p) if facing >= -0.02 * size => runs.last_mut().unwrap().push(p),
                        _ => {
                            if !runs.last().unwrap().is_empty() {
                                runs.push(Vec::new());
                            }
                        }
                    }
                }
                let shapes: Vec<Shape> = runs
                    .into_iter()
                    .filter(|run| run.len() > 1)
                    .map(Shape::Line)
                    .collect();
                if !shapes.is_empty() {
                    parts.push(Part {
                        handle: Handle::Axis(a),
                        shapes,
                    });
                }
            }
            parts.push(Part {
                handle: Handle::View,
                shapes: vec![Shape::Circle(center, RADIUS * 1.22)],
            });
        }
        _ => {}
    }
    parts
}

/// The handle under the mouse, if any. The nearest wins.
pub fn hit(parts: &[Part], mouse: [f32; 2]) -> Option<Handle> {
    let mut best: Option<(f32, Handle)> = None;
    for part in parts {
        for shape in &part.shapes {
            let d = match shape {
                Shape::Line(points) => points
                    .windows(2)
                    .map(|w| segment_distance(mouse, w[0], w[1]))
                    .fold(f32::MAX, f32::min),
                Shape::Triangle(corners) => {
                    let middle = [
                        (corners[0][0] + corners[1][0] + corners[2][0]) / 3.0,
                        (corners[0][1] + corners[1][1] + corners[2][1]) / 3.0,
                    ];
                    (distance(mouse, middle) - 6.0).max(0.0)
                }
                Shape::Square(at, half) => (distance(mouse, *at) - half).max(0.0),
                // The middle circle is grabbed anywhere inside; a ring only on its line.
                Shape::Circle(at, radius) if *radius < 20.0 => {
                    (distance(mouse, *at) - radius).max(0.0)
                }
                Shape::Circle(at, radius) => (distance(mouse, *at) - radius).abs(),
            };
            if d <= REACH && best.is_none_or(|(nearest, _)| d < nearest) {
                best = Some((d, part.handle));
            }
        }
    }
    best.map(|(_, handle)| handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Looking from the front: X to the right, Z up, Y away from the viewer.
    fn front() -> Screen {
        let mut camera = Camera {
            distance: 10.0,
            ..Camera::default()
        };
        camera.set_view(0.0, 0.0);
        Screen {
            camera,
            width: 800.0,
            height: 600.0,
        }
    }

    #[test]
    fn move_arrows_point_along_their_axes_and_hide_when_pointing_at_you() {
        let screen = front();
        let parts = parts(Kind::Move, [0.0; 3], &screen);
        let handles: Vec<Handle> = parts.iter().map(|p| p.handle).collect();
        // Y points straight away from the viewer, so it has no arrow.
        assert_eq!(
            handles,
            vec![Handle::Axis(0), Handle::Axis(2), Handle::View]
        );
        let center = [400.0, 300.0];
        assert_eq!(
            hit(&parts, [400.0 + RADIUS * 0.8, 300.0]),
            Some(Handle::Axis(0))
        );
        assert_eq!(
            hit(&parts, [400.0, 300.0 - RADIUS * 0.8]),
            Some(Handle::Axis(2))
        );
        assert_eq!(hit(&parts, center), Some(Handle::View));
        assert_eq!(hit(&parts, [400.0 - RADIUS * 0.8, 300.0]), None);
        assert_eq!(hit(&parts, [600.0, 500.0]), None);
        // The arrow's head can be grabbed too.
        assert_eq!(
            hit(&parts, [400.0 + RADIUS + 8.0, 300.0]),
            Some(Handle::Axis(0))
        );
    }

    #[test]
    fn rotate_rings_are_where_the_axes_say() {
        let screen = front();
        let parts = parts(Kind::Rotate, [0.0; 3], &screen);
        // The Y ring faces the viewer as a full circle; X and Z are seen edge on.
        let diagonal = std::f32::consts::FRAC_1_SQRT_2 * RADIUS;
        assert_eq!(
            hit(&parts, [400.0 + diagonal, 300.0 - diagonal]),
            Some(Handle::Axis(1))
        );
        assert_eq!(
            hit(&parts, [400.0, 300.0 + RADIUS * 0.5]),
            Some(Handle::Axis(0))
        );
        assert_eq!(
            hit(&parts, [400.0 - RADIUS * 0.5, 300.0]),
            Some(Handle::Axis(2))
        );
        // The outer white ring turns around the view.
        assert_eq!(
            hit(&parts, [400.0 + RADIUS * 1.22, 300.0]),
            Some(Handle::View)
        );
        assert_eq!(
            hit(&parts, [400.0 + RADIUS * 0.4, 300.0 - RADIUS * 0.4]),
            None
        );
    }

    #[test]
    fn only_the_near_half_of_a_tilted_ring_shows() {
        let mut screen = front();
        screen.camera.set_view(30.0, 40.0);
        let parts = parts(Kind::Rotate, [1.0, 2.0, 0.5], &screen);
        let toward = screen.camera.toward_camera();
        let pivot = [1.0, 2.0, 0.5];
        let size = RADIUS * screen.camera.world_per_pixel_at(pivot, screen.height);
        // Each drawn point of the Z ring must be on the viewer's side.
        let z_ring = parts.iter().find(|p| p.handle == Handle::Axis(2)).unwrap();
        let drawn: usize = z_ring
            .shapes
            .iter()
            .map(|s| match s {
                Shape::Line(points) => points.len(),
                _ => 0,
            })
            .sum();
        assert!(drawn > RING_POINTS / 3 && drawn < RING_POINTS, "{drawn}");
        // The far point of the ring is not drawn.
        let far: V3 = std::array::from_fn(|k| {
            let flat = [toward[0], toward[1], 0.0];
            let length = (flat[0] * flat[0] + flat[1] * flat[1]).sqrt();
            pivot[k] - flat[k] / length * size
        });
        let far_screen = screen.project(far).unwrap();
        let near_ring = z_ring.shapes.iter().any(|s| match s {
            Shape::Line(points) => points.iter().any(|p| distance(*p, far_screen) < 2.0),
            _ => false,
        });
        assert!(!near_ring);
    }

    #[test]
    fn scale_handles_and_other_kinds() {
        let screen = front();
        let parts = parts(Kind::Scale, [0.0; 3], &screen);
        assert!(matches!(parts[0].shapes[1], Shape::Square(..)));
        assert_eq!(
            hit(&parts, [400.0 + RADIUS + 6.0, 300.0]),
            Some(Handle::Axis(0))
        );
        assert!(super::parts(Kind::Inset, [0.0; 3], &screen).is_empty());
        assert_eq!(Handle::Axis(2).axis(), Some(2));
        assert_eq!(Handle::View.axis(), None);
        // Behind the camera, nothing.
        let behind = [0.0, -20.0, 0.0];
        assert!(super::parts(Kind::Move, behind, &screen).is_empty());
    }
}
