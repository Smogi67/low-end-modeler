//! The desktop app: opens the window and runs the main loop. See `app.rs` for the rest.

// Release builds open no console window. Errors appear in a message box.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod camera;
mod controls;
mod editor;
mod export;
mod grid;
mod imgui_gl;
mod picking;
mod renderer;
mod scene;
mod transform;
mod ui;

use std::cell::RefCell;
use std::io::Write;
use std::path::PathBuf;

use app::App;
use renderer::Renderer;
use sdl2::event::{Event, WindowEvent};
use sdl2::messagebox::{MessageBoxFlag, show_simple_message_box};
use sdl2::video::{GLProfile, SwapInterval};

const APP_NAME: &str = "Low-End Modeler";

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
        .window(APP_NAME, 1200, 780)
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

    let app = RefCell::new(App::new(renderer)?);

    // Windows pauses this thread's main loop while a window edge is being dragged,
    // and only keeps delivering events to the watch below, so window events draw
    // from there. The main loop never holds `app` while waiting for events.
    let _resize_watch = sdl.event()?.add_event_watch(|event| {
        if let Event::Window {
            win_event:
                WindowEvent::Exposed
                | WindowEvent::SizeChanged(..)
                | WindowEvent::Restored
                | WindowEvent::Maximized,
            ..
        } = event
            && let Ok(mut app) = app.try_borrow_mut()
        {
            app.draw(&window);
            window.gl_swap_window();
        }
    });

    let mouse = sdl.mouse();
    let keyboard = sdl.keyboard();
    let mut events = sdl.event_pump()?;
    let mut reported_gl_error = false;

    'main: loop {
        let wants_frame = app.borrow().wants_frame();
        if wants_frame {
            let mut app = app.borrow_mut();
            app.draw(&window);
            if let Some(path) = &screenshot_path {
                // The panel only takes its final shape after a first frame.
                app.draw(&window);
                let (width, height, pixels) = app.read_pixels(&window);
                save_ppm(path, width, height, &pixels)?;
                log.line(&format!("Screenshot saved to {path}"));
                break 'main;
            }
            window.gl_swap_window();
            if !reported_gl_error && let Some(code) = app.gl_error() {
                log.line(&format!("OpenGL error code: {code:#x}"));
                reported_gl_error = true;
            }
        }

        // Sleeps here until something happens, unless more frames are wanted. Then
        // everything else already waiting is handled too, so a fast drag draws once.
        let mut next = if wants_frame && app.borrow().wants_frame() {
            events.poll_event()
        } else {
            Some(events.wait_event())
        };
        while let Some(event) = next {
            if let Event::Quit { .. } = event {
                break 'main;
            }
            app.borrow_mut()
                .handle_event(&event, &mouse, keyboard.mod_state());
            next = events.poll_event();
        }
    }

    log.line(&format!("Frames drawn: {}", app.borrow().frames_drawn()));
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
