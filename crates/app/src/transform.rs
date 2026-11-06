//! The math for dragging a selection: move, rotate and scale, with an optional axis
//! lock and snapping. No window or graphics code, so it is tested directly.

use modeler_mesh::VertexId;

use crate::camera::Camera;

const SNAP_MOVE: f32 = 1.0;
const SNAP_ROTATE_RADIANS: f32 = 15.0 * std::f32::consts::PI / 180.0;
const SNAP_SCALE: f32 = 0.1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Move,
    Rotate,
    Scale,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Move => "Move",
            Kind::Rotate => "Rotate",
            Kind::Scale => "Scale",
        }
    }
}

type V3 = [f32; 3];

fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn scaled(a: V3, s: f32) -> V3 {
    a.map(|c| c * s)
}
fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn axis_vector(axis: usize) -> V3 {
    let mut v = [0.0; 3];
    v[axis] = 1.0;
    v
}

fn snapped(value: f32, step: f32) -> f32 {
    (value / step).round() * step
}

/// Where the mouse and view are, for working out a transform.
#[derive(Debug, Clone, Copy)]
pub struct Input {
    pub mouse: [f32; 2],
    pub snap: bool,
    pub camera: Camera,
    pub width: f32,
    pub height: f32,
}

/// A transform in progress.
#[derive(Debug, Clone)]
pub struct Transform {
    pub kind: Kind,
    /// 0, 1 or 2 when locked to X, Y or Z.
    pub axis: Option<usize>,
    start_mouse: [f32; 2],
    pub originals: Vec<(VertexId, V3)>,
    pub pivot: V3,
}

impl Transform {
    pub fn new(
        kind: Kind,
        start_mouse: [f32; 2],
        originals: Vec<(VertexId, V3)>,
        pivot: V3,
    ) -> Self {
        Self {
            kind,
            axis: None,
            start_mouse,
            originals,
            pivot,
        }
    }

    /// Pressing an axis key locks to it, and pressing it again unlocks.
    pub fn toggle_axis(&mut self, axis: usize) {
        self.axis = if self.axis == Some(axis) {
            None
        } else {
            Some(axis)
        };
    }

    /// The new position of a point that started at `point`.
    pub fn apply(&self, point: V3, input: &Input) -> V3 {
        match self.kind {
            Kind::Move => add(point, self.move_offset(input)),
            Kind::Rotate => self.rotate(point, input),
            Kind::Scale => self.scale(point, input),
        }
    }

    fn move_offset(&self, input: &Input) -> V3 {
        let dx = input.mouse[0] - self.start_mouse[0];
        let dy = input.mouse[1] - self.start_mouse[1];
        let (right, up) = input.camera.right_and_up();
        let per_pixel = input.camera.world_per_pixel_at(self.pivot, input.height);
        let mut offset = match self.axis {
            None => add(scaled(right, dx * per_pixel), scaled(up, -dy * per_pixel)),
            Some(axis) => {
                // How far along the axis the mouse has gone, as the axis looks on screen.
                let a = axis_vector(axis);
                let (sx, sy) = (dot(a, right), -dot(a, up));
                let squared = sx * sx + sy * sy;
                if squared < 1e-4 {
                    [0.0; 3] // The axis points straight at the viewer.
                } else {
                    scaled(a, (dx * sx + dy * sy) / squared * per_pixel)
                }
            }
        };
        if input.snap {
            offset = offset.map(|c| snapped(c, SNAP_MOVE));
        }
        offset
    }

    fn pivot_on_screen(&self, input: &Input) -> [f32; 2] {
        let aspect = input.width / input.height.max(1.0);
        let m = input.camera.view_projection(aspect);
        let p = self.pivot;
        let at = |row: usize| m[row] * p[0] + m[4 + row] * p[1] + m[8 + row] * p[2] + m[12 + row];
        let w = at(3);
        if w <= 1e-6 {
            return [input.width / 2.0, input.height / 2.0];
        }
        [
            (at(0) / w + 1.0) / 2.0 * input.width,
            (1.0 - at(1) / w) / 2.0 * input.height,
        ]
    }

    fn rotate(&self, point: V3, input: &Input) -> V3 {
        let center = self.pivot_on_screen(input);
        let angle_of = |p: [f32; 2]| (p[1] - center[1]).atan2(p[0] - center[0]);
        // Screen Y runs down, so a growing angle turns clockwise on screen.
        let mut clockwise = angle_of(input.mouse) - angle_of(self.start_mouse);
        if input.snap {
            clockwise = snapped(clockwise, SNAP_ROTATE_RADIANS);
        }
        let toward_viewer = input.camera.toward_camera();
        let (axis, angle) = match self.axis {
            None => (toward_viewer, -clockwise),
            Some(axis) => {
                let a = axis_vector(axis);
                // Follow the mouse, whichever way the axis faces.
                let sign = if dot(a, toward_viewer) >= 0.0 {
                    -1.0
                } else {
                    1.0
                };
                (a, clockwise * sign)
            }
        };
        let v = sub(point, self.pivot);
        let (sin, cos) = angle.sin_cos();
        // Rodrigues' rotation formula.
        let rotated = add(
            add(scaled(v, cos), scaled(cross(axis, v), sin)),
            scaled(axis, dot(axis, v) * (1.0 - cos)),
        );
        add(self.pivot, rotated)
    }

    fn scale(&self, point: V3, input: &Input) -> V3 {
        let center = self.pivot_on_screen(input);
        let distance =
            |p: [f32; 2]| ((p[0] - center[0]).powi(2) + (p[1] - center[1]).powi(2)).sqrt();
        let mut factor = distance(input.mouse) / distance(self.start_mouse).max(1.0);
        if input.snap {
            factor = snapped(factor, SNAP_SCALE);
        }
        let v = sub(point, self.pivot);
        match self.axis {
            None => add(self.pivot, scaled(v, factor)),
            Some(axis) => {
                let mut out = point;
                out[axis] = self.pivot[axis] + v[axis] * factor;
                out
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(camera: Camera, mouse: [f32; 2]) -> Input {
        Input {
            mouse,
            snap: false,
            camera,
            width: 800.0,
            height: 600.0,
        }
    }

    fn front() -> Camera {
        let mut camera = Camera {
            distance: 10.0,
            ..Camera::default()
        };
        camera.set_view(0.0, 0.0);
        camera
    }

    fn transform(kind: Kind, start: [f32; 2]) -> Transform {
        Transform::new(kind, start, Vec::new(), [0.0; 3])
    }

    fn near(a: V3, b: V3) {
        for i in 0..3 {
            assert!((a[i] - b[i]).abs() < 1e-3, "{a:?} vs {b:?}");
        }
    }

    fn project(camera: &Camera, p: V3, width: f32, height: f32) -> [f32; 2] {
        let m = camera.view_projection(width / height);
        let at = |row: usize| m[row] * p[0] + m[4 + row] * p[1] + m[8 + row] * p[2] + m[12 + row];
        let w = at(3);
        [
            (at(0) / w + 1.0) / 2.0 * width,
            (1.0 - at(1) / w) / 2.0 * height,
        ]
    }

    #[test]
    fn move_keeps_the_point_under_the_mouse() {
        for (yaw, pitch) in [(0.0, 0.0), (35.0, 25.0), (120.0, 60.0)] {
            let mut camera = front();
            camera.set_view(yaw, pitch);
            let t = transform(Kind::Move, [400.0, 300.0]);
            let moved = t.apply([0.0; 3], &input(camera, [460.0, 260.0]));
            let screen = project(&camera, moved, 800.0, 600.0);
            assert!(
                (screen[0] - 460.0).abs() < 2.0 && (screen[1] - 260.0).abs() < 2.0,
                "{screen:?}"
            );
        }
    }

    #[test]
    fn no_mouse_movement_means_no_change() {
        let point = [1.0, 2.0, 3.0];
        for kind in [Kind::Move, Kind::Rotate, Kind::Scale] {
            let t = Transform::new(kind, [500.0, 200.0], Vec::new(), [0.5, 0.5, 0.5]);
            near(t.apply(point, &input(front(), [500.0, 200.0])), point);
        }
    }

    #[test]
    fn axis_lock_moves_along_that_axis_only() {
        let camera = front();
        let mut t = transform(Kind::Move, [400.0, 300.0]);
        t.toggle_axis(0);
        let moved = t.apply([0.0; 3], &input(camera, [460.0, 300.0]));
        assert!(moved[0] > 0.0);
        assert_eq!((moved[1], moved[2]), (0.0, 0.0));
        // Vertical mouse movement does nothing to a horizontal axis.
        near(t.apply([0.0; 3], &input(camera, [400.0, 200.0])), [0.0; 3]);

        t.toggle_axis(2);
        assert_eq!(t.axis, Some(2));
        let up = t.apply([0.0; 3], &input(camera, [400.0, 200.0]));
        assert!(up[2] > 0.0, "moving the mouse up must move +Z up");
        t.toggle_axis(2);
        assert_eq!(t.axis, None);
    }

    #[test]
    fn an_axis_pointing_at_the_viewer_does_not_jump() {
        let mut t = transform(Kind::Move, [400.0, 300.0]);
        t.toggle_axis(1); // The front view looks along Y.
        near(t.apply([0.0; 3], &input(front(), [700.0, 100.0])), [0.0; 3]);
    }

    #[test]
    fn move_snaps_to_whole_units() {
        let t = transform(Kind::Move, [400.0, 300.0]);
        let mut with_snap = input(front(), [455.0, 300.0]);
        with_snap.snap = true;
        let moved = t.apply([0.25, 0.0, 0.0], &with_snap);
        assert_eq!((moved[0] - 0.25).fract(), 0.0, "{moved:?}");
    }

    #[test]
    fn rotation_follows_the_mouse_around_the_pivot() {
        for (yaw, pitch) in [(0.0, 0.0), (0.0, 90.0), (50.0, 30.0)] {
            let mut camera = front();
            camera.set_view(yaw, pitch);
            camera.orthographic = true;
            // A quarter turn clockwise on screen, from the right to the bottom.
            let t = transform(Kind::Rotate, [500.0, 300.0]);
            let [right, up] = {
                let (r, u) = camera.right_and_up();
                [r, u]
            };
            let rotated = t.apply(right, &input(camera, [400.0, 400.0]));
            near(rotated, up.map(|c| -c));
        }
    }

    #[test]
    fn axis_rotation_turns_around_that_axis_and_follows_the_mouse_from_both_sides() {
        let mut above = front();
        above.set_view(0.0, 90.0);
        above.orthographic = true;
        let mut t = transform(Kind::Rotate, [500.0, 300.0]);
        t.toggle_axis(2);
        // Clockwise seen from above takes +X to -Y.
        near(
            t.apply([1.0, 0.0, 5.0], &input(above, [400.0, 400.0])),
            [0.0, -1.0, 5.0],
        );

        let mut below = above;
        below.set_view(0.0, -90.0);
        let screen_before = project(&below, [1.0, 0.0, 0.0], 800.0, 600.0);
        assert!(screen_before[0] > 400.0);
        let turned = t.apply([1.0, 0.0, 0.0], &input(below, [400.0, 400.0]));
        let screen_after = project(&below, turned, 800.0, 600.0);
        assert!(
            screen_after[1] > 300.0 + 10.0,
            "must move toward the mouse: {screen_after:?}"
        );
    }

    #[test]
    fn rotation_keeps_distance_from_the_pivot_and_snaps() {
        let t = Transform::new(Kind::Rotate, [500.0, 300.0], Vec::new(), [1.0, 1.0, 1.0]);
        let moved = t.apply([3.0, 1.0, 1.0], &input(front(), [450.0, 380.0]));
        let d = sub(moved, [1.0, 1.0, 1.0]);
        assert!((dot(d, d).sqrt() - 2.0).abs() < 1e-3);

        let mut camera = front();
        camera.orthographic = true;
        let t = transform(Kind::Rotate, [500.0, 300.0]);
        let mut with_snap = input(camera, [500.0, 310.0]); // About 5.7 degrees.
        with_snap.snap = true;
        near(t.apply([1.0, 0.0, 0.0], &with_snap), [1.0, 0.0, 0.0]);
    }

    #[test]
    fn scale_follows_distance_from_the_pivot() {
        let t = transform(Kind::Scale, [500.0, 300.0]); // 100 pixels from the pivot.
        near(
            t.apply([1.0, 2.0, 3.0], &input(front(), [600.0, 300.0])),
            [2.0, 4.0, 6.0],
        );
        near(
            t.apply([1.0, 2.0, 3.0], &input(front(), [450.0, 300.0])),
            [0.5, 1.0, 1.5],
        );
        // Going through the pivot does not flip the shape.
        let at_pivot = t.apply([1.0, 2.0, 3.0], &input(front(), [400.0, 300.0]));
        near(at_pivot, [0.0; 3]);
    }

    #[test]
    fn scale_can_lock_to_an_axis_and_snap() {
        let mut t = Transform::new(Kind::Scale, [500.0, 300.0], Vec::new(), [1.0, 1.0, 1.0]);
        t.toggle_axis(0);
        let pivot_x = project(&front(), [1.0, 1.0, 1.0], 800.0, 600.0);
        let t2 = Transform {
            start_mouse: [pivot_x[0] + 100.0, pivot_x[1]],
            ..t.clone()
        };
        let done = t2.apply(
            [3.0, 5.0, 7.0],
            &input(front(), [pivot_x[0] + 200.0, pivot_x[1]]),
        );
        near(done, [5.0, 5.0, 7.0]);

        let t = transform(Kind::Scale, [500.0, 300.0]);
        let mut with_snap = input(front(), [512.0, 300.0]); // A factor of 1.12.
        with_snap.snap = true;
        near(t.apply([1.0, 0.0, 0.0], &with_snap), [1.1, 0.0, 0.0]);
    }

    #[test]
    fn starting_a_scale_on_the_pivot_gives_finite_numbers() {
        let t = transform(Kind::Scale, [400.0, 300.0]);
        let done = t.apply([1.0, 1.0, 1.0], &input(front(), [500.0, 300.0]));
        assert!(done.iter().all(|c| c.is_finite()));
    }

    #[test]
    fn names_are_for_the_panel() {
        assert_eq!(Kind::Move.name(), "Move");
        assert_eq!(Kind::Rotate.name(), "Rotate");
        assert_eq!(Kind::Scale.name(), "Scale");
    }
}
