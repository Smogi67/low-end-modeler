//! All OpenGL calls live here. Uses only OpenGL 2.1 features: GLSL 1.20 shaders
//! and vertex buffers, with no vertex array objects, shadows or anti-aliasing.

use glow::HasContext;

use crate::camera::Mat4;
use crate::grid::{FLOATS_PER_VERTEX, GridPlane, grid_vertices};
use crate::palette;
use crate::scene::{Changed, FLOATS_PER_LINE_VERTEX, FLOATS_PER_TRIANGLE_VERTEX, Scene};

const VERTEX_SHADER: &str = r#"#version 120
attribute vec3 a_position;
attribute vec3 a_color;
uniform mat4 u_view_projection;
uniform float u_depth_bias;
varying vec3 v_color;
varying vec3 v_world;
void main() {
    v_color = a_color;
    v_world = a_position;
    gl_Position = u_view_projection * vec4(a_position, 1.0);
    // Pushes grid lines back slightly, so one lying exactly on a face does not look
    // like an edge of that face.
    gl_Position.z += u_depth_bias * gl_Position.w;
    gl_PointSize = 5.0;
}
"#;

const FRAGMENT_SHADER: &str = r#"#version 120
uniform vec3 u_fade_center;
uniform float u_fade_radius;
// Above 0: the soft glow drawn around bright edges, this strong. Dark edges get none.
uniform float u_glow;
varying vec3 v_color;
varying vec3 v_world;
void main() {
    // The grid fades out with distance, which hides the flicker of far, dense lines.
    float alpha = 1.0;
    if (u_glow > 0.0) {
        float light = dot(v_color, vec3(0.3, 0.55, 0.15));
        alpha = u_glow * smoothstep(0.3, 0.55, light);
    }
    if (u_fade_radius > 0.0) {
        float d = distance(v_world, u_fade_center);
        alpha = 1.0 - smoothstep(u_fade_radius * 0.3, u_fade_radius, d);
    }
    gl_FragColor = vec4(v_color, alpha);
}
"#;

const MESH_VERTEX_SHADER: &str = r#"#version 120
attribute vec3 a_position;
attribute vec3 a_normal;
attribute float a_selected;
uniform mat4 u_view_projection;
varying vec3 v_normal;
varying float v_selected;
void main() {
    v_normal = a_normal;
    v_selected = a_selected;
    gl_Position = u_view_projection * vec4(a_position, 1.0);
}
"#;

const MESH_FRAGMENT_SHADER: &str = r#"#version 120
uniform vec3 u_light;
// The theme's colors: faces, faces seen from behind, and the selection tint with its
// strength in the fourth number.
uniform vec3 u_face;
uniform vec3 u_back;
uniform vec4 u_selected;
// 1 lights faces by which way they face, with darker sides; 0 lights them all evenly.
uniform float u_shadows;
varying vec3 v_normal;
varying float v_selected;
void main() {
    // The light follows the camera. A face seen from behind turns red, which shows
    // inside-out faces at a glance.
    vec3 color = gl_FrontFacing ? u_face : u_back;
    color = mix(color, u_selected.rgb, v_selected * u_selected.a);
    float side = gl_FrontFacing ? 1.0 : -1.0;
    vec3 n = normalize(v_normal) * side;
    // A key light near the camera, plus soft light from the sky above.
    float key = max(dot(n, u_light), 0.0);
    float sky = 0.5 + 0.5 * n.z;
    float lit = mix(0.92, 0.20 + 0.55 * key + 0.30 * sky, u_shadows);
    gl_FragColor = vec4(color * lit, 1.0);
}
"#;

/// Bright edges (selected and sharp ones) are drawn again on top, as Blender does: a
/// wide faint glow, then a thicker bright core. Each is a width in pixels and a
/// strength. Where glowing edges meet, the glow adds up, so corners stand out.
pub const EDGE_GLOW: [(f32, f32); 2] = [(7.0, 0.28), (2.5, 1.0)];

const POSITION_ATTRIBUTE: u32 = 0;
const COLOR_ATTRIBUTE: u32 = 1;
const SELECTED_ATTRIBUTE: u32 = 2;
const IDENTITY: Mat4 = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];
const GRID_DEPTH_BIAS: f32 = 0.0002;

pub struct Renderer {
    gl: glow::Context,
    program: glow::Program,
    view_projection: Option<glow::UniformLocation>,
    depth_bias: Option<glow::UniformLocation>,
    fade_center: Option<glow::UniformLocation>,
    fade_radius: Option<glow::UniformLocation>,
    glow: Option<glow::UniformLocation>,
    background_buffer: glow::Buffer,
    grid_buffer: glow::Buffer,
    /// Vertices in one plane of the grid.
    grid_vertex_count: i32,
    mesh_program: glow::Program,
    mesh_view_projection: Option<glow::UniformLocation>,
    mesh_light: Option<glow::UniformLocation>,
    mesh_shadows: Option<glow::UniformLocation>,
    mesh_face: Option<glow::UniformLocation>,
    mesh_back: Option<glow::UniformLocation>,
    mesh_selected: Option<glow::UniformLocation>,
    /// The theme the grid and background were made with.
    theme_version: std::cell::Cell<u64>,
    triangle_buffer: glow::Buffer,
    triangle_vertex_count: i32,
    edge_buffer: glow::Buffer,
    edge_vertex_count: i32,
    point_buffer: glow::Buffer,
    point_vertex_count: i32,
    /// With anti-aliasing, picking reads depth from here: depth read straight from a
    /// multisampled window is not reliable on every driver.
    pick_target: Option<PickTarget>,
}

/// An off-screen depth buffer for picking, and the size it was last given.
struct PickTarget {
    framebuffer: glow::Framebuffer,
    depth: glow::Renderbuffer,
    size: std::cell::Cell<(i32, i32)>,
}

/// One view's camera settings, for drawing.
pub struct View {
    pub view_projection: Mat4,
    pub light: [f32; 3],
    pub plane: GridPlane,
    /// The grid fades out around this point...
    pub fade_center: [f32; 3],
    /// ...and is gone at this distance from it.
    pub fade_radius: f32,
    /// How far points are pulled toward the camera, in depth-buffer units.
    pub point_depth_bias: f32,
    /// Edges, points and selection colors. Off in View mode.
    pub overlays: bool,
    /// The grid, which the settings can turn off.
    pub grid: bool,
    /// Darker sides on shapes, which the settings can turn off.
    pub shadows: bool,
    /// Plain dark edge lines even when `overlays` is off (View mode with shadows off).
    pub plain_edges: bool,
}

/// The background's square: the theme's bottom color below, its top color above.
pub fn background_corners() -> [f32; 24] {
    let t = palette::rgb(palette::BACKGROUND_TOP);
    let b = palette::rgb(palette::BACKGROUND_BOTTOM);
    [
        -1.0, -1.0, 0.0, b[0], b[1], b[2], 1.0, -1.0, 0.0, b[0], b[1], b[2], -1.0, 1.0, 0.0, t[0],
        t[1], t[2], 1.0, 1.0, 0.0, t[0], t[1], t[2],
    ]
}

fn as_bytes(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_ne_bytes()).collect()
}

/// What the graphics driver reports. Written to the log and the title bar, so the
/// owner can report it.
pub struct GlInfo {
    pub version: String,
    pub renderer: String,
    pub vendor: String,
    pub shading_language: String,
}

/// Reads "major.minor" from the start of a GL_VERSION string such as
/// "3.1.0 - Build 9.17.10.4459".
pub fn parse_version(text: &str) -> Option<(u32, u32)> {
    let mut parts = text.split(|c: char| !c.is_ascii_digit());
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    Some((major, minor))
}

pub fn supports_required_version(major: u32, minor: u32) -> bool {
    (major, minor) >= (2, 1)
}

impl Renderer {
    /// Returns the driver's report as well as the renderer, so a caller can
    /// log it even if setup fails later.
    pub fn new(gl: glow::Context) -> Result<(Self, GlInfo), String> {
        // SAFETY for this block and the methods below: glow's calls are unsafe
        // because they need a current OpenGL context. main keeps the context
        // current on this thread for as long as the renderer exists.
        unsafe {
            let version = gl.get_parameter_string(glow::VERSION);
            let (major, minor) = parse_version(&version)
                .ok_or_else(|| format!("Could not read the OpenGL version: {version:?}"))?;
            let supported = supports_required_version(major, minor);
            let info = GlInfo {
                renderer: gl.get_parameter_string(glow::RENDERER),
                vendor: gl.get_parameter_string(glow::VENDOR),
                // OpenGL 1.1 has no shading language, and asking stops the app.
                shading_language: if supported {
                    gl.get_parameter_string(glow::SHADING_LANGUAGE_VERSION)
                } else {
                    "none".to_string()
                },
                version,
            };
            if !supported {
                return Err(format!(
                    "Mixer needs OpenGL 2.1, but the graphics driver only offers {}.\n\
                     Graphics chip: {}\n\nUpdating the graphics driver through Windows Update \
                     or the maker's website may fix this.",
                    info.version, info.renderer
                ));
            }

            let program = create_program(
                &gl,
                VERTEX_SHADER,
                FRAGMENT_SHADER,
                &[
                    (POSITION_ATTRIBUTE, "a_position"),
                    (COLOR_ATTRIBUTE, "a_color"),
                ],
            )?;
            let mesh_program = create_program(
                &gl,
                MESH_VERTEX_SHADER,
                MESH_FRAGMENT_SHADER,
                &[
                    (POSITION_ATTRIBUTE, "a_position"),
                    (COLOR_ATTRIBUTE, "a_normal"),
                    // Left to the driver, this could share a slot with another attribute.
                    (SELECTED_ATTRIBUTE, "a_selected"),
                ],
            )?;
            let view_projection = gl.get_uniform_location(program, "u_view_projection");

            // All three grid planes share one buffer, one after another.
            let vertices: Vec<f32> = GridPlane::ALL.into_iter().flat_map(grid_vertices).collect();
            let grid_buffer = gl.create_buffer()?;
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(grid_buffer));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, &as_bytes(&vertices), glow::STATIC_DRAW);

            gl.enable(glow::DEPTH_TEST);
            gl.enable(glow::PROGRAM_POINT_SIZE);

            let renderer = Self {
                depth_bias: gl.get_uniform_location(program, "u_depth_bias"),
                fade_center: gl.get_uniform_location(program, "u_fade_center"),
                fade_radius: gl.get_uniform_location(program, "u_fade_radius"),
                glow: gl.get_uniform_location(program, "u_glow"),
                background_buffer: gl.create_buffer()?,
                theme_version: std::cell::Cell::new(u64::MAX),
                mesh_face: gl.get_uniform_location(mesh_program, "u_face"),
                mesh_back: gl.get_uniform_location(mesh_program, "u_back"),
                mesh_selected: gl.get_uniform_location(mesh_program, "u_selected"),
                mesh_view_projection: gl.get_uniform_location(mesh_program, "u_view_projection"),
                mesh_light: gl.get_uniform_location(mesh_program, "u_light"),
                mesh_shadows: gl.get_uniform_location(mesh_program, "u_shadows"),
                triangle_buffer: gl.create_buffer()?,
                triangle_vertex_count: 0,
                edge_buffer: gl.create_buffer()?,
                edge_vertex_count: 0,
                point_buffer: gl.create_buffer()?,
                point_vertex_count: 0,
                pick_target: None,
                mesh_program,
                program,
                view_projection,
                grid_buffer,
                grid_vertex_count: (vertices.len() / FLOATS_PER_VERTEX / GridPlane::ALL.len())
                    as i32,
                gl,
            };
            Ok((renderer, info))
        }
    }

    pub fn gl(&self) -> &glow::Context {
        &self.gl
    }

    /// Replaces all the mesh data on the graphics card.
    pub fn upload_scene(&mut self, scene: &Scene) {
        unsafe {
            let gl = &self.gl;
            for (buffer, data) in [
                (self.triangle_buffer, &scene.triangles),
                (self.edge_buffer, &scene.edges),
                (self.point_buffer, &scene.points),
            ] {
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(buffer));
                gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, &as_bytes(data), glow::DYNAMIC_DRAW);
            }
        }
        self.triangle_vertex_count = (scene.triangles.len() / FLOATS_PER_TRIANGLE_VERTEX) as i32;
        self.edge_vertex_count = (scene.edges.len() / FLOATS_PER_LINE_VERTEX) as i32;
        self.point_vertex_count = (scene.points.len() / FLOATS_PER_LINE_VERTEX) as i32;
    }

    /// Sends only the parts of the scene that changed.
    pub fn update_scene(&self, scene: &Scene, changed: &Changed) {
        unsafe {
            let gl = &self.gl;
            for (buffer, data, ranges) in [
                (self.triangle_buffer, &scene.triangles, &changed.triangles),
                (self.edge_buffer, &scene.edges, &changed.edges),
                (self.point_buffer, &scene.points, &changed.points),
            ] {
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(buffer));
                for range in ranges {
                    gl.buffer_sub_data_u8_slice(
                        glow::ARRAY_BUFFER,
                        (range.start * size_of::<f32>()) as i32,
                        &as_bytes(&data[range.clone()]),
                    );
                }
            }
        }
    }

    /// Draws only the faces' depth and reads it back, for checking what is hidden.
    /// The next normal frame replaces what this leaves in the buffers.
    /// How many samples per pixel the window has: 0 or 1 without anti-aliasing.
    pub fn samples(&self) -> i32 {
        unsafe { self.gl.get_parameter_i32(glow::SAMPLES) }
    }

    /// Sets up picking for a window with anti-aliasing. Returns an error if the
    /// graphics driver cannot, in which case picking reads the window as before.
    pub fn prepare_for_multisampling(&mut self) -> Result<(), String> {
        let gl = &self.gl;
        unsafe {
            let framebuffer = gl.create_framebuffer()?;
            let depth = gl.create_renderbuffer()?;
            gl.bind_renderbuffer(glow::RENDERBUFFER, Some(depth));
            gl.renderbuffer_storage(glow::RENDERBUFFER, glow::DEPTH_COMPONENT24, 1, 1);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
            gl.framebuffer_renderbuffer(
                glow::FRAMEBUFFER,
                glow::DEPTH_ATTACHMENT,
                glow::RENDERBUFFER,
                Some(depth),
            );
            // Depth only: no color to draw or read.
            gl.draw_buffer(glow::NONE);
            gl.read_buffer(glow::NONE);
            let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            gl.bind_renderbuffer(glow::RENDERBUFFER, None);
            if status != glow::FRAMEBUFFER_COMPLETE {
                gl.delete_framebuffer(framebuffer);
                gl.delete_renderbuffer(depth);
                return Err(format!(
                    "off-screen depth buffer not available ({status:#x})"
                ));
            }
            self.pick_target = Some(PickTarget {
                framebuffer,
                depth,
                size: std::cell::Cell::new((1, 1)),
            });
        }
        Ok(())
    }

    pub fn read_depth(&self, width: i32, height: i32, view_projection: &Mat4) -> Vec<f32> {
        let gl = &self.gl;
        let mut bytes = vec![0u8; width.max(0) as usize * height.max(0) as usize * 4];
        unsafe {
            if let Some(target) = &self.pick_target {
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(target.framebuffer));
                if target.size.get() != (width, height) {
                    gl.bind_renderbuffer(glow::RENDERBUFFER, Some(target.depth));
                    gl.renderbuffer_storage(
                        glow::RENDERBUFFER,
                        glow::DEPTH_COMPONENT24,
                        width.max(1),
                        height.max(1),
                    );
                    gl.bind_renderbuffer(glow::RENDERBUFFER, None);
                    target.size.set((width, height));
                }
            }
            gl.viewport(0, 0, width, height);
            gl.clear(glow::DEPTH_BUFFER_BIT);
            gl.color_mask(false, false, false, false);
            if self.triangle_vertex_count > 0 {
                gl.use_program(Some(self.mesh_program));
                gl.uniform_matrix_4_f32_slice(
                    self.mesh_view_projection.as_ref(),
                    false,
                    view_projection,
                );
                let stride = FLOATS_PER_TRIANGLE_VERTEX as i32 * size_of::<f32>() as i32;
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.triangle_buffer));
                gl.enable_vertex_attrib_array(POSITION_ATTRIBUTE);
                gl.vertex_attrib_pointer_f32(POSITION_ATTRIBUTE, 3, glow::FLOAT, false, stride, 0);
                gl.draw_arrays(glow::TRIANGLES, 0, self.triangle_vertex_count);
                gl.disable_vertex_attrib_array(POSITION_ATTRIBUTE);
            }
            gl.color_mask(true, true, true, true);
            gl.pixel_store_i32(glow::PACK_ALIGNMENT, 4);
            gl.read_pixels(
                0,
                0,
                width,
                height,
                glow::DEPTH_COMPONENT,
                glow::FLOAT,
                glow::PixelPackData::Slice(Some(&mut bytes)),
            );
            if self.pick_target.is_some() {
                gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            }
        }
        bytes
            .chunks_exact(4)
            .map(|b| f32::from_ne_bytes([b[0], b[1], b[2], b[3]]))
            .collect()
    }

    pub fn render(&self, width: i32, height: i32, view: &View) {
        let gl = &self.gl;
        let size = size_of::<f32>() as i32;
        if self.theme_version.get() != palette::version() {
            self.theme_version.set(palette::version());
            let grid: Vec<f32> = GridPlane::ALL.into_iter().flat_map(grid_vertices).collect();
            // SAFETY: as for the other calls.
            unsafe {
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.grid_buffer));
                gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, &as_bytes(&grid), glow::STATIC_DRAW);
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.background_buffer));
                gl.buffer_data_u8_slice(
                    glow::ARRAY_BUFFER,
                    &as_bytes(&background_corners()),
                    glow::STATIC_DRAW,
                );
                let [r, g, b] = palette::rgb(palette::BACKGROUND_BOTTOM);
                gl.clear_color(r, g, b, 1.0);
            }
        }
        unsafe {
            gl.viewport(0, 0, width, height);
            gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);

            gl.enable_vertex_attrib_array(POSITION_ATTRIBUTE);
            gl.enable_vertex_attrib_array(COLOR_ATTRIBUTE);

            // The background: a gradient on a square covering the whole view.
            let stride = FLOATS_PER_VERTEX as i32 * size;
            gl.use_program(Some(self.program));
            gl.uniform_matrix_4_f32_slice(self.view_projection.as_ref(), false, &IDENTITY);
            gl.uniform_1_f32(self.depth_bias.as_ref(), 0.0);
            gl.uniform_1_f32(self.fade_radius.as_ref(), 0.0);
            gl.disable(glow::DEPTH_TEST);
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.background_buffer));
            gl.vertex_attrib_pointer_f32(POSITION_ATTRIBUTE, 3, glow::FLOAT, false, stride, 0);
            gl.vertex_attrib_pointer_f32(COLOR_ATTRIBUTE, 3, glow::FLOAT, false, stride, 3 * size);
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
            gl.enable(glow::DEPTH_TEST);

            // The grid, blended so that it can fade out.
            gl.uniform_matrix_4_f32_slice(
                self.view_projection.as_ref(),
                false,
                &view.view_projection,
            );
            gl.uniform_1_f32(self.depth_bias.as_ref(), GRID_DEPTH_BIAS);
            let [cx, cy, cz] = view.fade_center;
            gl.uniform_3_f32(self.fade_center.as_ref(), cx, cy, cz);
            gl.uniform_1_f32(self.fade_radius.as_ref(), view.fade_radius);
            gl.enable(glow::BLEND);
            gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
            gl.depth_mask(false);
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.grid_buffer));
            gl.vertex_attrib_pointer_f32(POSITION_ATTRIBUTE, 3, glow::FLOAT, false, stride, 0);
            gl.vertex_attrib_pointer_f32(COLOR_ATTRIBUTE, 3, glow::FLOAT, false, stride, 3 * size);
            let first = GridPlane::ALL
                .iter()
                .position(|&p| p == view.plane)
                .unwrap_or(0) as i32;
            if view.grid {
                gl.draw_arrays(
                    glow::LINES,
                    first * self.grid_vertex_count,
                    self.grid_vertex_count,
                );
            }
            gl.depth_mask(true);
            gl.disable(glow::BLEND);
            gl.uniform_1_f32(self.fade_radius.as_ref(), 0.0);

            // Faces, pushed back slightly so that their edges show cleanly on top.
            if self.triangle_vertex_count > 0 {
                gl.use_program(Some(self.mesh_program));
                gl.uniform_matrix_4_f32_slice(
                    self.mesh_view_projection.as_ref(),
                    false,
                    &view.view_projection,
                );
                let [x, y, z] = view.light;
                gl.uniform_3_f32(self.mesh_light.as_ref(), x, y, z);
                let [r, g, b] = palette::rgb(palette::FACE);
                gl.uniform_3_f32(self.mesh_face.as_ref(), r, g, b);
                let [r, g, b] = palette::rgb(palette::FACE_BACK);
                gl.uniform_3_f32(self.mesh_back.as_ref(), r, g, b);
                let [r, g, b, a] = palette::get(palette::FACE_SELECTED);
                gl.uniform_4_f32(self.mesh_selected.as_ref(), r, g, b, a);
                gl.uniform_1_f32(
                    self.mesh_shadows.as_ref(),
                    if view.shadows { 1.0 } else { 0.0 },
                );
                let stride = FLOATS_PER_TRIANGLE_VERTEX as i32 * size;
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.triangle_buffer));
                gl.vertex_attrib_pointer_f32(POSITION_ATTRIBUTE, 3, glow::FLOAT, false, stride, 0);
                gl.vertex_attrib_pointer_f32(
                    COLOR_ATTRIBUTE,
                    3,
                    glow::FLOAT,
                    false,
                    stride,
                    3 * size,
                );
                if view.overlays {
                    gl.enable_vertex_attrib_array(SELECTED_ATTRIBUTE);
                    gl.vertex_attrib_pointer_f32(
                        SELECTED_ATTRIBUTE,
                        1,
                        glow::FLOAT,
                        false,
                        stride,
                        6 * size,
                    );
                } else {
                    // Every face unselected.
                    gl.vertex_attrib_1_f32(SELECTED_ATTRIBUTE, 0.0);
                }
                gl.enable(glow::POLYGON_OFFSET_FILL);
                gl.polygon_offset(1.0, 1.0);
                gl.draw_arrays(glow::TRIANGLES, 0, self.triangle_vertex_count);
                gl.disable(glow::POLYGON_OFFSET_FILL);
                gl.disable_vertex_attrib_array(SELECTED_ATTRIBUTE);
            }

            // In View mode, plain edge lines all in one color, when the settings ask.
            if !view.overlays && view.plain_edges && self.edge_vertex_count > 0 {
                gl.use_program(Some(self.program));
                gl.uniform_1_f32(self.depth_bias.as_ref(), 0.0);
                let stride = FLOATS_PER_LINE_VERTEX as i32 * size;
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.edge_buffer));
                gl.vertex_attrib_pointer_f32(POSITION_ATTRIBUTE, 3, glow::FLOAT, false, stride, 0);
                gl.disable_vertex_attrib_array(COLOR_ATTRIBUTE);
                let [r, g, b] = palette::rgb(palette::EDGE_PLAIN);
                gl.vertex_attrib_3_f32(COLOR_ATTRIBUTE, r, g, b);
                gl.draw_arrays(glow::LINES, 0, self.edge_vertex_count);
                gl.enable_vertex_attrib_array(COLOR_ATTRIBUTE);
            }

            // Edges and points, using the grid's shader.
            if view.overlays && (self.edge_vertex_count > 0 || self.point_vertex_count > 0) {
                gl.use_program(Some(self.program));
                gl.uniform_1_f32(self.depth_bias.as_ref(), 0.0);
                let stride = FLOATS_PER_LINE_VERTEX as i32 * size;
                for (buffer, count, mode) in [
                    (self.edge_buffer, self.edge_vertex_count, glow::LINES),
                    (self.point_buffer, self.point_vertex_count, glow::POINTS),
                ] {
                    if count > 0 {
                        gl.bind_buffer(glow::ARRAY_BUFFER, Some(buffer));
                        gl.vertex_attrib_pointer_f32(
                            POSITION_ATTRIBUTE,
                            3,
                            glow::FLOAT,
                            false,
                            stride,
                            0,
                        );
                        gl.vertex_attrib_pointer_f32(
                            COLOR_ATTRIBUTE,
                            3,
                            glow::FLOAT,
                            false,
                            stride,
                            3 * size,
                        );
                        // Points sit a little in front, so that a sloped face does
                        // not cut its center dot in half.
                        let bias = if mode == glow::POINTS {
                            view.point_depth_bias
                        } else {
                            0.0
                        };
                        gl.uniform_1_f32(self.depth_bias.as_ref(), bias);
                        gl.draw_arrays(mode, 0, count);
                        if mode == glow::LINES {
                            // The glow on bright edges: the same lines, wider and faint.
                            gl.enable(glow::BLEND);
                            gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
                            gl.depth_mask(false);
                            gl.depth_func(glow::LEQUAL);
                            for (width, strength) in EDGE_GLOW {
                                gl.line_width(width);
                                gl.uniform_1_f32(self.glow.as_ref(), strength);
                                gl.draw_arrays(mode, 0, count);
                            }
                            gl.uniform_1_f32(self.glow.as_ref(), 0.0);
                            gl.line_width(1.0);
                            gl.depth_func(glow::LESS);
                            gl.depth_mask(true);
                            gl.disable(glow::BLEND);
                        }
                    }
                }
            }

            gl.disable_vertex_attrib_array(COLOR_ATTRIBUTE);
            gl.disable_vertex_attrib_array(POSITION_ATTRIBUTE);
        }
    }

    /// The first OpenGL error waiting to be reported, if any.
    pub fn error_code(&self) -> Option<u32> {
        match unsafe { self.gl.get_error() } {
            glow::NO_ERROR => None,
            code => Some(code),
        }
    }

    /// The frame as RGB bytes, with the bottom row first.
    pub fn read_pixels(&self, width: i32, height: i32) -> Vec<u8> {
        let mut pixels = vec![0u8; width as usize * height as usize * 3];
        unsafe {
            self.gl.pixel_store_i32(glow::PACK_ALIGNMENT, 1);
            self.gl.read_pixels(
                0,
                0,
                width,
                height,
                glow::RGB,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut pixels)),
            );
        }
        pixels
    }
}

/// # Safety
/// The OpenGL context must be current on this thread.
pub unsafe fn create_program(
    gl: &glow::Context,
    vertex_shader: &str,
    fragment_shader: &str,
    attributes: &[(u32, &str)],
) -> Result<glow::Program, String> {
    unsafe {
        let program = gl.create_program()?;
        let mut shaders = Vec::new();
        for (kind, source) in [
            (glow::VERTEX_SHADER, vertex_shader),
            (glow::FRAGMENT_SHADER, fragment_shader),
        ] {
            let shader = gl.create_shader(kind)?;
            gl.shader_source(shader, source);
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                return Err(format!(
                    "A graphics shader failed to compile:\n{}",
                    gl.get_shader_info_log(shader)
                ));
            }
            gl.attach_shader(program, shader);
            shaders.push(shader);
        }
        for (location, name) in attributes {
            gl.bind_attrib_location(program, *location, name);
        }
        gl.link_program(program);
        if !gl.get_program_link_status(program) {
            return Err(format!(
                "A graphics shader failed to link:\n{}",
                gl.get_program_info_log(program)
            ));
        }
        for shader in shaders {
            gl.detach_shader(program, shader);
            gl.delete_shader(shader);
        }
        Ok(program)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_versions_as_drivers_report_them() {
        assert_eq!(parse_version("3.1.0 - Build 9.17.10.4459"), Some((3, 1)));
        assert_eq!(parse_version("2.1"), Some((2, 1)));
        assert_eq!(
            parse_version("4.5 (Compatibility Profile) Mesa 25.2.8"),
            Some((4, 5))
        );
        assert_eq!(parse_version("1.1.0"), Some((1, 1)));
        assert_eq!(parse_version("unknown"), None);
        assert_eq!(parse_version(""), None);
    }

    #[test]
    fn accepts_only_opengl_2_1_and_newer() {
        assert!(supports_required_version(2, 1));
        assert!(supports_required_version(3, 0));
        assert!(supports_required_version(4, 6));
        assert!(!supports_required_version(2, 0));
        assert!(!supports_required_version(1, 5));
    }
}
