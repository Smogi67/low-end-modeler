//! Draws Dear ImGui with OpenGL 2.1. The ready-made renderer for it needs an older
//! glow than this app uses, and this needs only a screenful of code.

use glow::HasContext;
use imgui::{DrawCmd, DrawVert};

const VERTEX_SHADER: &str = r#"#version 120
attribute vec2 a_position;
attribute vec2 a_uv;
attribute vec4 a_color;
uniform vec2 u_screen;
varying vec2 v_uv;
varying vec4 v_color;
void main() {
    v_uv = a_uv;
    v_color = a_color;
    gl_Position = vec4(a_position.x / u_screen.x * 2.0 - 1.0,
                       1.0 - a_position.y / u_screen.y * 2.0, 0.0, 1.0);
}
"#;

const FRAGMENT_SHADER: &str = r#"#version 120
uniform sampler2D u_texture;
varying vec2 v_uv;
varying vec4 v_color;
void main() {
    gl_FragColor = v_color * texture2D(u_texture, v_uv);
}
"#;

const POSITION: u32 = 0;
const UV: u32 = 1;
const COLOR: u32 = 2;

pub struct ImguiGl {
    program: glow::Program,
    screen: Option<glow::UniformLocation>,
    texture: glow::Texture,
    vertex_buffer: glow::Buffer,
    index_buffer: glow::Buffer,
}

impl ImguiGl {
    pub fn new(gl: &glow::Context, context: &mut imgui::Context) -> Result<Self, String> {
        // SAFETY: the OpenGL context is current on this thread for the app's lifetime.
        unsafe {
            let program = crate::renderer::create_program(
                gl,
                VERTEX_SHADER,
                FRAGMENT_SHADER,
                &[(POSITION, "a_position"), (UV, "a_uv"), (COLOR, "a_color")],
            )?;
            let screen = gl.get_uniform_location(program, "u_screen");

            let texture = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            for (name, value) in [
                (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
                (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
                (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
            ] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, name, value as i32);
            }
            let fonts = context.fonts();
            let atlas = fonts.build_rgba32_texture();
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA as i32,
                atlas.width as i32,
                atlas.height as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(atlas.data)),
            );

            Ok(Self {
                program,
                screen,
                texture,
                vertex_buffer: gl.create_buffer()?,
                index_buffer: gl.create_buffer()?,
            })
        }
    }

    pub fn render(&self, gl: &glow::Context, draw_data: &imgui::DrawData, height: i32) {
        let stride = size_of::<DrawVert>() as i32;
        // SAFETY: the context is current. Each slice view below covers exactly the
        // live data of a plain Copy array, for just the duration of the upload.
        unsafe {
            gl.disable(glow::DEPTH_TEST);
            gl.enable(glow::BLEND);
            gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
            gl.enable(glow::SCISSOR_TEST);
            gl.use_program(Some(self.program));
            gl.uniform_2_f32(
                self.screen.as_ref(),
                draw_data.display_size[0].max(1.0),
                draw_data.display_size[1].max(1.0),
            );
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.texture));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vertex_buffer));
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(self.index_buffer));
            for attribute in [POSITION, UV, COLOR] {
                gl.enable_vertex_attrib_array(attribute);
            }

            for list in draw_data.draw_lists() {
                let vertices = list.vtx_buffer();
                let indices = list.idx_buffer();
                gl.buffer_data_u8_slice(
                    glow::ARRAY_BUFFER,
                    std::slice::from_raw_parts(
                        vertices.as_ptr().cast::<u8>(),
                        size_of_val(vertices),
                    ),
                    glow::STREAM_DRAW,
                );
                gl.buffer_data_u8_slice(
                    glow::ELEMENT_ARRAY_BUFFER,
                    std::slice::from_raw_parts(indices.as_ptr().cast::<u8>(), size_of_val(indices)),
                    glow::STREAM_DRAW,
                );

                for command in list.commands() {
                    let DrawCmd::Elements { count, cmd_params } = command else {
                        continue;
                    };
                    let clip = cmd_params.clip_rect;
                    gl.scissor(
                        clip[0] as i32,
                        height - clip[3] as i32,
                        (clip[2] - clip[0]) as i32,
                        (clip[3] - clip[1]) as i32,
                    );
                    // OpenGL 2.1 has no base vertex, so move the attribute start instead.
                    let base = cmd_params.vtx_offset as i32 * stride;
                    gl.vertex_attrib_pointer_f32(POSITION, 2, glow::FLOAT, false, stride, base);
                    gl.vertex_attrib_pointer_f32(UV, 2, glow::FLOAT, false, stride, base + 8);
                    gl.vertex_attrib_pointer_f32(
                        COLOR,
                        4,
                        glow::UNSIGNED_BYTE,
                        true,
                        stride,
                        base + 16,
                    );
                    gl.draw_elements(
                        glow::TRIANGLES,
                        count as i32,
                        glow::UNSIGNED_SHORT,
                        (cmd_params.idx_offset * size_of::<imgui::DrawIdx>()) as i32,
                    );
                }
            }

            for attribute in [POSITION, UV, COLOR] {
                gl.disable_vertex_attrib_array(attribute);
            }
            gl.disable(glow::SCISSOR_TEST);
            gl.disable(glow::BLEND);
            gl.enable(glow::DEPTH_TEST);
        }
    }
}
