//! The desktop app. Stage 2: a window with a 3D grid and a Blender-style camera.
//!
//! Spec section 4 requires near 0% CPU when idle, so the main loop sleeps until
//! the system sends an event, and draws only if that event changed something.

// Release builds open no console window. Errors appear in a message box.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod camera;
mod controls;
mod grid;
mod renderer;

use std::io::Write;
use std::path::PathBuf;

use camera::Camera;
use controls::{Button, Drag, Modifiers, drag_for};
use renderer::Renderer;
use sdl2::event::{Event, WindowEvent};
use sdl2::keyboard::{Keycode, Mod};
use sdl2::messagebox::{MessageBoxFlag, show_simple_message_box};
use sdl2::mouse::{MouseButton, MouseWheelDirection};
use sdl2::video::{GLProfile, SwapInterval};

const APP_NAME: &str = "Low-End Modeler";
const VIEW_STEP_DEGREES: f32 = 15.0;

fn main() {
    std::panic::set_hook(Box::new(|info| {
        show_error(&format!("Internal error:\n{info}"))
    }));
    if let Err(message) = run() {
        show_error(&message);
        std::process::exit(1);
    }
}

fn show_error(message: &str) {
    Log::open().line(&format!("ERROR: {message}"));
    eprintln!("{message}");
    let _ = show_simple_message_box(MessageBoxFlag::ERROR, APP_NAME, message, None);
}

/// A small text file beside the .exe, so the owner can send details from their
/// laptop. It is only written at startup, on exit and on errors, to spare the disk.
struct Log {
    file: Option<std::fs::File>,
}

impl Log {
    fn path() -> Option<PathBuf> {
        Some(
            std::env::current_exe()
                .ok()?
                .with_file_name("low-end-modeler.log"),
        )
    }

    fn open() -> Self {
        let file = Self::path().and_then(|path| {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .ok()
        });
        Self { file }
    }

    fn line(&mut self, text: &str) {
        if let Some(file) = &mut self.file {
            let _ = writeln!(file, "{text}");
        }
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

fn run() -> Result<(), String> {
    let screenshot_path = {
        let mut args = std::env::args().skip(1);
        match (args.next().as_deref(), args.next()) {
            (Some("--screenshot"), Some(path)) => Some(path),
            _ => None,
        }
    };

    let sdl = sdl2::init()?;
    let video = sdl.video()?;
    let gl_attr = video.gl_attr();
    gl_attr.set_context_profile(GLProfile::Compatibility);
    gl_attr.set_context_version(2, 1);
    gl_attr.set_double_buffer(true);
    gl_attr.set_depth_size(24);
    gl_attr.set_multisample_samples(0);

    let mut window = video
        .window(APP_NAME, 1024, 640)
        .opengl()
        .resizable()
        .position_centered()
        .build()
        .map_err(|error| format!("Could not open a window: {error}"))?;
    let context = window
        .gl_create_context()
        .map_err(|error| format!("Could not start OpenGL 2.1: {error}"))?;
    window.gl_make_current(&context)?;
    // Limits drawing to the screen's refresh rate during drags. Not all drivers
    // allow this, and the app still works without it.
    let _ = video.gl_set_swap_interval(SwapInterval::VSync);

    // SAFETY: the context created above is current, and stays alive until run ends.
    let gl = unsafe {
        glow::Context::from_loader_function(|name| video.gl_get_proc_address(name) as *const _)
    };
    let (renderer, info) = Renderer::new(gl)?;

    let mut log = Log::open();
    log.line(&format!(
        "--- {APP_NAME} {} started",
        env!("CARGO_PKG_VERSION")
    ));
    log.line(&format!("OpenGL version: {}", info.version));
    log.line(&format!("GLSL version: {}", info.shading_language));
    log.line(&format!("Graphics chip: {}", info.renderer));
    log.line(&format!("Vendor: {}", info.vendor));
    window
        .set_title(&format!(
            "{APP_NAME} - OpenGL {} - {}",
            info.version, info.renderer
        ))
        .map_err(|error| error.to_string())?;

    let mut camera = Camera::default();
    let mut drag: Option<Drag> = None;
    let mut needs_redraw = true;
    let mut frames: u64 = 0;
    let mut reported_gl_error = false;
    let mouse = sdl.mouse();
    let keyboard = sdl.keyboard();
    let mut events = sdl.event_pump()?;

    'main: loop {
        if needs_redraw {
            let (width, height) = window.drawable_size();
            let (width, height) = (width as i32, height as i32);
            let aspect = width as f32 / height.max(1) as f32;
            renderer.render(
                width,
                height,
                &camera.view_projection(aspect),
                camera.grid_plane(),
            );
            if let Some(path) = &screenshot_path {
                save_ppm(path, width, height, &renderer.read_pixels(width, height))?;
                log.line(&format!("Screenshot saved to {path}"));
                break 'main;
            }
            window.gl_swap_window();
            frames += 1;
            needs_redraw = false;
            if !reported_gl_error && let Some(code) = renderer.error_code() {
                log.line(&format!("OpenGL error code: {code:#x}"));
                reported_gl_error = true;
            }
        }

        // Sleeps here until something happens. Then everything else that is
        // already waiting is handled too, so a fast drag draws only one frame.
        let first = events.wait_event();
        let mut next = Some(first);
        while let Some(event) = next {
            match event {
                Event::Quit { .. } => break 'main,
                Event::Window {
                    win_event:
                        WindowEvent::Exposed
                        | WindowEvent::SizeChanged(..)
                        | WindowEvent::Restored
                        | WindowEvent::Maximized,
                    ..
                } => needs_redraw = true,
                Event::MouseButtonDown { mouse_btn, .. } if drag.is_none() => {
                    let button = match mouse_btn {
                        MouseButton::Left => Button::Left,
                        MouseButton::Middle => Button::Middle,
                        MouseButton::Right => Button::Right,
                        _ => Button::Other,
                    };
                    drag = drag_for(button, modifiers(keyboard.mod_state()));
                    if drag.is_some() {
                        // Keeps the drag going if the mouse leaves the window.
                        mouse.capture(true);
                    }
                }
                Event::MouseButtonUp { .. } if drag.is_some() => {
                    drag = None;
                    mouse.capture(false);
                }
                Event::MouseMotion { xrel, yrel, .. } => {
                    let (dx, dy) = (xrel as f32, yrel as f32);
                    match drag {
                        Some(Drag::Orbit) => camera.orbit_pixels(dx, dy),
                        Some(Drag::Pan) => {
                            let height = window.drawable_size().1 as f32;
                            camera.pan_pixels(dx, dy, height);
                        }
                        Some(Drag::Zoom) => camera.zoom_pixels(dy),
                        None => {}
                    }
                    if drag.is_some() && (xrel != 0 || yrel != 0) {
                        needs_redraw = true;
                    }
                }
                Event::MouseWheel { y, direction, .. } if y != 0 => {
                    let steps = if direction == MouseWheelDirection::Flipped {
                        -y
                    } else {
                        y
                    };
                    camera.zoom_steps(steps as f32);
                    needs_redraw = true;
                }
                Event::KeyDown {
                    keycode: Some(key),
                    keymod,
                    ..
                } => {
                    needs_redraw |= apply_view_key(&mut camera, key, modifiers(keymod));
                }
                _ => {}
            }
            next = events.poll_event();
        }
    }

    log.line(&format!("Frames drawn: {frames}"));
    Ok(())
}

/// Writes a plain image file, for automated checks of what was drawn.
fn save_ppm(path: &str, width: i32, height: i32, bottom_up_rgb: &[u8]) -> Result<(), String> {
    let mut bytes = format!("P6\n{width} {height}\n255\n").into_bytes();
    bytes.reserve(bottom_up_rgb.len());
    for row in bottom_up_rgb.chunks_exact(width.max(1) as usize * 3).rev() {
        bytes.extend_from_slice(row);
    }
    std::fs::write(path, bytes).map_err(|error| format!("Could not save {path}: {error}"))
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
