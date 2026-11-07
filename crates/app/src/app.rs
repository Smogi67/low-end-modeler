//! Ties the document, camera, panel and drawing together.
//!
//! Spec section 4 requires near 0% CPU when idle. The main loop sleeps until an event
//! arrives, and only asks for frames if the event changed something. Dear ImGui takes
//! a couple of frames to show the result of a click, so such events ask for a few.

use std::path::PathBuf;
use std::time::Instant;

use imgui::{Key, MouseButton as ImguiButton};
use modeler_mesh::{Element, Mesh, MeshError, SelectMode, SelectOp};
use sdl2::event::Event;
use sdl2::keyboard::{Keycode, Mod};
use sdl2::mouse::{MouseButton, MouseUtil, MouseWheelDirection};
use sdl2::video::Window;

use crate::camera::Camera;
use crate::controls::{Button, Drag, Modifiers, drag_for};
use crate::editor::Editor;
use crate::export::{warnings, write_obj};
use crate::imgui_gl::ImguiGl;
use crate::picking::{DepthImage, Region, ViewInfo, pick_point, pick_region};
use crate::renderer::{Renderer, View};
use crate::scene::{MovedParts, Scene};
use crate::theme::{self, Fonts};
use crate::transform::{Input, Kind, Transform};
use crate::ui::{
    Action, DragTool, ModeChoice, PanelInfo, PanelState, ViewState, build as build_ui, draw_region,
};

const VIEW_STEP_DEGREES: f32 = 15.0;
/// Frames to draw after an interaction, so that Dear ImGui can settle.
const UI_SETTLE_FRAMES: u32 = 3;
/// A left press that moves less than this is a click rather than a drag.
const CLICK_PIXELS: f32 = 8.0;
const CIRCLE_BRUSH_RADIUS: f32 = 30.0;
/// How much nearer than the surface a point may be pretended to be, as a share of the
/// camera distance, so a point lying on a visible face is not hidden by that face.
const DEPTH_TOLERANCE: f32 = 0.004;
/// "Merge close vertices" joins vertices closer together than this, in model units.
const MERGE_DISTANCE: f32 = 0.001;

/// What must happen to the selection before a drag starts.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Prepare {
    Nothing,
    Duplicate,
    Extrude,
    Inset,
}

/// What the mouse is in the middle of doing.
enum Mouse {
    Idle,
    Camera(Drag),
    Selecting {
        region: Region,
        start: [f32; 2],
        moved: bool,
        click_only: bool,
        op: SelectOp,
    },
    Transforming {
        transform: Box<Transform>,
        parts: MovedParts,
    },
}

/// Which scene data must go to the graphics card before the next draw.
#[derive(PartialEq)]
enum Stale {
    Nothing,
    Moved,
    Everything,
}

pub struct App {
    renderer: Renderer,
    imgui: imgui::Context,
    imgui_gl: ImguiGl,
    editor: Editor,
    state: PanelState,
    camera: Camera,
    mouse_state: Mouse,
    scene: Scene,
    stale: Stale,
    frames_wanted: u32,
    frames_drawn: u64,
    last_frame: Instant,
    ui_rects: Vec<[f32; 4]>,
    fonts: Fonts,
    pointer: [f32; 2],
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
        let fonts = theme::install(&mut imgui);
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
            editor: Editor::new(),
            state: PanelState::default(),
            camera: Camera::default(),
            mouse_state: Mouse::Idle,
            scene: Scene::default(),
            stale: Stale::Everything,
            frames_wanted: UI_SETTLE_FRAMES,
            frames_drawn: 0,
            last_frame: Instant::now(),
            ui_rects: Vec::new(),
            fonts,
            pointer: [0.0; 2],
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

        let target = self.editor.refresh_target(&mut self.state);
        let document = self.editor.document();
        let mesh = document.mesh();
        let transform_help = match &self.mouse_state {
            Mouse::Transforming { transform, .. } => Some((transform.kind, transform.axis)),
            _ => None,
        };
        let info = PanelInfo {
            triangles: mesh.triangle_count(),
            vertices: mesh.vertex_count(),
            faces: mesh.face_count(),
            warnings: &self.warnings,
            target,
            mode: self.editor.mode_choice(),
            selected_vertices: mesh.vertices().filter(|(_, v)| v.is_selected()).count(),
            transform_help,
            undo_label: document.undo_label(),
            redo_label: document.redo_label(),
            export_folder: &self.export_folder_text,
            open_edges: mesh.open_edge_count(),
            view: {
                let (right, up) = self.camera.right_and_up();
                ViewState {
                    right,
                    up,
                    toward: self.camera.toward_camera(),
                    orthographic: self.camera.orthographic,
                }
            },
        };
        let ui = self.imgui.new_frame();
        let output = build_ui(ui, &mut self.state, &info, &self.fonts);
        let action = output.action;
        if let Mouse::Selecting {
            region,
            moved: true,
            ..
        } = &self.mouse_state
        {
            draw_region(ui, region);
        }
        self.ui_rects = output.rects;
        let draw_data = self.imgui.render();

        // Results appear in the frames that follow.
        match self.editor.adjust(&self.state) {
            Ok(true) => {
                self.stale = Stale::Everything;
                self.frames_wanted = self.frames_wanted.max(UI_SETTLE_FRAMES);
            }
            Ok(false) => {}
            Err(error) => self.state.message = format!("Could not change the shape: {error}"),
        }

        if self.stale == Stale::Everything {
            let mesh = self.editor.document().mesh();
            self.scene.rebuild(mesh, self.editor.drawing_mode());
            self.renderer.upload_scene(&self.scene);
            self.warnings = warnings(mesh.triangle_count(), mesh.open_edge_count());
        } else if self.stale == Stale::Moved
            && let Mouse::Transforming { parts, .. } = &self.mouse_state
        {
            let changed = self
                .scene
                .update_moved(self.editor.document().mesh(), parts);
            self.renderer.update_scene(&self.scene, &changed);
        }
        self.stale = Stale::Nothing;

        let aspect = width as f32 / height.max(1) as f32;
        let view = View {
            view_projection: self.camera.view_projection(aspect),
            light: self.camera.key_light(),
            plane: self.camera.grid_plane(),
            fade_center: self.camera.target,
            fade_radius: self.camera.grid_fade_radius(),
            point_depth_bias: self.camera.point_depth_bias(),
        };
        self.renderer.render(width, height, &view);
        self.imgui_gl.render(self.renderer.gl(), draw_data, height);

        self.frames_drawn += 1;
        self.frames_wanted = self.frames_wanted.saturating_sub(1);

        if let Some(action) = action {
            self.apply_action(action);
            self.request_frames();
        }
    }

    pub fn read_pixels(&self, window: &Window) -> (i32, i32, Vec<u8>) {
        let (width, height) = window.drawable_size();
        let (width, height) = (width as i32, height as i32);
        (width, height, self.renderer.read_pixels(width, height))
    }

    fn viewport(&self) -> (f32, f32) {
        let size = self.imgui.io().display_size;
        (size[0].max(1.0), size[1].max(1.0))
    }

    fn select_edit(&mut self, label: &'static str, edit: impl FnOnce(&mut Mesh, SelectMode)) {
        let result = self.editor.run(label, |mesh, mode| {
            edit(mesh, mode);
            Ok(())
        });
        self.finish_edit(result);
    }

    fn finish_edit(&mut self, result: Result<(), MeshError>) {
        match result {
            Ok(()) => self.stale = Stale::Everything,
            Err(error) => self.state.message = format!("That did not work: {error}"),
        }
    }

    fn apply_action(&mut self, action: Action) {
        match action {
            Action::Add(shape) => match self.editor.add(shape, &self.state) {
                Ok(()) => self.stale = Stale::Everything,
                Err(error) => self.state.message = format!("Could not add the shape: {error}"),
            },
            Action::Undo => {
                if self.editor.undo() {
                    self.stale = Stale::Everything;
                }
            }
            Action::Redo => {
                if self.editor.redo() {
                    self.stale = Stale::Everything;
                }
            }
            Action::Export => {
                self.state.message = match write_obj(
                    self.editor.document().mesh(),
                    &self.export_folder,
                    &self.state.file_name,
                    self.state.scale,
                ) {
                    Ok(path) => {
                        let name = path.file_name().unwrap_or_default().to_string_lossy();
                        format!("Saved {name}")
                    }
                    Err(error) => error,
                };
            }
            Action::SetMode(mode) => {
                if self.editor.set_mode(mode) {
                    self.stale = Stale::Everything;
                }
            }
            Action::SelectAll => self.select_edit("Select all", |mesh, _| mesh.select_all()),
            Action::SelectNone => self.select_edit("Select none", |mesh, _| mesh.deselect_all()),
            Action::SelectInvert => {
                self.select_edit("Invert selection", |mesh, mode| mesh.invert_selection(mode));
            }
            Action::SelectLinked => {
                self.select_edit("Select linked", |mesh, mode| mesh.select_linked(mode));
            }
            Action::SelectMore => {
                self.select_edit("Select more", |mesh, mode| mesh.select_more(mode));
            }
            Action::SelectLess => {
                self.select_edit("Select less", |mesh, mode| mesh.select_less(mode));
            }
            Action::View(preset) => {
                let (yaw, pitch) = preset.angles();
                self.camera.set_view(yaw, pitch);
            }
            Action::ToggleOrtho => self.camera.orthographic = !self.camera.orthographic,
            Action::ResetView => self.camera = Camera::default(),
            Action::Transform(kind) => self.start_transform(kind, Prepare::Nothing),
            Action::Duplicate => self.start_transform(Kind::Move, Prepare::Duplicate),
            Action::Extrude => self.start_transform(Kind::Move, Prepare::Extrude),
            Action::Inset => self.start_transform(Kind::Inset, Prepare::Inset),
            Action::Delete => {
                let result = self
                    .editor
                    .run("Delete", |mesh, mode| mesh.delete_selected(mode));
                self.finish_edit(result);
            }
            Action::Merge => {
                let result = self
                    .editor
                    .run("Merge", |mesh, mode| mesh.merge_selected_at_center(mode));
                self.finish_edit(result);
            }
            Action::MergeByDistance => {
                let mut removed = 0;
                let result = self.editor.run("Merge close vertices", |mesh, mode| {
                    removed = mesh.merge_selected_by_distance(MERGE_DISTANCE, mode)?;
                    Ok(())
                });
                self.state.message = format!("Merged away {removed} vertices.");
                self.finish_edit(result);
            }
            Action::Fill => {
                let result = self
                    .editor
                    .run("Fill", |mesh, mode| mesh.fill_selected(mode));
                self.finish_edit(result);
            }
            Action::Subdivide => {
                let result = self
                    .editor
                    .run("Subdivide", |mesh, mode| mesh.subdivide_selected(mode));
                self.finish_edit(result);
            }
            Action::Mirror(axis) => {
                let result = self
                    .editor
                    .run("Mirror", |mesh, _| match mesh.selection_center() {
                        Some(center) => mesh.mirror_selected(axis, center),
                        None => Ok(()),
                    });
                self.finish_edit(result);
            }
            #[cfg(windows)]
            Action::OpenExportFolder => {
                let _ = std::process::Command::new("explorer")
                    .arg(&self.export_folder)
                    .spawn();
            }
        }
    }

    /// Starts dragging the selection. Anything prepared first shares its undo step, so
    /// cancelling the drag removes it again.
    fn start_transform(&mut self, kind: Kind, prepare: Prepare) {
        if !matches!(self.mouse_state, Mouse::Idle) {
            return;
        }
        if self.editor.document().mesh().selection_center().is_none() {
            self.state.message = "Select something first.".to_string();
            return;
        }
        let label = match prepare {
            Prepare::Nothing => kind.name(),
            Prepare::Duplicate => "Duplicate",
            Prepare::Extrude => "Extrude",
            Prepare::Inset => "Inset",
        };
        if let Err(error) = self.editor.begin(label) {
            self.state.message = format!("That did not work: {error}");
            return;
        }
        let mode = self.editor.mode();
        let mut direction = None;
        let mut inset_targets = Vec::new();
        let prepared = match self.editor.active_mesh() {
            None => Ok(()),
            Some(mesh) => match prepare {
                Prepare::Nothing => Ok(()),
                Prepare::Duplicate => mesh.duplicate_selected(mode),
                Prepare::Extrude => {
                    direction = mesh.selected_faces_normal();
                    mesh.extrude_selected_faces(mode)
                }
                Prepare::Inset => mesh
                    .inset_selected_faces(mode)
                    .map(|moving| inset_targets = moving),
            },
        };
        if let Err(error) = prepared {
            let _ = self.editor.cancel();
            self.state.message = format!("That did not work: {error}");
            return;
        }
        if prepare != Prepare::Nothing {
            // New elements exist, so the card needs all of it once.
            let mesh = self.editor.document().mesh();
            self.scene.rebuild(mesh, self.editor.drawing_mode());
            self.renderer.upload_scene(&self.scene);
        }
        let mesh = self.editor.document().mesh();
        let (moving, centers): (Vec<_>, Vec<_>) = if prepare == Prepare::Inset {
            inset_targets.iter().copied().unzip()
        } else {
            (mesh.selected_vertices(), Vec::new())
        };
        let originals = moving
            .iter()
            .filter_map(|&id| Some((id, mesh.vertex(id)?.position())))
            .collect();
        let pivot = mesh.selection_center().unwrap_or([0.0; 3]);
        let parts = Scene::moved_parts(mesh, &moving);
        let mut transform = Transform::new(kind, self.pointer, originals, pivot);
        transform.direction = direction;
        transform.centers = centers;
        self.mouse_state = Mouse::Transforming {
            transform: Box::new(transform),
            parts,
        };
        self.request_frames();
    }

    fn update_transform(&mut self, snap: bool) {
        let (width, height) = self.viewport();
        let Mouse::Transforming { transform, .. } = &self.mouse_state else {
            return;
        };
        let input = Input {
            mouse: self.pointer,
            snap,
            camera: self.camera,
            width,
            height,
        };
        let moved: Vec<_> = (0..transform.originals.len())
            .map(|i| (transform.originals[i].0, transform.position_for(i, &input)))
            .collect();
        if let Some(mesh) = self.editor.active_mesh() {
            for (id, position) in moved {
                // A refused position just leaves that vertex in place for this frame.
                let _ = mesh.set_vertex_position(id, position);
            }
        }
        if self.stale != Stale::Everything {
            self.stale = Stale::Moved;
        }
        self.frames_wanted = self.frames_wanted.max(1);
    }

    fn finish_transform(&mut self, confirm: bool) {
        if !matches!(self.mouse_state, Mouse::Transforming { .. }) {
            return;
        }
        self.mouse_state = Mouse::Idle;
        let result = if confirm {
            self.editor.commit().map(|_| ())
        } else {
            self.editor.cancel()
        };
        self.finish_edit(result);
        self.request_frames();
    }

    /// Runs `pick` with a check for whether a point is hidden by the scene.
    fn with_visibility<T>(
        &self,
        pick: impl FnOnce(&Mesh, SelectMode, &ViewInfo, &dyn Fn([f32; 3]) -> bool) -> T,
    ) -> T {
        let (width, height) = self.viewport();
        let matrix = self.camera.view_projection(width / height);
        let view = ViewInfo {
            matrix,
            width,
            height,
        };
        let depth = self
            .renderer
            .read_depth(width as i32, height as i32, &matrix);
        let image = DepthImage {
            width: width as i32,
            height: height as i32,
            depth,
        };
        let toward = self.camera.toward_camera();
        let tolerance = self.camera.distance * DEPTH_TOLERANCE;
        let visible = |p: [f32; 3]| {
            let lifted = std::array::from_fn(|i| p[i] + toward[i] * tolerance);
            match (view.project(p), view.project(lifted)) {
                (Some(screen), Some(near)) => image.shows(screen[0], screen[1], near[2]),
                _ => false,
            }
        };
        pick(
            self.editor.document().mesh(),
            self.editor.mode(),
            &view,
            &visible,
        )
    }

    fn finish_selecting(&mut self) {
        let Mouse::Selecting {
            region,
            start,
            moved,
            op,
            ..
        } = std::mem::replace(&mut self.mouse_state, Mouse::Idle)
        else {
            return;
        };
        let mut elements: Vec<Element> = if moved {
            self.with_visibility(|mesh, mode, view, visible| {
                pick_region(mesh, mode, view, &region, visible)
            })
        } else {
            let hit = self.with_visibility(|mesh, mode, view, visible| {
                pick_point(mesh, mode, view, start[0], start[1], visible)
            });
            hit.into_iter().collect()
        };
        if self.editor.whole_parts() {
            let mesh = self.editor.document().mesh();
            elements = mesh
                .linked_faces(&elements)
                .into_iter()
                .map(Element::Face)
                .collect();
        }
        // A plain click on nothing clears the selection, while Shift and Ctrl keep it.
        let result = self.editor.run("Select", |mesh, mode| {
            mesh.select_elements(mode, &elements, op)
        });
        self.finish_edit(result);
        self.request_frames();
    }

    fn start_selecting(&mut self, modifiers: Modifiers) {
        let start = self.pointer;
        let region = match self.state.tool {
            DragTool::ClickOnly | DragTool::Box => Region::Box { a: start, b: start },
            DragTool::Circle => Region::Brush {
                path: vec![start],
                radius: CIRCLE_BRUSH_RADIUS,
            },
            DragTool::Lasso => Region::Lasso {
                points: vec![start],
            },
        };
        let op = if modifiers.shift {
            SelectOp::Add
        } else if modifiers.ctrl {
            SelectOp::Subtract
        } else {
            SelectOp::Replace
        };
        self.mouse_state = Mouse::Selecting {
            region,
            start,
            moved: false,
            click_only: self.state.tool == DragTool::ClickOnly,
            op,
        };
    }

    /// Handles one event. `mods` is the current state of Shift, Ctrl and Alt.
    pub fn handle_event(&mut self, event: &Event, mouse: &MouseUtil, mods: Mod) {
        if let Event::MouseMotion { x, y, .. } = *event {
            self.pointer = [x as f32, y as f32];
        }
        let snap = modifiers(mods).ctrl;

        // A transform takes all input until it ends, so nothing behind it can change.
        if matches!(self.mouse_state, Mouse::Transforming { .. }) {
            self.imgui_pointer_only(event);
            self.handle_transform_event(event, snap);
            return;
        }

        self.feed_imgui(event);
        let io = self.imgui.io();
        let (ui_mouse, ui_keyboard) = (io.want_capture_mouse, io.want_capture_keyboard);

        match *event {
            Event::MouseButtonDown { mouse_btn, .. } => {
                self.request_frames();
                if !matches!(self.mouse_state, Mouse::Idle) || ui_mouse || self.pointer_over_panel {
                    return;
                }
                let button = match mouse_btn {
                    MouseButton::Left => Button::Left,
                    MouseButton::Middle => Button::Middle,
                    MouseButton::Right => Button::Right,
                    _ => Button::Other,
                };
                let mods = modifiers(mods);
                if let Some(drag) = drag_for(button, mods) {
                    self.mouse_state = Mouse::Camera(drag);
                    // Keeps the drag going if the mouse leaves the window.
                    mouse.capture(true);
                } else if button == Button::Left {
                    self.start_selecting(mods);
                    mouse.capture(true);
                }
            }
            Event::MouseButtonUp { .. } => {
                self.request_frames();
                match self.mouse_state {
                    Mouse::Camera(_) => {
                        self.mouse_state = Mouse::Idle;
                        mouse.capture(false);
                    }
                    Mouse::Selecting { .. } => {
                        mouse.capture(false);
                        self.finish_selecting();
                    }
                    _ => {}
                }
            }
            Event::MouseMotion {
                x, y, xrel, yrel, ..
            } => {
                let (dx, dy) = (xrel as f32, yrel as f32);
                let pointer = self.pointer;
                let height = self.viewport().1;
                match &mut self.mouse_state {
                    Mouse::Camera(Drag::Orbit) => self.camera.orbit_pixels(dx, dy),
                    Mouse::Camera(Drag::Pan) => self.camera.pan_pixels(dx, dy, height),
                    Mouse::Camera(Drag::Zoom) => self.camera.zoom_pixels(dy),
                    Mouse::Selecting {
                        region,
                        start,
                        moved,
                        click_only,
                        ..
                    } => {
                        let far = (pointer[0] - start[0])
                            .abs()
                            .max((pointer[1] - start[1]).abs());
                        *moved |= far >= CLICK_PIXELS && !*click_only;
                        match region {
                            Region::Box { b, .. } => *b = pointer,
                            Region::Brush { path, .. } => path.push(pointer),
                            Region::Lasso { points } => points.push(pointer),
                        }
                    }
                    _ => {}
                }
                if !matches!(self.mouse_state, Mouse::Idle) {
                    self.frames_wanted = self.frames_wanted.max(1);
                }
                // Moving over empty viewport needs no drawing. The panel highlights
                // its buttons, so it does, as does leaving it or using a slider.
                let over = in_rects(&self.ui_rects, x as f32, y as f32);
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
                    if key == Keycode::Escape && matches!(self.mouse_state, Mouse::Selecting { .. })
                    {
                        self.mouse_state = Mouse::Idle;
                        mouse.capture(false);
                    } else if key == Keycode::L && !mods.ctrl {
                        self.select_linked_under_pointer();
                    } else if let Some(action) = shortcut(key, mods) {
                        self.apply_action(action);
                    } else {
                        apply_view_key(&mut self.camera, key, mods);
                    }
                }
            }
            Event::KeyUp { .. } | Event::TextInput { .. } => self.request_frames(),
            _ => {}
        }
    }

    fn select_linked_under_pointer(&mut self) {
        let [x, y] = self.pointer;
        let hit = self.with_visibility(|mesh, mode, view, visible| {
            pick_point(mesh, mode, view, x, y, visible)
        });
        let result = self.editor.run("Select linked", |mesh, mode| {
            if let Some(element) = hit {
                mesh.select_elements(mode, &[element], SelectOp::Add)?;
                mesh.select_linked(mode);
            }
            Ok(())
        });
        self.finish_edit(result);
    }

    fn handle_transform_event(&mut self, event: &Event, snap: bool) {
        match *event {
            Event::MouseMotion { .. } => self.update_transform(snap),
            Event::MouseButtonDown {
                mouse_btn: MouseButton::Left,
                ..
            } => self.finish_transform(true),
            Event::MouseButtonDown {
                mouse_btn: MouseButton::Right,
                ..
            } => self.finish_transform(false),
            Event::MouseWheel { y, direction, .. } if y != 0 => {
                let steps = if direction == MouseWheelDirection::Flipped {
                    -y
                } else {
                    y
                };
                self.camera.zoom_steps(steps as f32);
                self.update_transform(snap);
            }
            Event::KeyDown {
                keycode: Some(key), ..
            } => match key {
                Keycode::Escape => self.finish_transform(false),
                Keycode::Return | Keycode::KpEnter | Keycode::Space => {
                    self.finish_transform(true);
                }
                Keycode::X | Keycode::Y | Keycode::Z => {
                    let axis = match key {
                        Keycode::X => 0,
                        Keycode::Y => 1,
                        _ => 2,
                    };
                    if let Mouse::Transforming { transform, .. } = &mut self.mouse_state {
                        transform.toggle_axis(axis);
                    }
                    self.update_transform(snap);
                    self.request_frames();
                }
                Keycode::LCtrl | Keycode::RCtrl => self.update_transform(true),
                _ => {}
            },
            Event::KeyUp {
                keycode: Some(Keycode::LCtrl | Keycode::RCtrl),
                ..
            } => self.update_transform(false),
            _ => {}
        }
    }

    fn imgui_pointer_only(&mut self, event: &Event) {
        if let Event::MouseMotion { x, y, .. } = event {
            self.imgui
                .io_mut()
                .add_mouse_pos_event([*x as f32, *y as f32]);
        }
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

/// True if (x, y) is inside any of the rectangles, given as x, y, width and height.
fn in_rects(rects: &[[f32; 4]], x: f32, y: f32) -> bool {
    rects
        .iter()
        .any(|r| (r[0]..=r[0] + r[2]).contains(&x) && (r[1]..=r[1] + r[3]).contains(&y))
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
    if mods.alt {
        return None;
    }
    if mods.ctrl {
        return match key {
            Keycode::Z if mods.shift => Some(Action::Redo),
            Keycode::Z => Some(Action::Undo),
            Keycode::Y => Some(Action::Redo),
            Keycode::I => Some(Action::SelectInvert),
            Keycode::L => Some(Action::SelectLinked),
            Keycode::Plus | Keycode::KP_PLUS | Keycode::Equals => Some(Action::SelectMore),
            Keycode::Minus | Keycode::KP_MINUS => Some(Action::SelectLess),
            _ => None,
        };
    }
    if mods.shift {
        return (key == Keycode::D).then_some(Action::Duplicate);
    }
    match key {
        Keycode::Num1 => Some(Action::SetMode(ModeChoice::Vertex)),
        Keycode::Num2 => Some(Action::SetMode(ModeChoice::Edge)),
        Keycode::Num3 => Some(Action::SetMode(ModeChoice::Face)),
        Keycode::Num4 => Some(Action::SetMode(ModeChoice::Part)),
        Keycode::A => Some(Action::SelectAll),
        Keycode::G => Some(Action::Transform(Kind::Move)),
        Keycode::R => Some(Action::Transform(Kind::Rotate)),
        Keycode::S => Some(Action::Transform(Kind::Scale)),
        Keycode::E => Some(Action::Extrude),
        Keycode::I => Some(Action::Inset),
        Keycode::X | Keycode::Delete => Some(Action::Delete),
        Keycode::M => Some(Action::Merge),
        Keycode::F => Some(Action::Fill),
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
        assert_eq!(shortcut(Keycode::Q, CTRL), None);
    }

    #[test]
    fn selection_and_tool_shortcuts() {
        let shift = Modifiers {
            shift: true,
            ..NONE
        };
        let alt = Modifiers { alt: true, ..NONE };
        let pairs = [
            (Keycode::Num1, Action::SetMode(ModeChoice::Vertex)),
            (Keycode::Num2, Action::SetMode(ModeChoice::Edge)),
            (Keycode::Num3, Action::SetMode(ModeChoice::Face)),
            (Keycode::Num4, Action::SetMode(ModeChoice::Part)),
            (Keycode::A, Action::SelectAll),
            (Keycode::G, Action::Transform(Kind::Move)),
            (Keycode::R, Action::Transform(Kind::Rotate)),
            (Keycode::S, Action::Transform(Kind::Scale)),
            (Keycode::E, Action::Extrude),
            (Keycode::I, Action::Inset),
            (Keycode::X, Action::Delete),
            (Keycode::Delete, Action::Delete),
            (Keycode::M, Action::Merge),
            (Keycode::F, Action::Fill),
        ];
        for (key, action) in pairs {
            assert_eq!(shortcut(key, NONE), Some(action));
            assert_eq!(shortcut(key, alt), None);
        }
        assert_eq!(shortcut(Keycode::D, shift), Some(Action::Duplicate));
        assert_eq!(shortcut(Keycode::D, NONE), None);
        assert_eq!(shortcut(Keycode::I, CTRL), Some(Action::SelectInvert));
        assert_eq!(shortcut(Keycode::L, CTRL), Some(Action::SelectLinked));
        assert_eq!(shortcut(Keycode::Plus, CTRL), Some(Action::SelectMore));
        assert_eq!(shortcut(Keycode::Minus, CTRL), Some(Action::SelectLess));
    }

    #[test]
    fn interface_areas_include_their_edges_only() {
        let rects = [[10.0, 20.0, 100.0, 50.0], [500.0, 0.0, 10.0, 10.0]];
        assert!(in_rects(&rects, 10.0, 20.0));
        assert!(in_rects(&rects, 110.0, 70.0));
        assert!(in_rects(&rects, 505.0, 5.0));
        assert!(!in_rects(&rects, 9.0, 30.0));
        assert!(!in_rects(&rects, 50.0, 71.0));
        assert!(!in_rects(&[], 0.0, 0.0));
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
