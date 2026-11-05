//! Ties the document, camera, panel and drawing together.
//!
//! Spec section 4 requires near 0% CPU when idle. The main loop sleeps until an event
//! arrives, and only asks for frames if the event changed something. Dear ImGui takes
//! a couple of frames to show the result of a click, so such events ask for a few.

use std::path::PathBuf;
use std::time::Instant;

use imgui::{Key, MouseButton as ImguiButton};
use modeler_mesh::{Document, Mesh};
use sdl2::event::Event;
use sdl2::keyboard::{Keycode, Mod};
use sdl2::mouse::{MouseButton, MouseUtil, MouseWheelDirection};
use sdl2::video::Window;

use crate::camera::Camera;
use crate::controls::{Button, Drag, Modifiers, drag_for};
use crate::export::{warnings, write_obj};
use crate::imgui_gl::ImguiGl;
use crate::renderer::{Renderer, View};
use crate::scene::{edge_vertices, triangle_vertices};
use crate::ui::{Action, PANEL_MARGIN, PANEL_WIDTH, PanelInfo, PanelState, panel};

const VIEW_STEP_DEGREES: f32 = 15.0;
/// Frames to draw after an interaction, so that Dear ImGui can settle.
const UI_SETTLE_FRAMES: u32 = 3;

pub struct App {
    renderer: Renderer,
    imgui: imgui::Context,
    imgui_gl: ImguiGl,
    document: Document,
    state: PanelState,
    camera: Camera,
    drag: Option<Drag>,
    mesh_changed: bool,
    frames_wanted: u32,
    frames_drawn: u64,
    last_frame: Instant,
    panel_height: f32,
    pointer_over_panel: bool,
    export_folder: PathBuf,
    export_folder_text: String,
    warnings: Vec<String>,
}

impl App {
    pub fn new(renderer: Renderer) -> Result<Self, String> {
        let mut imgui = imgui::Context::create();
        // No settings file: nothing to save, and it spares the disk.
        imgui.set_ini_filename(None);
        imgui.set_log_filename(None);
        imgui.io_mut().config_input_text_cursor_blink = false;
        let imgui_gl = ImguiGl::new(renderer.gl(), &mut imgui)?;
        let export_folder = std::env::current_exe()
            .ok()
            .and_then(|path| Some(path.parent()?.join("exports")))
            .unwrap_or_else(|| PathBuf::from("exports"));
        let export_folder_text = export_folder.display().to_string();
        Ok(Self {
            renderer,
            imgui,
            imgui_gl,
            document: Document::new(Mesh::new()),
            state: PanelState::default(),
            camera: Camera::default(),
            drag: None,
            mesh_changed: true,
            frames_wanted: UI_SETTLE_FRAMES,
            frames_drawn: 0,
            last_frame: Instant::now(),
            panel_height: 0.0,
            pointer_over_panel: false,
            export_folder,
            export_folder_text,
            warnings: Vec::new(),
        })
    }

    pub fn frames_drawn(&self) -> u64 {
        self.frames_drawn
    }

    pub fn wants_frame(&self) -> bool {
        self.frames_wanted > 0
    }

    pub fn request_frames(&mut self) {
        self.frames_wanted = self.frames_wanted.max(UI_SETTLE_FRAMES);
    }

    /// The OpenGL error waiting to be reported, if any.
    pub fn gl_error(&self) -> Option<u32> {
        self.renderer.error_code()
    }

    /// Draws one frame into the back buffer. The caller swaps it onto the screen.
    pub fn draw(&mut self, window: &Window) {
        let (width, height) = window.drawable_size();
        let (width, height) = (width as i32, height as i32);

        let io = self.imgui.io_mut();
        io.display_size = [width as f32, height as f32];
        let now = Instant::now();
        io.delta_time = (now - self.last_frame).as_secs_f32().max(0.001);
        self.last_frame = now;

        let mesh = self.document.mesh();
        let info = PanelInfo {
            triangles: mesh.triangle_count(),
            vertices: mesh.vertex_count(),
            faces: mesh.face_count(),
            warnings: &self.warnings,
            undo_label: self.document.undo_label(),
            redo_label: self.document.redo_label(),
            export_folder: &self.export_folder_text,
        };
        let ui = self.imgui.new_frame();
        let (action, panel_height) = panel(ui, &mut self.state, &info);
        self.panel_height = panel_height;
        let draw_data = self.imgui.render();

        // The action's result appears in the frames that follow.
        if let Some(action) = action {
            Self::apply_action(
                action,
                &mut self.document,
                &mut self.state,
                &self.export_folder,
                &mut self.mesh_changed,
            );
            self.frames_wanted = self.frames_wanted.max(UI_SETTLE_FRAMES);
        }
        if self.mesh_changed {
            let mesh = self.document.mesh();
            self.renderer
                .upload_mesh(&triangle_vertices(mesh), &edge_vertices(mesh));
            self.warnings = warnings(mesh.triangle_count(), mesh.open_edge_count());
            self.mesh_changed = false;
        }

        let aspect = width as f32 / height.max(1) as f32;
        let view = View {
            view_projection: self.camera.view_projection(aspect),
            light: self.camera.toward_camera(),
            plane: self.camera.grid_plane(),
        };
        self.renderer.render(width, height, &view);
        self.imgui_gl.render(self.renderer.gl(), draw_data, height);

        self.frames_drawn += 1;
        self.frames_wanted = self.frames_wanted.saturating_sub(1);
    }

    pub fn read_pixels(&self, window: &Window) -> (i32, i32, Vec<u8>) {
        let (width, height) = window.drawable_size();
        let (width, height) = (width as i32, height as i32);
        (width, height, self.renderer.read_pixels(width, height))
    }

    fn apply_action(
        action: Action,
        document: &mut Document,
        state: &mut PanelState,
        export_folder: &std::path::Path,
        mesh_changed: &mut bool,
    ) {
        match action {
            Action::Add(shape) => {
                let primitive = state.primitive(shape);
                let center = state.location;
                match document.edit(shape.edit_label(), |mesh| {
                    mesh.add_primitive(primitive, center).map(|_| ())
                }) {
                    Ok(()) => *mesh_changed = true,
                    Err(error) => state.message = format!("Could not add the shape: {error}"),
                }
            }
            Action::Undo => *mesh_changed |= document.undo().is_some(),
            Action::Redo => *mesh_changed |= document.redo().is_some(),
            Action::Export => {
                state.message = match write_obj(
                    document.mesh(),
                    export_folder,
                    &state.file_name,
                    state.scale,
                ) {
                    Ok(path) => {
                        let name = path.file_name().unwrap_or_default().to_string_lossy();
                        format!("Saved {name}")
                    }
                    Err(error) => error,
                };
            }
            #[cfg(windows)]
            Action::OpenExportFolder => {
                let _ = std::process::Command::new("explorer")
                    .arg(export_folder)
                    .spawn();
            }
        }
    }

    /// Handles one event. `mods` is the current state of Shift, Ctrl and Alt.
    pub fn handle_event(&mut self, event: &Event, mouse: &MouseUtil, mods: Mod) {
        self.feed_imgui(event);
        let io = self.imgui.io();
        let (ui_mouse, ui_keyboard) = (io.want_capture_mouse, io.want_capture_keyboard);

        match *event {
            Event::MouseButtonDown { mouse_btn, .. } => {
                self.request_frames();
                if self.drag.is_none() && !ui_mouse && !self.pointer_over_panel {
                    let button = match mouse_btn {
                        MouseButton::Left => Button::Left,
                        MouseButton::Middle => Button::Middle,
                        MouseButton::Right => Button::Right,
                        _ => Button::Other,
                    };
                    self.drag = drag_for(button, modifiers(mods));
                    if self.drag.is_some() {
                        // Keeps the drag going if the mouse leaves the window.
                        mouse.capture(true);
                    }
                }
            }
            Event::MouseButtonUp { .. } => {
                self.request_frames();
                if self.drag.take().is_some() {
                    mouse.capture(false);
                }
            }
            Event::MouseMotion {
                x, y, xrel, yrel, ..
            } => {
                let (dx, dy) = (xrel as f32, yrel as f32);
                match self.drag {
                    Some(Drag::Orbit) => self.camera.orbit_pixels(dx, dy),
                    Some(Drag::Pan) => {
                        let height = self.imgui.io().display_size[1];
                        self.camera.pan_pixels(dx, dy, height);
                    }
                    Some(Drag::Zoom) => self.camera.zoom_pixels(dy),
                    None => {}
                }
                if self.drag.is_some() {
                    self.frames_wanted = self.frames_wanted.max(1);
                }
                // Moving over empty viewport needs no drawing. The panel highlights
                // its buttons, so it does, as does leaving it or using a slider.
                let over = in_panel(x as f32, y as f32, self.panel_height);
                if over || self.pointer_over_panel || ui_mouse {
                    self.request_frames();
                }
                self.pointer_over_panel = over;
            }
            Event::MouseWheel { y, direction, .. } if y != 0 => {
                self.request_frames();
                if !ui_mouse && !self.pointer_over_panel {
                    let steps = if direction == MouseWheelDirection::Flipped {
                        -y
                    } else {
                        y
                    };
                    self.camera.zoom_steps(steps as f32);
                }
            }
            Event::KeyDown {
                keycode: Some(key),
                keymod,
                ..
            } => {
                self.request_frames();
                if !ui_keyboard {
                    let mods = modifiers(keymod);
                    if !self.apply_shortcut(key, mods) {
                        apply_view_key(&mut self.camera, key, mods);
                    }
                }
            }
            Event::KeyUp { .. } | Event::TextInput { .. } => self.request_frames(),
            _ => {}
        }
    }

    fn apply_shortcut(&mut self, key: Keycode, mods: Modifiers) -> bool {
        let action = match shortcut(key, mods) {
            Some(action) => action,
            None => return false,
        };
        Self::apply_action(
            action,
            &mut self.document,
            &mut self.state,
            &self.export_folder,
            &mut self.mesh_changed,
        );
        true
    }

    fn feed_imgui(&mut self, event: &Event) {
        let io = self.imgui.io_mut();
        match event {
            Event::MouseMotion { x, y, .. } => {
                io.add_mouse_pos_event([*x as f32, *y as f32]);
            }
            Event::MouseButtonDown { mouse_btn, .. } | Event::MouseButtonUp { mouse_btn, .. } => {
                let down = matches!(event, Event::MouseButtonDown { .. });
                let button = match mouse_btn {
                    MouseButton::Left => Some(ImguiButton::Left),
                    MouseButton::Right => Some(ImguiButton::Right),
                    MouseButton::Middle => Some(ImguiButton::Middle),
                    _ => None,
                };
                if let Some(button) = button {
                    io.add_mouse_button_event(button, down);
                }
            }
            Event::MouseWheel { y, .. } => io.add_mouse_wheel_event([0.0, *y as f32]),
            Event::TextInput { text, .. } => {
                for character in text.chars() {
                    io.add_input_character(character);
                }
            }
            Event::KeyDown {
                keycode: Some(key),
                keymod,
                ..
            }
            | Event::KeyUp {
                keycode: Some(key),
                keymod,
                ..
            } => {
                let down = matches!(event, Event::KeyDown { .. });
                let mods = modifiers(*keymod);
                io.add_key_event(Key::ModCtrl, mods.ctrl);
                io.add_key_event(Key::ModShift, mods.shift);
                io.add_key_event(Key::ModAlt, mods.alt);
                if let Some(key) = imgui_key(*key) {
                    io.add_key_event(key, down);
                }
            }
            _ => {}
        }
    }
}

fn in_panel(x: f32, y: f32, panel_height: f32) -> bool {
    (PANEL_MARGIN..=PANEL_MARGIN + PANEL_WIDTH).contains(&x)
        && (PANEL_MARGIN..=PANEL_MARGIN + panel_height).contains(&y)
}

/// Keys that text fields need.
fn imgui_key(key: Keycode) -> Option<Key> {
    Some(match key {
        Keycode::Tab => Key::Tab,
        Keycode::Left => Key::LeftArrow,
        Keycode::Right => Key::RightArrow,
        Keycode::Up => Key::UpArrow,
        Keycode::Down => Key::DownArrow,
        Keycode::Home => Key::Home,
        Keycode::End => Key::End,
        Keycode::Delete => Key::Delete,
        Keycode::Backspace => Key::Backspace,
        Keycode::Return => Key::Enter,
        Keycode::KpEnter => Key::KeypadEnter,
        Keycode::Escape => Key::Escape,
        Keycode::A => Key::A,
        Keycode::C => Key::C,
        Keycode::V => Key::V,
        Keycode::X => Key::X,
        Keycode::Y => Key::Y,
        Keycode::Z => Key::Z,
        _ => return None,
    })
}

fn shortcut(key: Keycode, mods: Modifiers) -> Option<Action> {
    if !mods.ctrl {
        return None;
    }
    match key {
        Keycode::Z if mods.shift => Some(Action::Redo),
        Keycode::Z => Some(Action::Undo),
        Keycode::Y => Some(Action::Redo),
        _ => None,
    }
}

fn modifiers(state: Mod) -> Modifiers {
    Modifiers {
        shift: state.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD),
        ctrl: state.intersects(Mod::LCTRLMOD | Mod::RCTRLMOD),
        alt: state.intersects(Mod::LALTMOD | Mod::RALTMOD),
    }
}

/// Numpad views as in Blender, and F keys for keyboards without a numpad. Returns true if the key changed the camera.
fn apply_view_key(camera: &mut Camera, key: Keycode, mods: Modifiers) -> bool {
    match key {
        Keycode::KP_1 | Keycode::F1 if mods.ctrl => camera.set_view(180.0, 0.0),
        Keycode::KP_1 | Keycode::F1 => camera.set_view(0.0, 0.0),
        Keycode::KP_3 | Keycode::F3 if mods.ctrl => camera.set_view(-90.0, 0.0),
        Keycode::KP_3 | Keycode::F3 => camera.set_view(90.0, 0.0),
        Keycode::KP_7 | Keycode::F7 if mods.ctrl => camera.set_view(0.0, -90.0),
        Keycode::KP_7 | Keycode::F7 => camera.set_view(0.0, 90.0),
        Keycode::KP_5 | Keycode::F5 => camera.orthographic = !camera.orthographic,
        Keycode::KP_4 => camera.orbit_degrees(-VIEW_STEP_DEGREES, 0.0),
        Keycode::KP_6 => camera.orbit_degrees(VIEW_STEP_DEGREES, 0.0),
        Keycode::KP_8 => camera.orbit_degrees(0.0, VIEW_STEP_DEGREES),
        Keycode::KP_2 => camera.orbit_degrees(0.0, -VIEW_STEP_DEGREES),
        Keycode::KP_PLUS => camera.zoom_steps(1.0),
        Keycode::KP_MINUS => camera.zoom_steps(-1.0),
        Keycode::Home => *camera = Camera::default(),
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Modifiers = Modifiers {
        shift: false,
        ctrl: false,
        alt: false,
    };
    const CTRL: Modifiers = Modifiers {
        shift: false,
        ctrl: true,
        alt: false,
    };

    #[test]
    fn numpad_views_match_blender() {
        let mut camera = Camera::default();
        assert!(apply_view_key(&mut camera, Keycode::KP_1, NONE));
        assert_eq!((camera.yaw, camera.pitch), (0.0, 0.0));
        assert!(apply_view_key(&mut camera, Keycode::KP_3, NONE));
        assert_eq!(camera.yaw, 90f32.to_radians());
        assert!(apply_view_key(&mut camera, Keycode::KP_7, NONE));
        assert_eq!(camera.pitch, 90f32.to_radians());
        assert!(apply_view_key(&mut camera, Keycode::KP_1, CTRL));
        assert_eq!(camera.yaw, 180f32.to_radians());
        assert!(apply_view_key(&mut camera, Keycode::KP_3, CTRL));
        assert_eq!(camera.yaw, (-90f32).to_radians());
        assert!(apply_view_key(&mut camera, Keycode::KP_7, CTRL));
        assert_eq!(camera.pitch, (-90f32).to_radians());
    }

    #[test]
    fn f_keys_do_the_same_as_the_numpad() {
        for (numpad, f_key, mods) in [
            (Keycode::KP_1, Keycode::F1, NONE),
            (Keycode::KP_3, Keycode::F3, NONE),
            (Keycode::KP_7, Keycode::F7, NONE),
            (Keycode::KP_5, Keycode::F5, NONE),
            (Keycode::KP_1, Keycode::F1, CTRL),
            (Keycode::KP_3, Keycode::F3, CTRL),
            (Keycode::KP_7, Keycode::F7, CTRL),
        ] {
            let (mut a, mut b) = (Camera::default(), Camera::default());
            assert!(apply_view_key(&mut a, numpad, mods));
            assert!(apply_view_key(&mut b, f_key, mods));
            assert_eq!(a, b);
            assert_ne!(a, Camera::default());
        }
    }

    #[test]
    fn numpad_5_toggles_orthographic() {
        let mut camera = Camera::default();
        assert!(apply_view_key(&mut camera, Keycode::KP_5, NONE));
        assert!(camera.orthographic);
        assert!(apply_view_key(&mut camera, Keycode::KP_5, NONE));
        assert!(!camera.orthographic);
    }

    #[test]
    fn stepping_keys_turn_the_view_and_home_resets_it() {
        let mut camera = Camera::default();
        let start = camera;
        apply_view_key(&mut camera, Keycode::KP_6, NONE);
        assert!(camera.yaw > start.yaw);
        apply_view_key(&mut camera, Keycode::KP_4, NONE);
        apply_view_key(&mut camera, Keycode::KP_8, NONE);
        assert!(camera.pitch > start.pitch);
        apply_view_key(&mut camera, Keycode::KP_2, NONE);
        apply_view_key(&mut camera, Keycode::KP_PLUS, NONE);
        assert!(camera.distance < start.distance);
        apply_view_key(&mut camera, Keycode::KP_MINUS, NONE);
        apply_view_key(&mut camera, Keycode::Home, NONE);
        assert_eq!(camera, start);
    }

    #[test]
    fn undo_and_redo_shortcuts() {
        let ctrl_shift = Modifiers {
            shift: true,
            ..CTRL
        };
        assert_eq!(shortcut(Keycode::Z, CTRL), Some(Action::Undo));
        assert_eq!(shortcut(Keycode::Z, ctrl_shift), Some(Action::Redo));
        assert_eq!(shortcut(Keycode::Y, CTRL), Some(Action::Redo));
        assert_eq!(shortcut(Keycode::Z, NONE), None);
        assert_eq!(shortcut(Keycode::A, CTRL), None);
    }

    #[test]
    fn panel_area_includes_its_edges_only() {
        assert!(in_panel(PANEL_MARGIN, PANEL_MARGIN, 100.0));
        assert!(in_panel(
            PANEL_MARGIN + PANEL_WIDTH,
            PANEL_MARGIN + 100.0,
            100.0
        ));
        assert!(!in_panel(PANEL_MARGIN - 1.0, 50.0, 100.0));
        assert!(!in_panel(PANEL_MARGIN + PANEL_WIDTH + 1.0, 50.0, 100.0));
        assert!(!in_panel(50.0, PANEL_MARGIN + 101.0, 100.0));
    }

    #[test]
    fn other_keys_do_not_ask_for_a_redraw() {
        let mut camera = Camera::default();
        let start = camera;
        assert!(!apply_view_key(&mut camera, Keycode::A, NONE));
        assert_eq!(camera, start);
    }

    #[test]
    fn modifier_keys_on_either_side_count() {
        assert!(modifiers(Mod::LSHIFTMOD).shift && modifiers(Mod::RSHIFTMOD).shift);
        assert!(modifiers(Mod::LCTRLMOD).ctrl && modifiers(Mod::RCTRLMOD).ctrl);
        assert!(modifiers(Mod::LALTMOD).alt && modifiers(Mod::RALTMOD).alt);
        assert_eq!(modifiers(Mod::NUMMOD), NONE);
    }
}
