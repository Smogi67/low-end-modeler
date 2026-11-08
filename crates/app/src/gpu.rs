//! The rendering engine Mixer runs with: OpenGL 2.1 (`renderer.rs` and `imgui_gl.rs`),
//! or one of the newer engines through wgpu (`wgpu_renderer.rs`). The rest of Mixer
//! talks to this and does not care which.

use sdl2::video::Window;

use crate::camera::Mat4;
use crate::imgui_gl::ImguiGl;
use crate::renderer::{Renderer, View};
use crate::scene::{Changed, Scene};
use crate::wgpu_renderer::WgpuRenderer;

pub enum Gpu {
    /// OpenGL 2.1; the interface's renderer comes once Dear ImGui exists.
    Gl {
        renderer: Box<Renderer>,
        ui: Option<ImguiGl>,
    },
    Wgpu(Box<WgpuRenderer>),
}

impl Gpu {
    /// The engine's key in settings.txt.
    pub fn key(&self) -> &'static str {
        match self {
            Gpu::Gl { .. } => "opengl2.1",
            Gpu::Wgpu(renderer) => renderer.engine().key(),
        }
    }

    /// Gets ready to draw the interface, and builds its letters.
    pub fn prepare_ui(&mut self, imgui: &mut imgui::Context) -> Result<(), String> {
        match self {
            Gpu::Gl { renderer, ui } => {
                *ui = Some(ImguiGl::new(renderer.gl(), imgui)?);
            }
            Gpu::Wgpu(renderer) => renderer.load_fonts(imgui),
        }
        Ok(())
    }

    /// Builds the letters again, after the fonts changed.
    pub fn reload_fonts(&mut self, imgui: &mut imgui::Context) {
        match self {
            Gpu::Gl { renderer, ui } => {
                if let Some(ui) = ui {
                    ui.reload_fonts(renderer.gl(), imgui);
                }
            }
            Gpu::Wgpu(renderer) => renderer.load_fonts(imgui),
        }
    }

    pub fn upload_scene(&mut self, scene: &Scene) {
        match self {
            Gpu::Gl { renderer, .. } => renderer.upload_scene(scene),
            Gpu::Wgpu(renderer) => renderer.upload_scene(scene),
        }
    }

    pub fn update_scene(&self, scene: &Scene, changed: &Changed) {
        match self {
            Gpu::Gl { renderer, .. } => renderer.update_scene(scene, changed),
            Gpu::Wgpu(renderer) => renderer.update_scene(scene, changed),
        }
    }

    /// Draws the 3D view and the interface. It shows on `present`.
    pub fn draw(&mut self, width: i32, height: i32, view: &View, draw_data: &imgui::DrawData) {
        match self {
            Gpu::Gl { renderer, ui } => {
                renderer.render(width, height, view);
                if let Some(ui) = ui {
                    ui.render(renderer.gl(), draw_data, height);
                }
            }
            Gpu::Wgpu(renderer) => renderer.render(width, height, view, draw_data),
        }
    }

    /// Shows the frame drawn last.
    pub fn present(&mut self, window: &Window) {
        match self {
            Gpu::Gl { .. } => window.gl_swap_window(),
            Gpu::Wgpu(renderer) => renderer.present(),
        }
    }

    /// The faces' depth, bottom row first, for checking what is hidden.
    pub fn read_depth(&self, width: i32, height: i32, view_projection: &Mat4) -> Vec<f32> {
        match self {
            Gpu::Gl { renderer, .. } => renderer.read_depth(width, height, view_projection),
            Gpu::Wgpu(renderer) => renderer.read_depth(width, height, view_projection),
        }
    }

    /// The frame drawn last as RGB bytes, bottom row first.
    pub fn read_pixels(&self, width: i32, height: i32) -> Vec<u8> {
        match self {
            Gpu::Gl { renderer, .. } => renderer.read_pixels(width, height),
            Gpu::Wgpu(renderer) => renderer.read_pixels(width, height),
        }
    }

    /// An error the graphics driver reported, if any, for the log.
    pub fn error(&self) -> Option<String> {
        match self {
            Gpu::Gl { renderer, .. } => renderer
                .error_code()
                .map(|code| format!("OpenGL error code: {code:#x}")),
            Gpu::Wgpu(renderer) => renderer
                .take_error()
                .map(|text| format!("{} error: {text}", renderer.engine().name())),
        }
    }
}
