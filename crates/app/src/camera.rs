//! The viewport camera: a turntable orbit around a target point, with Z up as in
//! Blender. This file has no graphics code, so it is tested without a window.

use crate::grid::GridPlane;

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

    /// The screen's right and up directions in world space.
    pub fn right_and_up(&self) -> ([f32; 3], [f32; 3]) {
        let [right, up, _] = self.rotation_rows();
        (right, up)
    }

    /// World units per pixel, at the depth of `point`.
    pub fn world_per_pixel_at(&self, point: [f32; 3], viewport_height: f32) -> f32 {
        let height = viewport_height.max(1.0);
        if self.orthographic {
            return self.world_height_at_target() / height;
        }
        let toward = self.toward_camera();
        let along: f32 = (0..3)
            .map(|i| (point[i] - self.target[i]) * toward[i])
            .sum();
        let depth = (self.distance - along).max(self.distance * 0.01);
        2.0 * depth * (FIELD_OF_VIEW / 2.0).tan() / height
    }

    /// The direction from the target toward the camera.
    pub fn toward_camera(&self) -> [f32; 3] {
        self.rotation_rows()[2]
    }

    /// The main light: from near the camera, a little above and to the left, so that
    /// faces turned toward the viewer differ in brightness.
    pub fn key_light(&self) -> [f32; 3] {
        let [right, up, toward] = self.rotation_rows();
        let v: [f32; 3] = std::array::from_fn(|i| toward[i] + up[i] * 0.55 - right[i] * 0.35);
        let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        v.map(|c| c / length)
    }

    /// How far from the target the grid stays visible.
    pub fn grid_fade_radius(&self) -> f32 {
        (self.distance * 3.0).clamp(8.0, 60.0)
    }

    /// The depth shift that brings a point 2% of the camera distance closer, at the
    /// target's depth. Perspective depth is not linear, so this depends on the view.
    pub fn point_depth_bias(&self) -> f32 {
        let pull = self.distance * 0.02;
        let (near, far) = self.depth_range();
        if self.orthographic {
            -2.0 * pull / (far - near)
        } else {
            // The slope of depth at distance d is 2 * far * near / ((far - near) * d^2).
            -2.0 * far * near * pull / ((far - near) * self.distance * self.distance)
        }
    }

    /// The near and far limits of the view.
    fn depth_range(&self) -> (f32, f32) {
        // The far limit must always reach the whole grid, whatever the zoom.
        let far = (self.distance * 100.0).max(1000.0);
        let near = if self.orthographic {
            -far
        } else {
            self.distance * 0.01
        };
        (near, far)
    }

    /// The grid plane to draw from this angle.
    pub fn grid_plane(&self) -> GridPlane {
        const LIMIT: f32 = 0.01;
        if self.pitch.sin().abs() > LIMIT {
            GridPlane::Ground
        } else if self.yaw.sin().abs() < LIMIT {
            GridPlane::FrontBack
        } else if self.yaw.cos().abs() < LIMIT {
            GridPlane::Side
        } else {
            GridPlane::Ground
        }
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
        let (near, far) = self.depth_range();
        if self.orthographic {
            let half_height = self.world_height_at_target() / 2.0;
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
    fn the_grid_stands_upright_only_in_level_views_along_an_axis() {
        assert_eq!(camera(0.0, 0.0).grid_plane(), GridPlane::FrontBack);
        assert_eq!(camera(180.0, 0.0).grid_plane(), GridPlane::FrontBack);
        assert_eq!(camera(90.0, 0.0).grid_plane(), GridPlane::Side);
        assert_eq!(camera(-90.0, 0.0).grid_plane(), GridPlane::Side);
        assert_eq!(camera(0.0, 90.0).grid_plane(), GridPlane::Ground);
        assert_eq!(camera(0.0, -90.0).grid_plane(), GridPlane::Ground);
        assert_eq!(camera(0.0, 30.0).grid_plane(), GridPlane::Ground);
        assert_eq!(camera(45.0, 0.0).grid_plane(), GridPlane::Ground);
        assert_eq!(Camera::default().grid_plane(), GridPlane::Ground);
    }

    #[test]
    fn closer_points_cover_less_world_per_pixel() {
        let cam = camera(0.0, 0.0);
        let at_target = cam.world_per_pixel_at([0.0; 3], 600.0);
        assert!(close(at_target, cam.world_height_at_target() / 600.0));
        assert!(cam.world_per_pixel_at([0.0, -5.0, 0.0], 600.0) < at_target);
        assert!(cam.world_per_pixel_at([0.0, 5.0, 0.0], 600.0) > at_target);
        // A point behind the camera still gives a usable number.
        assert!(cam.world_per_pixel_at([0.0, -50.0, 0.0], 600.0) > 0.0);
        let flat = Camera {
            orthographic: true,
            ..cam
        };
        assert_eq!(
            flat.world_per_pixel_at([0.0, -5.0, 0.0], 600.0),
            flat.world_per_pixel_at([0.0, 5.0, 0.0], 600.0)
        );
    }

    #[test]
    fn the_key_light_comes_from_the_front_and_is_one_unit_long() {
        for (yaw, pitch) in [(0.0, 0.0), (40.0, 30.0), (170.0, -60.0)] {
            let cam = camera(yaw, pitch);
            let light = cam.key_light();
            let length = (light[0].powi(2) + light[1].powi(2) + light[2].powi(2)).sqrt();
            assert!(close(length, 1.0));
            let toward = cam.toward_camera();
            assert!((0..3).map(|i| light[i] * toward[i]).sum::<f32>() > 0.5);
        }
        for (distance, radius) in [(0.1, 8.0), (1000.0, 60.0)] {
            let cam = Camera {
                distance,
                ..Camera::default()
            };
            assert_eq!(cam.grid_fade_radius(), radius);
        }
    }

    #[test]
    fn the_point_bias_moves_a_point_two_percent_closer_at_the_target() {
        for orthographic in [false, true] {
            for distance in [0.5, 6.0, 40.0, 300.0] {
                let cam = Camera {
                    distance,
                    orthographic,
                    ..Camera::default()
                };
                let m = multiply(&cam.projection_matrix(1.5), &cam.view_matrix());
                let toward = cam.toward_camera();
                let depth = |pull: f32| {
                    let p: [f32; 3] = std::array::from_fn(|i| cam.target[i] + toward[i] * pull);
                    project(&m, p)[2]
                };
                let wanted = depth(distance * 0.02) - depth(0.0);
                let got = cam.point_depth_bias();
                assert!(
                    (got - wanted).abs() <= wanted.abs() * 0.05,
                    "ortho {orthographic}, distance {distance}: {got} vs {wanted}"
                );
            }
        }
    }

    #[test]
    fn toward_camera_points_at_the_camera() {
        assert_near(camera(0.0, 0.0).toward_camera(), [0.0, -1.0, 0.0]);
        assert_near(camera(90.0, 0.0).toward_camera(), [1.0, 0.0, 0.0]);
        assert_near(camera(0.0, 90.0).toward_camera(), [0.0, 0.0, 1.0]);
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
