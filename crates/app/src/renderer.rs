//! All OpenGL calls live here. Uses only OpenGL 2.1 features: GLSL 1.20 shaders
//! and vertex buffers, with no vertex array objects, shadows or anti-aliasing.

use glow::HasContext;

use crate::camera::Mat4;
use crate::grid::{FLOATS_PER_VERTEX, GridPlane, grid_vertices};

const VERTEX_SHADER: &str = r#"#version 120
attribute vec3 a_position;
attribute vec3 a_color;
uniform mat4 u_view_projection;
varying vec3 v_color;
void main() {
    v_color = a_color;
    gl_Position = u_view_projection * vec4(a_position, 1.0);
}
"#;

const FRAGMENT_SHADER: &str = r#"#version 120
varying vec3 v_color;
void main() {
    gl_FragColor = vec4(v_color, 1.0);
}
"#;

const POSITION_ATTRIBUTE: u32 = 0;
const COLOR_ATTRIBUTE: u32 = 1;
const BACKGROUND: [f32; 3] = [0.19, 0.19, 0.19];

pub struct Renderer {
    gl: glow::Context,
    program: glow::Program,
    view_projection: Option<glow::UniformLocation>,
    grid_buffer: glow::Buffer,
    /// Vertices in one plane of the grid.
    grid_vertex_count: i32,
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
            let info = GlInfo {
                renderer: gl.get_parameter_string(glow::RENDERER),
                vendor: gl.get_parameter_string(glow::VENDOR),
                shading_language: gl.get_parameter_string(glow::SHADING_LANGUAGE_VERSION),
                version,
            };
            if !supports_required_version(major, minor) {
                return Err(format!(
                    "This app needs OpenGL 2.1, but the graphics driver only offers {}.\n\
                     Graphics chip: {}\n\nUpdating the graphics driver may fix this.",
                    info.version, info.renderer
                ));
            }

            let program = create_program(&gl)?;
            let view_projection = gl.get_uniform_location(program, "u_view_projection");

            // All three grid planes share one buffer, one after another.
            let vertices: Vec<f32> = GridPlane::ALL.into_iter().flat_map(grid_vertices).collect();
            let bytes: Vec<u8> = vertices.iter().flat_map(|v| v.to_ne_bytes()).collect();
            let grid_buffer = gl.create_buffer()?;
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(grid_buffer));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, &bytes, glow::STATIC_DRAW);

            gl.enable(glow::DEPTH_TEST);
            gl.clear_color(BACKGROUND[0], BACKGROUND[1], BACKGROUND[2], 1.0);

            let renderer = Self {
                gl,
                program,
                view_projection,
                grid_buffer,
                grid_vertex_count: (vertices.len() / FLOATS_PER_VERTEX / GridPlane::ALL.len())
                    as i32,
            };
            Ok((renderer, info))
        }
    }

    pub fn render(&self, width: i32, height: i32, view_projection: &Mat4, plane: GridPlane) {
        let gl = &self.gl;
        unsafe {
            gl.viewport(0, 0, width, height);
            gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
            gl.use_program(Some(self.program));
            gl.uniform_matrix_4_f32_slice(self.view_projection.as_ref(), false, view_projection);

            let stride = (FLOATS_PER_VERTEX * size_of::<f32>()) as i32;
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.grid_buffer));
            gl.enable_vertex_attrib_array(POSITION_ATTRIBUTE);
            gl.enable_vertex_attrib_array(COLOR_ATTRIBUTE);
            gl.vertex_attrib_pointer_f32(POSITION_ATTRIBUTE, 3, glow::FLOAT, false, stride, 0);
            gl.vertex_attrib_pointer_f32(
                COLOR_ATTRIBUTE,
                3,
                glow::FLOAT,
                false,
                stride,
                (3 * size_of::<f32>()) as i32,
            );
            let first = GridPlane::ALL.iter().position(|&p| p == plane).unwrap_or(0) as i32;
            gl.draw_arrays(
                glow::LINES,
                first * self.grid_vertex_count,
                self.grid_vertex_count,
            );
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

unsafe fn create_program(gl: &glow::Context) -> Result<glow::Program, String> {
    unsafe {
        let program = gl.create_program()?;
        let mut shaders = Vec::new();
        for (kind, source) in [
            (glow::VERTEX_SHADER, VERTEX_SHADER),
            (glow::FRAGMENT_SHADER, FRAGMENT_SHADER),
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
        gl.bind_attrib_location(program, POSITION_ATTRIBUTE, "a_position");
        gl.bind_attrib_location(program, COLOR_ATTRIBUTE, "a_color");
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
