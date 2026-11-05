//! The viewport camera: a turntable orbit around a target point, with Z up as in
//! Blender. This file has no graphics code, so it is tested without a window.

/// Matrices are column-major, which is the layout OpenGL expects.
pub type Mat4 = [f32; 16];

const FIELD_OF_VIEW: f32 = 50.0 * std::f32::consts::PI / 180.0;
const ORBIT_RADIANS_PER_PIXEL: f32 = 0.008;
const ZOOM_PER_WHEEL_STEP: f32 = 0.9;
const MIN_DISTANCE: f32 = 0.05;
const MAX_DISTANCE: f32 = 2000.0;
const HALF_PI: f32 = std::f32::consts::FRAC_PI_2;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    pub target: [f32; 3],
    pub distance: f32,
    /// Rotation around Z. Zero looks along +Y, and larger values move the camera
    /// toward +X.
    pub yaw: f32,
    /// Height above the ground plane. Zero is level and 90 degrees looks straight
    /// down. Going past that is allowed, as in Blender.
    pub pitch: f32,
    pub orthographic: bool,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            target: [0.0; 3],
            distance: 12.0,
            yaw: 40f32.to_radians(),
            pitch: 30f32.to_radians(),
            orthographic: false,
        }
    }
}

impl Camera {
    /// Standard views, which keep the target and distance.
    pub fn set_view(&mut self, yaw_degrees: f32, pitch_degrees: f32) {
        self.yaw = yaw_degrees.to_radians();
        self.pitch = pitch_degrees.to_radians();
    }

    pub fn orbit_pixels(&mut self, dx: f32, dy: f32) {
        self.yaw -= dx * ORBIT_RADIANS_PER_PIXEL;
        self.pitch += dy * ORBIT_RADIANS_PER_PIXEL;
    }

    pub fn orbit_degrees(&mut self, yaw: f32, pitch: f32) {
        self.yaw += yaw.to_radians();
        self.pitch += pitch.to_radians();
    }

    /// Moves the view so that the scene follows the mouse.
    pub fn pan_pixels(&mut self, dx: f32, dy: f32, viewport_height: f32) {
        let scale = self.world_height_at_target() / viewport_height.max(1.0);
        let [right, up, _] = self.rotation_rows();
        for axis in 0..3 {
            self.target[axis] += (-dx * right[axis] + dy * up[axis]) * scale;
        }
    }

    /// Positive steps zoom in.
    pub fn zoom_steps(&mut self, steps: f32) {
        self.distance =
            (self.distance * ZOOM_PER_WHEEL_STEP.powf(steps)).clamp(MIN_DISTANCE, MAX_DISTANCE);
    }

    pub fn zoom_pixels(&mut self, dy: f32) {
        self.zoom_steps(-dy * 0.05);
    }

    /// World units covered by the viewport's height, at the target's depth.
    fn world_height_at_target(&self) -> f32 {
        2.0 * self.distance * (FIELD_OF_VIEW / 2.0).tan()
    }

    /// The view's right, up and backward directions in world space.
    fn rotation_rows(&self) -> [[f32; 3]; 3] {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let (sin_tilt, cos_tilt) = (self.pitch - HALF_PI).sin_cos();
        [
            [cos_yaw, sin_yaw, 0.0],
            [-sin_yaw * cos_tilt, cos_yaw * cos_tilt, -sin_tilt],
            [-sin_yaw * sin_tilt, cos_yaw * sin_tilt, cos_tilt],
        ]
    }

    pub fn view_matrix(&self) -> Mat4 {
        let [r, u, b] = self.rotation_rows();
        let t = self.target;
        let dot = |row: [f32; 3]| row[0] * t[0] + row[1] * t[1] + row[2] * t[2];
        [
            r[0],
            u[0],
            b[0],
            0.0,
            r[1],
            u[1],
            b[1],
            0.0,
            r[2],
            u[2],
            b[2],
            0.0,
            -dot(r),
            -dot(u),
            -dot(b) - self.distance,
            1.0,
        ]
    }

    pub fn projection_matrix(&self, aspect: f32) -> Mat4 {
        let aspect = if aspect > 0.0 { aspect } else { 1.0 };
        // The far limit must always reach the whole grid, whatever the zoom.
        let far = (self.distance * 100.0).max(1000.0);
        if self.orthographic {
            let half_height = self.world_height_at_target() / 2.0;
            let near = -far;
            [
                1.0 / (half_height * aspect),
                0.0,
                0.0,
                0.0,
                0.0,
                1.0 / half_height,
                0.0,
                0.0,
                0.0,
                0.0,
                -2.0 / (far - near),
                0.0,
                0.0,
                0.0,
                -(far + near) / (far - near),
                1.0,
            ]
        } else {
            let near = self.distance * 0.01;
            let f = 1.0 / (FIELD_OF_VIEW / 2.0).tan();
            [
                f / aspect,
                0.0,
                0.0,
                0.0,
                0.0,
                f,
                0.0,
                0.0,
                0.0,
                0.0,
                (far + near) / (near - far),
                -1.0,
                0.0,
                0.0,
                2.0 * far * near / (near - far),
                0.0,
            ]
        }
    }

    pub fn view_projection(&self, aspect: f32) -> Mat4 {
        multiply(&self.projection_matrix(aspect), &self.view_matrix())
    }
}

pub fn multiply(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut out = [0.0; 16];
    for column in 0..4 {
        for row in 0..4 {
            out[column * 4 + row] = (0..4).map(|k| a[k * 4 + row] * b[column * 4 + k]).sum();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Where a world point lands on screen: x and y run from -1 to 1 across the viewport.
    fn project(matrix: &Mat4, p: [f32; 3]) -> [f32; 3] {
        let at = |row: usize| {
            matrix[row] * p[0] + matrix[4 + row] * p[1] + matrix[8 + row] * p[2] + matrix[12 + row]
        };
        let w = at(3);
        [at(0) / w, at(1) / w, at(2) / w]
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    fn assert_near(actual: [f32; 3], expected: [f32; 3]) {
        for i in 0..3 {
            assert!(close(actual[i], expected[i]), "{actual:?} vs {expected:?}");
        }
    }

    fn camera(yaw: f32, pitch: f32) -> Camera {
        let mut camera = Camera {
            distance: 10.0,
            ..Camera::default()
        };
        camera.set_view(yaw, pitch);
        camera
    }

    #[test]
    fn the_target_is_at_the_center_of_the_screen() {
        for orthographic in [false, true] {
            let camera = Camera {
                target: [3.0, -2.0, 1.0],
                orthographic,
                ..Camera::default()
            };
            let p = project(&camera.view_projection(1.5), camera.target);
            assert!(close(p[0], 0.0) && close(p[1], 0.0), "{p:?}");
            assert!(
                p[2].abs() < 1.0,
                "the target must be inside the depth range"
            );
        }
    }

    #[test]
    fn front_view_has_x_right_and_z_up() {
        let m = camera(0.0, 0.0).view_projection(1.0);
        assert!(project(&m, [1.0, 0.0, 0.0])[0] > 0.0);
        assert!(close(project(&m, [1.0, 0.0, 0.0])[1], 0.0));
        assert!(project(&m, [0.0, 0.0, 1.0])[1] > 0.0);
        assert!(close(project(&m, [0.0, 0.0, 1.0])[0], 0.0));
        // Moving along +Y goes away from the camera.
        assert!(project(&m, [0.0, 1.0, 0.0])[2] > project(&m, [0.0; 3])[2]);
    }

    #[test]
    fn right_view_has_y_right_and_z_up() {
        let m = camera(90.0, 0.0).view_projection(1.0);
        assert!(project(&m, [0.0, 1.0, 0.0])[0] > 0.0);
        assert!(project(&m, [0.0, 0.0, 1.0])[1] > 0.0);
        assert!(project(&m, [1.0, 0.0, 0.0])[2] < project(&m, [0.0; 3])[2]);
    }

    #[test]
    fn top_view_has_x_right_and_y_up() {
        let m = camera(0.0, 90.0).view_projection(1.0);
        assert!(project(&m, [1.0, 0.0, 0.0])[0] > 0.0);
        assert!(project(&m, [0.0, 1.0, 0.0])[1] > 0.0);
        assert!(project(&m, [0.0, 0.0, 1.0])[2] < project(&m, [0.0; 3])[2]);
    }

    #[test]
    fn views_past_the_top_stay_valid() {
        let m = camera(30.0, 170.0).view_projection(1.0);
        let p = project(&m, [0.0; 3]);
        assert!(p.iter().all(|c| c.is_finite()) && close(p[0], 0.0) && close(p[1], 0.0));
    }

    #[test]
    fn dragging_right_turns_the_scene_right() {
        let mut camera = camera(0.0, 0.0);
        let point = [1.0, -1.0, 0.0];
        let before = project(&camera.view_projection(1.0), point)[0];
        camera.orbit_pixels(20.0, 0.0);
        let after = project(&camera.view_projection(1.0), point)[0];
        assert!(after > before);
    }

    #[test]
    fn dragging_down_looks_from_higher_up() {
        let mut camera = camera(0.0, 20.0);
        let pitch = camera.pitch;
        camera.orbit_pixels(0.0, 10.0);
        assert!(close(camera.pitch - pitch, 0.08));
        camera.orbit_degrees(90.0, 0.0);
        assert!(close(camera.yaw, 90f32.to_radians()));
    }

    #[test]
    fn panning_keeps_a_point_under_the_mouse() {
        const HEIGHT: f32 = 600.0;
        for (orthographic, yaw, pitch) in [
            (false, 0.0, 0.0),
            (false, 37.0, 25.0),
            (true, 37.0, 25.0),
            (false, 120.0, 80.0),
        ] {
            let mut cam = camera(yaw, pitch);
            cam.orthographic = orthographic;
            cam.target = [1.0, 2.0, 3.0];
            let before = project(&cam.view_projection(1.0), cam.target);
            let point = cam.target;
            cam.pan_pixels(30.0, -45.0, HEIGHT);
            let after = project(&cam.view_projection(1.0), point);
            // The point should move by exactly the dragged pixels. Screen y is down.
            assert!(
                close(after[0] - before[0], 30.0 * 2.0 / HEIGHT),
                "{after:?}"
            );
            assert!(
                close(after[1] - before[1], 45.0 * 2.0 / HEIGHT),
                "{after:?}"
            );
        }
    }

    #[test]
    fn zoom_moves_closer_and_stops_at_its_limits() {
        let mut camera = Camera::default();
        let start = camera.distance;
        camera.zoom_steps(1.0);
        assert!(close(camera.distance, start * 0.9));
        camera.zoom_steps(-1.0);
        assert!(close(camera.distance, start));
        camera.zoom_pixels(-20.0);
        assert!(close(camera.distance, start * 0.9));
        camera.zoom_steps(10_000.0);
        assert_eq!(camera.distance, MIN_DISTANCE);
        camera.zoom_steps(-10_000.0);
        assert_eq!(camera.distance, MAX_DISTANCE);
    }

    #[test]
    fn orthographic_and_perspective_agree_at_the_target_depth() {
        let mut cam = camera(20.0, 30.0);
        let [right, _, _] = cam.rotation_rows();
        let point = right.map(|c| c * 2.0);
        let perspective = project(&cam.view_projection(1.5), point);
        cam.orthographic = true;
        let orthographic = project(&cam.view_projection(1.5), point);
        assert!(close(perspective[0], orthographic[0]));
        assert!(close(perspective[1], orthographic[1]));
    }

    #[test]
    fn orthographic_view_ignores_distance_from_the_camera() {
        let mut cam = camera(0.0, 0.0);
        cam.orthographic = true;
        let m = cam.view_projection(1.0);
        let near = project(&m, [1.0, -3.0, 1.0]);
        let far = project(&m, [1.0, 3.0, 1.0]);
        assert!(close(near[0], far[0]) && close(near[1], far[1]));
    }

    #[test]
    fn wide_windows_do_not_stretch_the_scene() {
        let cam = camera(0.0, 0.0);
        let square = project(&cam.view_projection(1.0), [1.0, 0.0, 1.0]);
        let wide = project(&cam.view_projection(2.0), [1.0, 0.0, 1.0]);
        assert!(close(wide[0], square[0] / 2.0));
        assert!(close(wide[1], square[1]));
    }

    #[test]
    fn a_collapsed_window_gives_no_invalid_numbers() {
        let cam = Camera::default();
        assert!(cam.projection_matrix(0.0).iter().all(|c| c.is_finite()));
        let mut panned = cam;
        panned.pan_pixels(5.0, 5.0, 0.0);
        assert!(panned.target.iter().all(|c| c.is_finite()));
    }

    #[test]
    fn the_whole_grid_is_inside_the_depth_range_at_all_zooms() {
        for distance in [MIN_DISTANCE, 1.0, 12.0, 100.0, MAX_DISTANCE] {
            for orthographic in [false, true] {
                let mut cam = camera(0.0, 0.0);
                cam.distance = distance;
                cam.orthographic = orthographic;
                let far_corner = project(&cam.view_projection(1.0), [0.0, 50.0, 0.0]);
                assert!(far_corner[2].abs() <= 1.0, "{distance} {far_corner:?}");
            }
        }
    }

    #[test]
    fn multiply_with_identity_changes_nothing() {
        let identity = {
            let mut m = [0.0; 16];
            for i in 0..4 {
                m[i * 5] = 1.0;
            }
            m
        };
        let m = Camera::default().view_matrix();
        assert_eq!(multiply(&identity, &m), m);
        assert_eq!(multiply(&m, &identity), m);
    }

    #[test]
    fn view_matrix_keeps_lengths() {
        let m = Camera::default().view_matrix();
        let p = project(&m, [3.0, 4.0, 0.0]);
        let q = project(&m, [0.0, 0.0, 0.0]);
        let length = ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt();
        assert!(close(length, 5.0));
        assert_near(q, [0.0, 0.0, -Camera::default().distance]);
    }
}
