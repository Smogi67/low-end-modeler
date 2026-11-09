//! Ties the document, camera, panel and drawing together.
//!
//! Spec section 4 requires near 0% CPU when idle. The main loop sleeps until an event
//! arrives, and only asks for frames if the event changed something. Dear ImGui takes
//! a couple of frames to show the result of a click, so such events ask for a few.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use imgui::{Key, MouseButton as ImguiButton};
use mixer_mesh::{
    BevelSettings, BooleanOp, EdgeId, Element, FaceId, FileKind, MAX_BEVEL_SEGMENTS, MAX_LOOP_CUTS,
    Mesh, MeshError, Modifier, ModifierId, ProjectSettings, SelectMode, SelectOp,
};
use sdl2::event::Event;
use sdl2::keyboard::{Keycode, Mod};
use sdl2::mouse::{MouseButton, MouseUtil, MouseWheelDirection};
use sdl2::video::Window;

use crate::camera::Camera;
use crate::controls::{Button, Drag, Modifiers, drag_for};
use crate::editor::Editor;
use crate::export::{
    ExportFormat, PROJECT_EXTENSION, open_file, save_project, warnings, write_export,
};
use crate::gizmo::{self, Handle};
use crate::gpu::Gpu;
use crate::knife::{KnifeCut, knife_cut, snapped_vertex};
use crate::picking::{DepthImage, Region, ViewInfo, pick_edge, pick_point, pick_region};
use crate::places;
use crate::renderer::View;
use crate::scene::{MovedParts, Scene};
use crate::settings::Settings;
use crate::settings_window::{SettingsAction, SettingsState, build_settings};
use crate::start::{StartAction, StartState, build_start};
use crate::theme::{self, Fonts};
use crate::transform::{Input, Kind, Transform};
use crate::ui::{
    Action, DragTool, ModeChoice, ModifierKind, PanelInfo, PanelState, PathPurpose, Shape,
    ViewState, build as build_ui, draw_gizmo, draw_guide, draw_knife, draw_lines, draw_region,
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
/// A bevel narrower than this, in model units, is dropped rather than kept.
const MIN_BEVEL_WIDTH: f32 = 1e-4;
/// How near the pointer an edge must be to choose it for a loop cut, in pixels.
const LOOP_CUT_REACH: f32 = 40.0;
/// How near the pointer an edge must be for double-click loop selection, in pixels.
const LOOP_SELECT_REACH: f32 = 14.0;

/// Something that would throw away unsaved changes, waiting for an answer.
#[derive(Debug, Clone, PartialEq)]
enum Pending {
    New,
    /// Back to the start window.
    Start,
    /// Open this file, or ask which one.
    Open(Option<PathBuf>),
    Quit,
}

/// The result of asking for a file path. Windows uses the first two, other systems
/// the last.
#[allow(dead_code)]
enum Picked {
    Path(PathBuf),
    Cancelled,
    /// The app's own path box is open; the answer comes as an action.
    Asking,
}

/// What must happen to the selection before a drag starts.
#[derive(Clone, PartialEq)]
enum Prepare {
    Nothing,
    Duplicate,
    Extrude,
    Inset,
    /// Cut this many loops across the ring through this edge.
    LoopCut(EdgeId, usize),
    /// Bevel with these settings, starting at no width.
    Bevel(Box<BevelSettings>),
}

/// The view and a depth image of it, for telling whether a point is hidden.
struct Sight {
    view: ViewInfo,
    image: DepthImage,
    toward: [f32; 3],
    tolerance: f32,
}

impl Sight {
    fn visible(&self, p: [f32; 3]) -> bool {
        let lifted = std::array::from_fn(|i| p[i] + self.toward[i] * self.tolerance);
        match (self.view.project(p), self.view.project(lifted)) {
            (Some(screen), Some(near)) => self.image.shows(screen[0], screen[1], near[2]),
            _ => false,
        }
    }
}

/// Choosing where to loop cut.
struct LoopCutChoice {
    cuts: usize,
    edge: Option<EdgeId>,
    /// Where the cut would go, on screen.
    lines: Vec<[[f32; 2]; 2]>,
    /// Read once at the start, since the view does not change meanwhile.
    sight: Sight,
}

/// Drawing a knife path: clicks so far, and where they cross the mesh.
struct KnifeChoice {
    clicks: Vec<[f32; 2]>,
    /// Cut hidden faces too, not just the ones that can be seen.
    through: bool,
    /// The cut the clicks make, and with the pointer as one more click.
    cut: KnifeCut,
    preview: KnifeCut,
    sight: Sight,
}

/// Dragging a bisect line.
struct BisectChoice {
    start: Option<[f32; 2]>,
    end: [f32; 2],
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
    LoopCut(Box<LoopCutChoice>),
    Knife(Box<KnifeChoice>),
    Bisect(Box<BisectChoice>),
}

/// Which scene data must go to the graphics card before the next draw.
#[derive(PartialEq)]
enum Stale {
    Nothing,
    Moved,
    Everything,
}

pub struct App {
    gpu: Gpu,
    imgui: imgui::Context,
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
    /// The number of cuts last used, offered again next time.
    loop_cuts: usize,
    /// The handles shown on the selection, if a handle tool is on.
    gizmo_tool: Option<Kind>,
    /// The face last clicked in Part mode: its part is the one subtract cuts out.
    last_clicked: Option<FaceId>,
    /// View mode: the shape alone, without edges, points or selection colors.
    view_only: bool,
    /// The handle under the mouse.
    gizmo_hover: Option<Handle>,
    /// The history version right after the last bevel, while its panel may still
    /// change it.
    last_bevel: Option<u64>,
    /// The panel's width is 0, so the last bevel is undone for now; a wider width
    /// bevels again.
    bevel_removed: bool,
    /// How far the bevel being dragged moves points per unit of its width type.
    bevel_per_unit: f32,
    /// The version after the last modifier change, and which modifier, so that a
    /// slider drag makes one undo step instead of one per frame.
    last_modifier_edit: Option<(u64, ModifierId)>,
    /// Whether Ctrl is held, for showing snapped values.
    snapping: bool,
    /// The project file being edited, if it has been saved or opened.
    project_path: Option<PathBuf>,
    document_name: String,
    /// The history version when last saved or opened.
    saved_version: u64,
    /// Waiting for an answer about unsaved changes, and the question.
    confirming: Option<(Pending, String)>,
    /// What to do once a typed Save As path arrives.
    after_save: Option<Pending>,
    quit: bool,
    /// The start window, while it shows.
    start: Option<Box<StartState>>,
    settings: Settings,
    /// Whether this run has anti-aliasing, which only changes on a restart.
    antialiasing_now: bool,
    /// Anti-aliasing was asked for at the start, but the driver refused it.
    antialiasing_refused: bool,
    /// The settings window, while it shows.
    settings_window: Option<Box<SettingsState>>,
    /// The rendering engine this run uses, which only changes on a restart.
    renderer_now: String,
    /// The engine the settings asked for at the start, which may have failed.
    renderer_wanted: String,
    /// How much bigger than normal the interface is drawn.
    ui_scale: f32,
    /// The window's size in pixels. The interface works in pixels divided by
    /// `ui_scale`; the 3D view and the mouse in pixels.
    pixels: [f32; 2],
    /// When the project was last saved, by hand or automatically.
    last_save: Instant,
    /// A material shown as edited while its color or sliders are dragged, before it is
    /// saved as an undo step.
    material_preview: Option<(mixer_mesh::MaterialId, mixer_mesh::Material)>,
}

impl App {
    pub fn new(
        mut gpu: Gpu,
        antialiasing_wanted: bool,
        antialiasing_now: bool,
    ) -> Result<Self, String> {
        let mut imgui = imgui::Context::create();
        // No settings file: nothing to save, and it spares the disk.
        imgui.set_ini_filename(None);
        imgui.set_log_filename(None);
        imgui.io_mut().config_input_text_cursor_blink = false;
        let settings = Settings::load();
        let ui_scale = settings.ui_scale();
        let mut fonts = theme::install(&mut imgui);
        if ui_scale != 1.0 {
            fonts = theme::install_fonts(&mut imgui, ui_scale);
        }
        gpu.prepare_ui(&mut imgui)?;
        let mut editor = Editor::new();
        editor.set_undo_steps(settings.undo_steps as usize);
        let renderer_now = gpu.key().to_string();
        let renderer_wanted = settings.renderer.clone();
        let export_folder = places::mixer_folder().join("Exports");
        let export_folder_text = export_folder.display().to_string();
        Ok(Self {
            gpu,
            imgui,
            editor,
            state: PanelState {
                export_format: settings.export_format,
                scale: settings.export_scale,
                ..PanelState::default()
            },
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
            loop_cuts: 1,
            gizmo_tool: None,
            last_clicked: None,
            view_only: false,
            gizmo_hover: None,
            last_bevel: None,
            bevel_removed: false,
            bevel_per_unit: 1.0,
            last_modifier_edit: None,
            snapping: false,
            project_path: None,
            document_name: "Untitled".to_string(),
            saved_version: 0,
            confirming: None,
            after_save: None,
            quit: false,
            start: None,
            settings,
            antialiasing_now,
            antialiasing_refused: antialiasing_wanted && !antialiasing_now,
            settings_window: None,
            renderer_now,
            renderer_wanted,
            ui_scale,
            pixels: [1.0, 1.0],
            last_save: Instant::now(),
            material_preview: None,
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

    /// An error the graphics driver reported, if any, for the log.
    pub fn gpu_error(&self) -> Option<String> {
        self.gpu.error()
    }

    /// Shows the frame drawn last.
    pub fn present(&mut self, window: &Window) {
        self.gpu.present(window);
    }

    /// A message for the status bar, and for the start window if it shows.
    pub fn note(&mut self, text: String) {
        if let Some(start) = self.start.as_mut() {
            start.message = text.clone();
        }
        self.state.message = text;
    }

    /// Draws one frame into the back buffer. The caller swaps it onto the screen.
    pub fn draw(&mut self, window: &Window) {
        let (width, height) = window.drawable_size();
        let (width, height) = (width as i32, height as i32);

        self.pixels = [width as f32, height as f32];
        let io = self.imgui.io_mut();
        let s = self.ui_scale;
        io.display_size = [width as f32 / s, height as f32 / s];
        io.display_framebuffer_scale = [s, s];
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
        let parts_selected = mesh
            .parts()
            .iter()
            .filter(|part| {
                part.iter()
                    .any(|&f| mesh.face(f).is_some_and(|f| f.is_selected()))
            })
            .count();
        let mut materials: Vec<crate::ui::MaterialRow> = mesh
            .materials()
            .into_iter()
            .map(|(id, material)| crate::ui::MaterialRow {
                id,
                material,
                faces: 0,
                selected: 0,
            })
            .collect();
        let mut plain_faces = (0, 0);
        for (_, f) in mesh.faces() {
            let counts = match f.material() {
                Some(m) => match materials.iter_mut().find(|r| r.id == m) {
                    Some(row) => (&mut row.faces, &mut row.selected),
                    None => continue,
                },
                None => (&mut plain_faces.0, &mut plain_faces.1),
            };
            *counts.0 += 1;
            *counts.1 += usize::from(f.is_selected());
        }
        // A material being edited shows as edited.
        if let Some((id, draft)) = self.material_preview
            && let Some(row) = materials.iter_mut().find(|r| r.id == id)
        {
            row.material = draft;
        }
        let info = PanelInfo {
            materials,
            plain_faces,
            // What gets exported, modifiers' copies included.
            triangles: self.scene.triangle_count(),
            triangle_limit: self.settings.triangle_limit(),
            snap: self.settings.snap,
            vertices: mesh.vertex_count(),
            faces: mesh.face_count(),
            warnings: &self.warnings,
            target,
            mode: self.editor.mode_choice(),
            selected_vertices: mesh.vertices().filter(|(_, v)| v.is_selected()).count(),
            transform_help,
            transform_waiting: self.bevel_waiting(),
            transform_dragging: self.dragging_handle(),
            loop_cut: match &self.mouse_state {
                Mouse::LoopCut(choice) => Some(choice.cuts),
                _ => None,
            },
            bevel: match &self.mouse_state {
                Mouse::Transforming { transform, .. } if transform.kind == Kind::Bevel => {
                    let size = self.pixels;
                    let input = Input {
                        mouse: self.pointer,
                        snap: self.snapping,
                        steps: self.settings.snap,
                        camera: self.camera,
                        width: size[0].max(1.0),
                        height: size[1].max(1.0),
                    };
                    Some((
                        transform.bevel_width(&input) / self.bevel_per_unit.max(1e-9),
                        self.state.bevel.segments,
                        self.state.bevel.width_type,
                    ))
                }
                _ => None,
            },
            undo_label: document.undo_label(),
            redo_label: document.redo_label(),
            export_folder: &self.export_folder_text,
            open_edges: mesh.open_edge_count(),
            document: &self.document_name,
            unsaved: document.version() != self.saved_version,
            confirm: self
                .confirming
                .as_ref()
                .map(|(_, question)| question.as_str()),
            gizmo_tool: self.gizmo_tool,
            knife: match &self.mouse_state {
                Mouse::Knife(choice) => Some(choice.through),
                _ => None,
            },
            bisect: matches!(self.mouse_state, Mouse::Bisect(_)),
            view_only: self.view_only,
            parts_selected,
            modifiers: if parts_selected == 1 {
                mesh.modifiers_of_selected()
            } else {
                Vec::new()
            },
            bevel_panel: self.last_bevel == Some(document.version())
                && matches!(self.mouse_state, Mouse::Idle),
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
        let gizmo_parts = self.gizmo_parts();
        // Blender's dashed line from the selection's center to the mouse, while a bevel,
        // a rotation or a scale follows the mouse (not while a handle is dragged).
        let guide = match &self.mouse_state {
            Mouse::Transforming { transform, .. }
                if !transform.waiting
                    && (transform.kind == Kind::Bevel
                        || (matches!(transform.kind, Kind::Rotate | Kind::Scale)
                            && !transform.release_confirms)) =>
            {
                let (width, height) = self.viewport();
                let center = transform.pivot_on_screen(&Input {
                    mouse: self.pointer,
                    snap: false,
                    steps: self.settings.snap,
                    camera: self.camera,
                    width,
                    height,
                });
                Some((center, self.pointer, false))
            }
            _ => None,
        };
        let ui = self.imgui.new_frame();
        let mut start_action = None;
        let mut settings_action = None;
        let whole = crate::ui::UiOutput {
            action: None,
            rects: vec![[0.0, 0.0, width as f32, height as f32]],
        };
        let output = match (&mut self.settings_window, &mut self.start) {
            (Some(window), _) => {
                settings_action = build_settings(ui, window, &self.fonts);
                whole
            }
            (None, Some(start)) => {
                start_action = build_start(ui, start, &self.fonts);
                whole
            }
            (None, None) => build_ui(ui, &mut self.state, &info, &self.fonts),
        };
        let action = output.action;
        if let Mouse::Selecting {
            region,
            moved: true,
            ..
        } = &self.mouse_state
        {
            draw_region(ui, region);
        }
        if let Mouse::LoopCut(choice) = &self.mouse_state {
            draw_lines(ui, &choice.lines);
        }
        draw_gizmo(ui, &gizmo_parts, self.gizmo_hover);
        match &self.mouse_state {
            Mouse::Knife(choice) => {
                let mut path: Vec<[f32; 2]> = choice.clicks.clone();
                path.push(self.pointer);
                draw_knife(ui, &path, &choice.preview.screen);
            }
            Mouse::Bisect(choice) => {
                if let Some(start) = choice.start {
                    draw_lines(ui, &[[start, choice.end]]);
                }
            }
            _ => {}
        }
        if let Some((center, pointer, held)) = guide {
            draw_guide(ui, center, pointer, held);
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

        let modified = Scene::needs_rebuild_to_move(self.editor.document().mesh());
        if self.stale == Stale::Everything || (self.stale == Stale::Moved && modified) {
            let mesh = self.editor.document().mesh();
            match self.material_preview {
                // A copy with the material as being edited.
                Some((id, draft)) => {
                    let mut copy = mesh.clone();
                    let _ = copy.set_material(id, draft);
                    self.scene.rebuild(&copy, self.editor.drawing_mode());
                }
                None => self.scene.rebuild(mesh, self.editor.drawing_mode()),
            }
            self.gpu.upload_scene(&self.scene);
            // Warnings are about what gets exported.
            let shown = mesh.evaluated();
            self.warnings = warnings(
                shown.triangle_count(),
                shown.open_edge_count(),
                self.settings.triangle_limit(),
            );
        } else if self.stale == Stale::Moved
            && let Mouse::Transforming { parts, .. } = &self.mouse_state
        {
            let changed = self
                .scene
                .update_moved(self.editor.document().mesh(), parts);
            self.gpu.update_scene(&self.scene, &changed);
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
            overlays: !self.view_only,
            grid: self.settings.grid,
            shadows: self.settings.shadows,
            plain_edges: !self.settings.shadows && self.settings.flat_edges,
        };
        self.gpu.draw(width, height, &view, draw_data);

        self.frames_drawn += 1;
        self.frames_wanted = self.frames_wanted.saturating_sub(1);

        if let Some(action) = action {
            self.apply_action(action);
            self.request_frames();
        }
        if let Some(action) = start_action {
            self.apply_start(action);
            self.request_frames();
        }
        if let Some(action) = settings_action {
            self.apply_settings_window(action);
            self.request_frames();
        }
    }

    /// Opens the settings window over whatever shows.
    fn open_settings(&mut self) {
        self.settings_window = Some(Box::new(SettingsState {
            page: 0,
            values: self.settings.clone(),
            antialiasing_now: self.antialiasing_now,
            antialiasing_refused: self.antialiasing_refused,
            renderer_now: self.renderer_now.clone(),
            renderer_wanted: self.renderer_wanted.clone(),
            engines: crate::engines::read(),
            theme: crate::palette::current(),
        }));
        self.mouse_state = Mouse::Idle;
        self.request_frames();
    }

    fn apply_settings_window(&mut self, action: SettingsAction) {
        match action {
            SettingsAction::Close => self.settings_window = None,
            SettingsAction::Changed => {
                let Some(window) = &self.settings_window else {
                    return;
                };
                let new = window.values.clone();
                let theme = window.theme;
                if theme != crate::palette::current() {
                    crate::palette::set(&theme);
                    theme::install_style(&mut self.imgui);
                    if let Err(error) = crate::palette::save(&theme) {
                        self.state.message = format!("Could not save the theme: {error}");
                    }
                    // Edges and points take their colors when the scene is built.
                    self.stale = Stale::Everything;
                }
                self.change_settings(new);
            }
        }
    }

    /// Takes new settings: saves them, and applies what can change while Mixer runs.
    fn change_settings(&mut self, new: Settings) {
        let old = std::mem::replace(&mut self.settings, new);
        let new = &self.settings;
        if new.ui_scale() != old.ui_scale() {
            self.ui_scale = new.ui_scale();
            self.fonts = theme::install_fonts(&mut self.imgui, self.ui_scale);
            self.gpu.reload_fonts(&mut self.imgui);
        }
        if new.triangle_limit != old.triangle_limit {
            // The export warnings are worked out again with the new limit.
            self.stale = Stale::Everything;
        }
        if new.undo_steps != old.undo_steps {
            self.editor.set_undo_steps(new.undo_steps as usize);
        }
        if new.recent != old.recent
            && let Some(start) = self.start.as_mut()
        {
            start.recent = places::recent_projects(new.recent as usize);
        }
        if let Err(error) = self.settings.save() {
            self.state.message = format!("Could not save the settings: {error}");
        }
    }

    /// How long until an automatic save is due, if one will be.
    pub fn autosave_wait(&self) -> Option<Duration> {
        let minutes = self.settings.autosave;
        if minutes == 0 || self.project_path.is_none() {
            return None;
        }
        let every = Duration::from_secs(u64::from(minutes) * 60);
        Some(every.saturating_sub(self.last_save.elapsed()))
    }

    /// Saves the project if an automatic save is due and nothing is in the way: there
    /// are unsaved changes, and no drag, question or window is open.
    pub fn tick(&mut self) {
        if self.autosave_wait() != Some(Duration::ZERO) {
            return;
        }
        let busy = !matches!(self.mouse_state, Mouse::Idle)
            || self.start.is_some()
            || self.settings_window.is_some()
            || self.confirming.is_some()
            || self.state.path_prompt.is_some()
            || self.editor.document().version() == self.saved_version;
        if busy {
            // Tries again a little later.
            self.last_save = Instant::now()
                .checked_sub(Duration::from_secs(u64::from(self.settings.autosave) * 60))
                .map_or_else(Instant::now, |due| due + Duration::from_secs(10));
            return;
        }
        if let Some(path) = self.project_path.clone()
            && self.save_to(path)
        {
            self.state.message = format!("Auto-saved {}", self.document_name);
        }
        self.request_frames();
    }

    /// Shows the start window, with the recent projects as they are now.
    pub fn show_start(&mut self) {
        let folder = if std::env::var_os("MIXER_HOME").is_some() {
            places::projects_folder().display().to_string()
        } else {
            "Documents > Mixer > Projects".to_string()
        };
        let start = StartState::new(
            places::recent_projects(self.settings.recent as usize),
            folder,
            self.settings.export_format,
            self.settings.export_scale,
        );
        self.start = Some(Box::new(start));
        self.mouse_state = Mouse::Idle;
        self.request_frames();
    }

    fn apply_start(&mut self, action: StartAction) {
        let Some(start) = self.start.as_mut() else {
            return;
        };
        start.message.clear();
        match action {
            StartAction::Quick => {
                // Each quick project in a folder of its own, with its exports inside.
                let path = places::unused_project_path(
                    &places::quick_folder(),
                    &format!("Quick {}", places::timestamp()),
                    PROJECT_EXTENSION,
                );
                self.state.export_format = self.settings.export_format;
                self.state.scale = self.settings.export_scale;
                self.begin_project(path, Some(Shape::Cube));
            }
            StartAction::ShowRecent(i) => {
                if let Some(recent) = start.recent.get(i)
                    && let Err(error) = places::show_in_folder(&recent.path)
                {
                    start.message = error;
                }
            }
            StartAction::DeleteRecent(i) => start.confirm_delete = Some(i),
            StartAction::CancelDelete => start.confirm_delete = None,
            StartAction::ConfirmDelete => {
                let Some(recent) = start
                    .confirm_delete
                    .take()
                    .and_then(|i| start.recent.get(i))
                else {
                    return;
                };
                let path = recent.path.clone();
                let name = file_stem(&path);
                match places::recycle(&places::deletion_target(&path)) {
                    Ok(()) => {
                        places::forget(&path);
                        // The open project is gone: nothing is left to save it to.
                        if self.project_path.as_deref() == Some(path.as_path()) {
                            self.project_path = None;
                            self.saved_version = self.editor.document().version();
                        }
                        start.recent = places::recent_projects(self.settings.recent as usize);
                        start.message = if cfg!(windows) {
                            format!("{name} is in the Recycle Bin.")
                        } else {
                            format!("{name} was moved to the Deleted folder.")
                        };
                    }
                    Err(error) => start.message = format!("Could not delete {name}: {error}"),
                }
            }
            StartAction::Create => {
                let Some(name) = places::clean_project_name(&start.name) else {
                    start.message = "Type a name for the project first.".to_string();
                    return;
                };
                let folder = places::projects_folder().join(&name);
                let path = folder.join(format!("{name}.{PROJECT_EXTENSION}"));
                if path.exists() {
                    start.message = format!(
                        "A project called {name} already exists. Open it from Recent, or pick another name."
                    );
                    return;
                }
                let (shape, format, scale) = (start.shape, start.export_format, start.scale);
                self.state.export_format = format;
                self.state.scale = scale;
                self.begin_project(path, shape);
            }
            StartAction::OpenRecent(i) => {
                if let Some(recent) = start.recent.get(i) {
                    let path = recent.path.clone();
                    self.perform(Pending::Open(Some(path)));
                }
            }
            StartAction::OpenOther => {
                if let Picked::Path(path) = self.pick_path(PathPurpose::Open) {
                    self.perform(Pending::Open(Some(path)));
                } else if let Some(start) = self.start.as_mut() {
                    // Without Windows's dialog, a path can be given when starting Mixer.
                    self.state.path_prompt = None;
                    if !cfg!(windows) {
                        start.message =
                            "Start Mixer with a file's path to open it on this system.".to_string();
                    }
                }
            }
            StartAction::OpenFolder => {
                let folder = places::mixer_folder();
                let _ = std::fs::create_dir_all(places::projects_folder());
                #[cfg(windows)]
                {
                    let _ = std::process::Command::new("explorer").arg(&folder).spawn();
                }
                #[cfg(not(windows))]
                {
                    start.message = format!("The Mixer folder is {}", folder.display());
                }
            }
            StartAction::Settings => self.open_settings(),
        }
    }

    /// Makes a new project file at `path`, starting with `shape`, and opens the editor.
    fn begin_project(&mut self, path: PathBuf, shape: Option<Shape>) {
        self.editor.replace(Mesh::new());
        self.camera = Camera::default();
        self.state.file_name = file_stem(&path);
        self.state.message.clear();
        if let Some(shape) = shape {
            let mut settings = self.state.clone();
            settings.location = [0.0; 3];
            if let Err(error) = self.editor.add(shape, &settings) {
                self.state.message = format!("Could not add the shape: {error}");
            }
        }
        self.export_folder = places::exports_beside(&path);
        self.export_folder_text = self.export_folder.display().to_string();
        self.stale = Stale::Everything;
        if self.save_to(path) {
            self.start = None;
        } else if let Some(start) = self.start.as_mut() {
            start.message = self.state.message.clone();
        }
        self.request_frames();
    }

    pub fn read_pixels(&self, window: &Window) -> (i32, i32, Vec<u8>) {
        let (width, height) = window.drawable_size();
        let (width, height) = (width as i32, height as i32);
        (width, height, self.gpu.read_pixels(width, height))
    }

    /// The mouse wheel or a touchpad's two-finger scroll, over the 3D view.
    fn wheel_view(&mut self, x: f32, y: f32, mods: Modifiers) {
        /// Pixels of turning or sliding per scroll unit, in touchpad mode.
        const TOUCHPAD_PIXELS: f32 = 20.0;
        let (width, height) = self.viewport();
        let zoom = |app: &mut Self, steps: f32| {
            let steps = if app.settings.invert_zoom {
                -steps
            } else {
                steps
            };
            if app.settings.zoom_to_pointer {
                app.camera.zoom_at(steps, app.pointer, width, height);
            } else {
                app.camera.zoom_steps(steps);
            }
        };
        if self.settings.touchpad {
            if mods.ctrl {
                zoom(self, y);
            } else if mods.shift {
                self.camera
                    .pan_pixels(x * TOUCHPAD_PIXELS, y * TOUCHPAD_PIXELS, height);
            } else {
                let speed = self.settings.turn_speed as f32 / 100.0 * TOUCHPAD_PIXELS;
                self.camera.orbit_pixels(x * speed, y * speed);
            }
        } else if y != 0.0 {
            zoom(self, y);
        }
    }

    /// The 3D view's size in pixels.
    fn viewport(&self) -> (f32, f32) {
        (self.pixels[0].max(1.0), self.pixels[1].max(1.0))
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
        // Changing the shape needs to see it, so it ends View mode.
        if self.view_only && changes_the_shape(action) {
            self.view_only = false;
        }
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
            Action::New => self.request(Pending::New),
            Action::Settings => self.open_settings(),
            Action::Search => {
                self.state.search = Some(crate::search::SearchState {
                    focus: true,
                    ..Default::default()
                });
            }
            Action::StartWindow => self.request(Pending::Start),
            Action::Open => self.request(Pending::Open(None)),
            Action::Save => self.save(false, None),
            Action::SaveAs => self.save(true, None),
            Action::ConfirmSave => {
                if let Some((pending, _)) = self.confirming.take() {
                    self.save(false, Some(pending));
                }
            }
            Action::ConfirmDiscard => {
                if let Some((pending, _)) = self.confirming.take() {
                    self.perform(pending);
                }
            }
            Action::ConfirmCancel => self.confirming = None,
            Action::PathChosen => {
                if let Some(prompt) = self.state.path_prompt.take() {
                    let path = PathBuf::from(prompt.text.trim());
                    match prompt.purpose {
                        PathPurpose::Open => self.perform(Pending::Open(Some(path))),
                        PathPurpose::SaveAs => {
                            let then = self.after_save.take();
                            if self.save_to(path)
                                && let Some(pending) = then
                            {
                                self.perform(pending);
                            }
                        }
                    }
                }
            }
            Action::PathCancelled => {
                self.state.path_prompt = None;
                self.after_save = None;
            }
            Action::Export => {
                self.state.message = match write_export(
                    self.editor.document().mesh(),
                    &self.export_folder,
                    &self.state.file_name,
                    self.state.scale,
                    self.state.export_format,
                ) {
                    Ok(path) => {
                        if self.settings.open_export_folder {
                            let _ = places::show_in_folder(&path);
                        }
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
            Action::SelectLoops | Action::SelectRings => {
                let ring = action == Action::SelectRings;
                let label = if ring { "Select rings" } else { "Select loops" };
                let result = self.editor.run(label, |mesh, mode| {
                    mesh.select_loops_of_selection(mode, ring)
                });
                self.finish_edit(result);
            }
            Action::LoopCut => self.start_loop_cut(),
            Action::Bevel | Action::BevelVertices => {
                self.state.bevel.vertices = action == Action::BevelVertices;
                self.start_transform(Kind::Bevel, Prepare::Bevel(Box::new(self.state.bevel)));
            }
            Action::AdjustBevel => self.adjust_bevel(),
            Action::GizmoTool(kind) => {
                self.gizmo_tool = if self.gizmo_tool == Some(kind) {
                    None
                } else {
                    Some(kind)
                };
                self.gizmo_hover = None;
            }
            Action::Bridge => {
                let result = self
                    .editor
                    .run("Bridge", |mesh, mode| mesh.bridge_selected_loops(mode));
                self.finish_edit(result);
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
            Action::Merge if self.editor.whole_parts() => {
                self.state.message =
                    "M pulls points together. Pick Vertex mode, select the points, then press M. To join shapes, use Join or Union.".to_string();
            }
            Action::Join => {
                let mut joined = 0;
                let result = self.editor.run("Join", |mesh, _| {
                    joined = mesh.join_selected_parts()?;
                    Ok(())
                });
                if result.is_ok() {
                    self.state.message = format!("Joined {joined} parts into one.");
                }
                self.finish_edit(result);
            }
            Action::Separate => {
                let result = self.editor.run("Separate", |mesh, _| {
                    mesh.separate_selected_parts().map(|_| ())
                });
                if result.is_ok() {
                    self.state.message = "Separated.".to_string();
                }
                self.finish_edit(result);
            }
            Action::Boolean(op) => self.boolean(op),
            Action::AddModifier(kind) => {
                let (modifier, name) = match kind {
                    ModifierKind::Mirror => (Modifier::mirror(), "Mirror"),
                    ModifierKind::Array => (Modifier::array(), "Array"),
                };
                let result = self.editor.run("Add modifier", |mesh, _| {
                    mesh.add_modifier_to_selected(modifier).map(|_| ())
                });
                if result.is_ok() {
                    self.state.message =
                        format!("{name} added. Edit the shape and the copy follows.");
                }
                self.finish_edit(result);
            }
            Action::ChangeModifier => self.change_modifier(),
            Action::RemoveModifier(id) => {
                let result = self
                    .editor
                    .run("Remove modifier", |mesh, _| mesh.remove_modifier(id));
                self.finish_edit(result);
            }
            Action::ApplyModifiers => {
                let mut faces = 0;
                let result = self.editor.run("Apply modifiers", |mesh, mode| {
                    faces = mesh.apply_modifiers_of_selected(mode)?;
                    Ok(())
                });
                if result.is_ok() {
                    self.state.message = format!("Applied: the shape now has {faces} real faces.");
                }
                self.finish_edit(result);
            }
            Action::AddMaterial => {
                // A copy of the chosen one, or a fresh one, with a name not yet used.
                let mesh = self.editor.document().mesh();
                let base = self
                    .state
                    .active_material
                    .and_then(|id| mesh.material(id))
                    .unwrap_or_else(|| mixer_mesh::Material::new("Material"));
                let taken: Vec<String> = mesh
                    .materials()
                    .iter()
                    .map(|(_, m)| m.name().to_string())
                    .collect();
                let stem = base
                    .name()
                    .trim_end_matches(|c: char| c.is_ascii_digit() || c == '.')
                    .to_string();
                let stem = if stem.is_empty() {
                    "Material".to_string()
                } else {
                    stem
                };
                let mut name = stem.clone();
                let mut n = 1;
                while taken.contains(&name) {
                    name = format!("{stem}.{n:03}");
                    n += 1;
                }
                let mut material = base;
                material.set_name(&name);
                let mut made = None;
                let result = self.editor.run("New material", |mesh, _| {
                    made = Some(mesh.add_material(material)?);
                    Ok(())
                });
                if result.is_ok() {
                    self.state.active_material = made;
                    self.state.message = format!("Made the material {name}.");
                }
                self.finish_edit(result);
            }
            Action::RemoveMaterial(id) => {
                self.material_preview = None;
                let result = self
                    .editor
                    .run("Remove material", |mesh, _| mesh.remove_material(id));
                if result.is_ok() {
                    self.state.active_material = None;
                }
                self.finish_edit(result);
            }
            Action::ChangeSnap => {
                if let Some(steps) = self.state.snap_draft.take() {
                    self.settings.snap = steps.clamped();
                    if let Err(error) = self.settings.save() {
                        self.state.message = format!("Could not save the settings: {error}");
                    }
                }
            }
            Action::PreviewMaterial => {
                self.material_preview = self.state.material_draft;
                self.stale = Stale::Everything;
            }
            Action::ChangeMaterial => {
                self.material_preview = None;
                if let Some((id, draft)) = self.state.material_draft {
                    let result = self
                        .editor
                        .run("Change material", |mesh, _| mesh.set_material(id, draft));
                    self.finish_edit(result);
                }
                self.stale = Stale::Everything;
            }
            Action::AssignMaterial(material) => {
                let mut changed = 0;
                let result = self.editor.run("Assign material", |mesh, _| {
                    changed = mesh.assign_material_selected(material)?;
                    Ok(())
                });
                if result.is_ok() {
                    let faces = if changed == 1 { "face" } else { "faces" };
                    self.state.message = format!("Gave {changed} {faces} the material.");
                }
                self.finish_edit(result);
            }
            Action::SelectMaterial(material, select) => {
                let result = self.editor.run("Select by material", |mesh, mode| {
                    let faces: Vec<mixer_mesh::Element> = mesh
                        .faces_with_material(material)
                        .into_iter()
                        .map(mixer_mesh::Element::Face)
                        .collect();
                    if faces.is_empty() {
                        return Ok(());
                    }
                    let op = if select {
                        mixer_mesh::SelectOp::Add
                    } else {
                        mixer_mesh::SelectOp::Subtract
                    };
                    mesh.select_elements(mixer_mesh::SelectMode::Face, &faces, op)?;
                    mesh.flush_selection(mode);
                    Ok(())
                });
                self.finish_edit(result);
            }
            Action::MarkSharp(sharp) => {
                let mut changed = 0;
                let result = self.editor.run("Sharp edges", |mesh, _| {
                    changed = mesh.set_sharp_selected(sharp)?;
                    Ok(())
                });
                if result.is_ok() {
                    let what = if sharp { "Marked" } else { "Cleared" };
                    self.state.message = format!("{what} {changed} sharp edges.");
                }
                self.finish_edit(result);
            }
            Action::Shade(angle) => {
                let mut changed = 0;
                let result = self.editor.run("Shading", |mesh, _| {
                    changed = mesh.set_shading_selected(angle)?;
                    Ok(())
                });
                if result.is_ok() {
                    let how = match angle {
                        0 => "Flat",
                        180 => "Smooth",
                        _ => "Auto smooth",
                    };
                    self.state.message = format!("{how} shading on {changed} faces.");
                }
                self.finish_edit(result);
                self.stale = Stale::Everything;
            }
            Action::Knife => self.start_knife(),
            Action::Bisect => self.start_bisect(),
            Action::ToggleView => {
                self.view_only = !self.view_only;
                self.gizmo_hover = None;
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

    fn has_unsaved_changes(&self) -> bool {
        self.editor.document().version() != self.saved_version
    }

    /// Does something that replaces the model, asking first if that would lose changes.
    fn request(&mut self, pending: Pending) {
        if self.has_unsaved_changes() {
            let question = match pending {
                Pending::Quit => format!("Save changes to {} before closing?", self.document_name),
                _ => format!("Save changes to {} first?", self.document_name),
            };
            self.confirming = Some((pending, question));
        } else {
            self.perform(pending);
        }
        self.request_frames();
    }

    /// Asks for a file: with Windows's own dialog, or elsewhere in the app's own box.
    fn pick_path(&mut self, purpose: PathPurpose) -> Picked {
        let folder = self
            .project_path
            .as_ref()
            .and_then(|p| p.parent().map(PathBuf::from))
            .unwrap_or_else(|| self.export_folder.with_file_name("projects"));
        #[cfg(any(windows, check_windows))]
        {
            use crate::platform::{FileType, pick_file};
            let _ = std::fs::create_dir_all(&folder);
            let chosen = match purpose {
                PathPurpose::Open => pick_file(
                    false,
                    "Open",
                    &[
                        FileType {
                            name: "Projects and OBJ files",
                            pattern: "*.mix;*.lem;*.obj",
                        },
                        FileType {
                            name: "All files",
                            pattern: "*.*",
                        },
                    ],
                    &folder,
                    "",
                    PROJECT_EXTENSION,
                ),
                PathPurpose::SaveAs => pick_file(
                    true,
                    "Save as",
                    &[FileType {
                        name: "Mixer project",
                        pattern: "*.mix",
                    }],
                    &folder,
                    &self.document_name,
                    PROJECT_EXTENSION,
                ),
            };
            match chosen {
                Some(path) => Picked::Path(path),
                None => Picked::Cancelled,
            }
        }
        #[cfg(not(any(windows, check_windows)))]
        {
            let name = match purpose {
                PathPurpose::Open => String::new(),
                PathPurpose::SaveAs => format!("{}.{PROJECT_EXTENSION}", self.state.file_name),
            };
            self.state.path_prompt = Some(crate::ui::PathPrompt {
                purpose,
                text: folder.join(name).display().to_string(),
            });
            Picked::Asking
        }
    }

    /// Saves to the open project, or asks where first. Then does `then`, if saved.
    fn save(&mut self, as_new: bool, then: Option<Pending>) {
        let path = match (&self.project_path, as_new) {
            (Some(path), false) => path.clone(),
            _ => match self.pick_path(PathPurpose::SaveAs) {
                Picked::Path(path) => path,
                Picked::Cancelled => return,
                Picked::Asking => {
                    self.after_save = then;
                    return;
                }
            },
        };
        if self.save_to(path)
            && let Some(pending) = then
        {
            self.perform(pending);
        }
    }

    fn save_to(&mut self, mut path: PathBuf) -> bool {
        if path.extension().is_none() {
            path.set_extension(PROJECT_EXTENSION);
        }
        let settings = ProjectSettings {
            export_format: Some(self.state.export_format.extension().to_string()),
            export_scale: Some(self.state.scale),
            export_name: Some(self.state.file_name.clone()),
        };
        match save_project(self.editor.document().mesh(), &settings, &path) {
            Ok(()) => {
                places::remember(&path);
                self.document_name = file_name(&path);
                self.state.message = format!("Saved {}", self.document_name);
                self.project_path = Some(path);
                self.saved_version = self.editor.document().version();
                self.last_save = Instant::now();
                true
            }
            Err(error) => {
                self.state.message = error;
                false
            }
        }
    }

    fn perform(&mut self, pending: Pending) {
        match pending {
            Pending::Quit => self.quit = true,
            Pending::New => {
                self.editor.replace(Mesh::new());
                self.project_path = None;
                self.document_name = "Untitled".to_string();
                self.saved_version = self.editor.document().version();
                self.state.message = "New project.".to_string();
                self.stale = Stale::Everything;
            }
            Pending::Open(None) => {
                if let Picked::Path(path) = self.pick_path(PathPurpose::Open) {
                    self.perform(Pending::Open(Some(path)));
                }
            }
            Pending::Open(Some(path)) => match open_file(&path) {
                Ok((mesh, report, kind)) => {
                    self.editor.replace(mesh);
                    self.state.file_name = file_stem(&path);
                    if kind == FileKind::Project {
                        self.project_path = Some(path.clone());
                        self.document_name = file_name(&path);
                        self.export_folder = places::exports_beside(&path);
                        self.export_folder_text = self.export_folder.display().to_string();
                        let settings = &report.settings;
                        if let Some(format) = settings
                            .export_format
                            .as_deref()
                            .and_then(ExportFormat::from_name)
                        {
                            self.state.export_format = format;
                        }
                        if let Some(scale) = settings.export_scale {
                            self.state.scale = scale;
                        }
                        if let Some(name) = &settings.export_name {
                            self.state.file_name = name.clone();
                        }
                        places::remember(&path);
                    } else {
                        // An imported OBJ is saved as a new project, never over the OBJ.
                        self.project_path = None;
                        self.document_name = format!("{} (imported)", file_stem(&path));
                    }
                    self.saved_version = self.editor.document().version();
                    let skipped = match report.skipped_faces {
                        0 => String::new(),
                        1 => " 1 broken face was left out.".to_string(),
                        n => format!(" {n} broken faces were left out."),
                    };
                    self.state.message = format!("Opened {}.{skipped}", file_name(&path));
                    self.frame_model();
                    self.stale = Stale::Everything;
                    self.start = None;
                }
                Err(error) => match self.start.as_mut() {
                    Some(start) => start.message = error,
                    None => self.state.message = error,
                },
            },
            Pending::Start => self.show_start(),
        }
        self.request_frames();
    }

    /// Points the camera at the whole model, as after opening a file.
    fn frame_model(&mut self) {
        let mesh = self.editor.document().mesh();
        let mut low = [f32::MAX; 3];
        let mut high = [f32::MIN; 3];
        for (_, vertex) in mesh.vertices() {
            let p = vertex.position();
            for i in 0..3 {
                low[i] = low[i].min(p[i]);
                high[i] = high[i].max(p[i]);
            }
        }
        self.camera = Camera::default();
        if low[0] <= high[0] {
            self.camera.target = std::array::from_fn(|i| (low[i] + high[i]) / 2.0);
            let size = (0..3).map(|i| high[i] - low[i]).fold(0.0, f32::max);
            // Room around it, so the whole shape and some ground show.
            self.camera.distance = (size * 3.0).clamp(6.0, 5000.0);
        }
    }

    /// Opens a file named when the app was started.
    /// What shows when Mixer starts without a file: the start window, or, if the
    /// settings turn it off, the most recent project.
    pub fn open_first(&mut self) {
        let recent = places::recent_projects(1);
        match recent.first() {
            Some(last) if !self.settings.start_window => {
                let path = last.path.clone();
                self.open_at_start(path);
            }
            _ => self.show_start(),
        }
    }

    pub fn open_at_start(&mut self, path: PathBuf) {
        self.perform(Pending::Open(Some(path)));
    }

    /// The window is closing. True if it may close now; otherwise the app asks about
    /// unsaved changes first, and `quit_now` turns true once that is settled.
    pub fn request_quit(&mut self) -> bool {
        if matches!(self.mouse_state, Mouse::Transforming { .. }) {
            self.finish_transform(false);
        }
        self.mouse_state = Mouse::Idle;
        if self.has_unsaved_changes() {
            self.request(Pending::Quit);
            false
        } else {
            true
        }
    }

    pub fn quit_now(&self) -> bool {
        self.quit
    }

    /// Starts dragging the selection. Anything prepared first shares its undo step, so
    /// cancelling the drag removes it again.
    fn start_transform(&mut self, kind: Kind, prepare: Prepare) {
        if !matches!(self.mouse_state, Mouse::Idle) {
            return;
        }
        let cutting = matches!(prepare, Prepare::LoopCut(..));
        if !cutting && self.editor.document().mesh().selection_center().is_none() {
            self.state.message = "Select something first.".to_string();
            return;
        }
        let label = match prepare {
            Prepare::Nothing => kind.name(),
            Prepare::Duplicate => "Duplicate",
            Prepare::Extrude => "Extrude",
            Prepare::Inset => "Inset",
            Prepare::LoopCut(..) => "Loop cut",
            Prepare::Bevel(_) => "Bevel",
        };
        if let Err(error) = self.editor.begin(label) {
            self.state.message = format!("That did not work: {error}");
            return;
        }
        let mode = self.editor.mode();
        let mut direction = None;
        let mut inset_targets = Vec::new();
        let mut cut_edges = None;
        let mut bevel = None;
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
                Prepare::LoopCut(edge, cuts) => mesh
                    .loop_cut(edge, cuts, mode)
                    .map(|made| cut_edges = Some(made)),
                Prepare::Bevel(ref settings) => mesh
                    .bevel_selected(
                        &BevelSettings {
                            width: 0.0,
                            ..**settings
                        },
                        mode,
                    )
                    .map(|made| bevel = Some(made)),
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
            self.gpu.upload_scene(&self.scene);
        }
        let mesh = self.editor.document().mesh();
        let rails = if kind == Kind::Slide {
            let edges = cut_edges.unwrap_or_else(|| {
                mesh.edges()
                    .filter(|(_, e)| e.is_selected())
                    .map(|(id, _)| id)
                    .collect()
            });
            match mesh.edge_slide_rails(&edges) {
                Ok(rails) => rails,
                Err(error) => {
                    // A cut with nothing to slide along is still a finished cut.
                    let result = if cutting {
                        self.editor.commit().map(|_| ())
                    } else {
                        self.state.message = format!("Could not slide: {error}");
                        self.editor.cancel()
                    };
                    self.finish_edit(result);
                    return;
                }
            }
        } else {
            Vec::new()
        };
        let (moving, centers): (Vec<_>, Vec<_>) = if prepare == Prepare::Inset {
            inset_targets.iter().copied().unzip()
        } else if let Some(made) = &bevel {
            (made.moving.iter().map(|m| m.0).collect(), Vec::new())
        } else if kind == Kind::Slide {
            (rails.iter().map(|r| r.0).collect(), Vec::new())
        } else {
            (mesh.selected_vertices(), Vec::new())
        };
        let originals: Vec<_> = moving
            .iter()
            .filter_map(|&id| Some((id, mesh.vertex(id)?.position())))
            .collect();
        let pivot = if kind == Kind::Slide {
            let count = originals.len().max(1) as f32;
            let mut sum = [0.0; 3];
            for (_, p) in &originals {
                for i in 0..3 {
                    sum[i] += p[i] / count;
                }
            }
            sum
        } else {
            mesh.selection_center().unwrap_or([0.0; 3])
        };
        let parts = Scene::moved_parts(mesh, &moving);
        let mut transform = Transform::new(kind, self.pointer, originals, pivot);
        transform.direction = direction;
        transform.centers = centers;
        transform.set_rails(
            &rails
                .iter()
                .map(|&(_, start, ends)| (start, ends))
                .collect::<Vec<_>>(),
        );
        if let Some(made) = bevel {
            transform.bevel_steps = made.moving.iter().map(|m| m.2).collect();
            self.bevel_per_unit = made.per_unit;
            transform.max_width = made.max_width;
        }
        transform.keep_on_cancel = cutting;
        transform.plain = prepare == Prepare::Nothing;
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
        if transform.waiting {
            return;
        }
        let input = Input {
            mouse: self.pointer,
            snap,
            steps: self.settings.snap,
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
        let Mouse::Transforming { transform, .. } =
            std::mem::replace(&mut self.mouse_state, Mouse::Idle)
        else {
            return;
        };
        let mut confirm = confirm;
        let bevel_width = (confirm && transform.kind == Kind::Bevel).then(|| {
            let (width, height) = self.viewport();
            transform.bevel_width(&Input {
                mouse: self.pointer,
                snap: self.snapping,
                steps: self.settings.snap,
                camera: self.camera,
                width,
                height,
            })
        });
        // A bevel let go without being dragged has no width. Kept, it would leave points
        // on top of each other that later bevels cannot work with, so it is dropped, as
        // Blender does.
        if bevel_width.is_some_and(|w| w < MIN_BEVEL_WIDTH) {
            confirm = false;
            self.state.message =
                "No bevel: move the mouse away from the center, then click.".into();
        }
        let bevel_width = bevel_width.filter(|_| confirm);
        let keep = !confirm && transform.keep_on_cancel && transform.kind != Kind::Bevel;
        if keep && let Some(mesh) = self.editor.active_mesh() {
            for &(id, position) in &transform.originals {
                let _ = mesh.set_vertex_position(id, position);
            }
        }
        let result = if confirm || keep {
            self.editor.commit().map(|_| ())
        } else {
            self.editor.cancel()
        };
        let worked = result.is_ok();
        self.finish_edit(result);
        // The bevel panel can change this bevel, until something else changes.
        if let (Some(width), true) = (bevel_width, worked) {
            // The panel shows the width as its type measures it.
            self.state.bevel.width = width / self.bevel_per_unit.max(1e-9);
            self.last_bevel = Some(self.editor.document().version());
            self.bevel_removed = false;
        }
        self.request_frames();
    }

    /// Redoes the last bevel with the panel's settings, as Blender's panel does.
    fn adjust_bevel(&mut self) {
        if self.last_bevel != Some(self.editor.document().version())
            || !matches!(self.mouse_state, Mouse::Idle)
        {
            return;
        }
        let settings = self.state.bevel;
        // While the width is 0 the bevel is taken away (it would only leave points on
        // top of each other), so there is nothing to undo first.
        if !self.bevel_removed && !self.editor.undo() {
            return;
        }
        if settings.width <= 0.0 {
            self.bevel_removed = true;
            self.last_bevel = Some(self.editor.document().version());
            self.stale = Stale::Everything;
            self.request_frames();
            return;
        }
        let was_removed = std::mem::take(&mut self.bevel_removed);
        let mut made_width = None;
        let result = self.editor.run("Bevel", |mesh, mode| {
            mesh.bevel_selected(&settings, mode)
                .map(|made| made_width = Some((made.width, made.per_unit)))
        });
        match result {
            Ok(()) => {
                // Shows the width actually used, if the clamp held it back.
                if let Some((width, per_unit)) = made_width
                    && width < settings.width * per_unit - 1e-6
                {
                    self.state.bevel.width = width / per_unit.max(1e-9);
                }
                self.last_bevel = Some(self.editor.document().version());
            }
            Err(error) => {
                // Puts the previous bevel back.
                if was_removed {
                    self.bevel_removed = true;
                } else {
                    self.editor.redo();
                }
                self.last_bevel = Some(self.editor.document().version());
                self.state.message = format!("Could not bevel: {error}");
            }
        }
        self.stale = Stale::Everything;
        self.request_frames();
    }

    /// Sets a modifier to the settings in the panel. Changes to the same modifier one
    /// after another, as while dragging a slider, replace each other in the undo list.
    fn change_modifier(&mut self) {
        let Some((id, modifier)) = self.state.modifier_edit.take() else {
            return;
        };
        if !matches!(self.mouse_state, Mouse::Idle) {
            return;
        }
        let version = self.editor.document().version();
        let undone = self.last_modifier_edit == Some((version, id)) && self.editor.undo();
        let result = self.editor.run("Modifier settings", |mesh, _| {
            mesh.set_modifier(id, modifier)
        });
        match result {
            Ok(()) => {
                self.last_modifier_edit = Some((self.editor.document().version(), id));
            }
            Err(error) => {
                // Puts the previous change back.
                if undone {
                    self.editor.redo();
                }
                self.last_modifier_edit = undone.then(|| (self.editor.document().version(), id));
                self.state.message = format!("Could not change the modifier: {error}");
            }
        }
        self.stale = Stale::Everything;
        self.request_frames();
    }

    /// The handles on the selection, if a handle tool is on and nothing else is going on.
    fn gizmo_parts(&self) -> Vec<gizmo::Part> {
        let (Some(kind), Mouse::Idle) = (self.gizmo_tool, &self.mouse_state) else {
            return Vec::new();
        };
        if self.view_only {
            return Vec::new();
        }
        let Some(pivot) = self.editor.document().mesh().selection_center() else {
            return Vec::new();
        };
        let (width, height) = self.viewport();
        gizmo::parts(
            kind,
            pivot,
            &gizmo::Screen {
                camera: self.camera,
                width,
                height,
            },
        )
    }

    /// The current view, with its depth read back from the graphics card.
    fn sight(&self) -> Sight {
        let (width, height) = self.viewport();
        let matrix = self.camera.view_projection(width / height);
        let depth = self.gpu.read_depth(width as i32, height as i32, &matrix);
        Sight {
            view: ViewInfo {
                matrix,
                width,
                height,
            },
            image: DepthImage {
                width: width as i32,
                height: height as i32,
                depth,
            },
            toward: self.camera.toward_camera(),
            tolerance: self.camera.distance * DEPTH_TOLERANCE,
        }
    }

    /// Runs `pick` with a check for whether a point is hidden by the scene.
    fn with_visibility<T>(
        &self,
        pick: impl FnOnce(&Mesh, SelectMode, &ViewInfo, &dyn Fn([f32; 3]) -> bool) -> T,
    ) -> T {
        let sight = self.sight();
        pick(
            self.editor.document().mesh(),
            self.editor.mode(),
            &sight.view,
            &|p| sight.visible(p),
        )
    }

    /// Starts choosing where to loop cut. The cut happens on a click.
    fn start_loop_cut(&mut self) {
        if !matches!(self.mouse_state, Mouse::Idle) {
            return;
        }
        if self.editor.document().mesh().edge_count() == 0 {
            self.state.message = "Add a shape first.".to_string();
            return;
        }
        self.mouse_state = Mouse::LoopCut(Box::new(LoopCutChoice {
            cuts: self.loop_cuts,
            edge: None,
            lines: Vec::new(),
            sight: self.sight(),
        }));
        self.update_loop_cut();
        self.request_frames();
    }

    /// Finds the edge under the pointer and where the cut would go.
    fn update_loop_cut(&mut self) {
        let [x, y] = self.pointer;
        let mesh = self.editor.document().mesh();
        let Mouse::LoopCut(choice) = &mut self.mouse_state else {
            return;
        };
        let sight = &choice.sight;
        choice.edge = pick_edge(mesh, &sight.view, x, y, LOOP_CUT_REACH, &|p| {
            sight.visible(p)
        });
        choice.lines = choice
            .edge
            .and_then(|edge| mesh.loop_cut_preview(edge, choice.cuts).ok())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|[a, b]| {
                let (a, b) = (sight.view.project(a)?, sight.view.project(b)?);
                Some([[a[0], a[1]], [b[0], b[1]]])
            })
            .collect();
        self.frames_wanted = self.frames_wanted.max(1);
    }

    fn change_cuts(&mut self, by: i32) {
        if let Mouse::LoopCut(choice) = &mut self.mouse_state {
            let cuts = (choice.cuts as i32 + by).clamp(1, MAX_LOOP_CUTS as i32) as usize;
            choice.cuts = cuts;
            self.loop_cuts = cuts;
        }
        self.update_loop_cut();
    }

    fn handle_loop_cut_event(&mut self, event: &Event) {
        self.request_frames();
        match *event {
            Event::MouseMotion { .. } => self.update_loop_cut(),
            Event::MouseButtonDown {
                mouse_btn: MouseButton::Left,
                ..
            } => {
                let Mouse::LoopCut(choice) = &self.mouse_state else {
                    return;
                };
                match (choice.edge, choice.lines.is_empty()) {
                    (Some(edge), false) => {
                        let cuts = choice.cuts;
                        self.mouse_state = Mouse::Idle;
                        self.start_transform(Kind::Slide, Prepare::LoopCut(edge, cuts));
                    }
                    (Some(_), true) => {
                        self.state.message =
                            "A loop cut needs a four-sided face next to the edge.".to_string();
                    }
                    (None, _) => self.state.message = "Point at an edge to cut.".to_string(),
                }
            }
            Event::MouseButtonDown {
                mouse_btn: MouseButton::Right,
                ..
            }
            | Event::KeyDown {
                keycode: Some(Keycode::Escape),
                ..
            } => self.mouse_state = Mouse::Idle,
            Event::MouseWheel { y, direction, .. } if y != 0 => {
                let steps = if direction == MouseWheelDirection::Flipped {
                    -y
                } else {
                    y
                };
                self.change_cuts(steps.signum());
            }
            Event::KeyDown {
                keycode: Some(key), ..
            } => match key {
                Keycode::Plus | Keycode::KP_PLUS | Keycode::Equals | Keycode::Up => {
                    self.change_cuts(1);
                }
                Keycode::Minus | Keycode::KP_MINUS | Keycode::Down => self.change_cuts(-1),
                _ => {}
            },
            _ => {}
        }
    }

    /// Union, subtract or intersect of the selected parts.
    fn boolean(&mut self, op: BooleanOp) {
        let cutter = self.last_clicked;
        let mut faces = 0;
        let result = self.editor.run("Combine", |mesh, mode| {
            faces = mesh.boolean_selected(op, cutter, mode)?;
            Ok(())
        });
        if result.is_ok() {
            let name = match op {
                BooleanOp::Union => "Union",
                BooleanOp::Subtract => "Subtract",
                BooleanOp::Intersect => "Intersect",
            };
            self.state.message = format!("{name} done: {faces} faces.");
        }
        self.finish_edit(result);
    }

    /// Starts drawing a knife path.
    fn start_knife(&mut self) {
        if !matches!(self.mouse_state, Mouse::Idle) {
            return;
        }
        if self.editor.document().mesh().face_count() == 0 {
            self.state.message = "Add a shape first.".to_string();
            return;
        }
        self.mouse_state = Mouse::Knife(Box::new(KnifeChoice {
            clicks: Vec::new(),
            through: false,
            cut: KnifeCut::default(),
            preview: KnifeCut::default(),
            sight: self.sight(),
        }));
        self.update_knife();
        self.request_frames();
    }

    /// Works out the cut again after the clicks or the pointer moved.
    fn update_knife(&mut self) {
        let pointer = self.pointer;
        let mesh = self.editor.document().mesh();
        let Mouse::Knife(choice) = &mut self.mouse_state else {
            return;
        };
        let sight = &choice.sight;
        let through = choice.through;
        let visible = |p: [f32; 3]| through || sight.visible(p);
        choice.cut = knife_cut(mesh, &sight.view, &choice.clicks, &visible);
        let mut with_pointer = choice.clicks.clone();
        with_pointer.push(pointer);
        choice.preview = knife_cut(mesh, &sight.view, &with_pointer, &visible);
        self.frames_wanted = self.frames_wanted.max(1);
    }

    fn finish_knife(&mut self) {
        let Mouse::Knife(choice) = std::mem::replace(&mut self.mouse_state, Mouse::Idle) else {
            return;
        };
        if choice.cut.points.len() < 2 {
            self.state.message =
                "Click across a face, from one edge to another, then press Enter.".to_string();
            return;
        }
        let mut edges = 0;
        let result = self.editor.run("Knife", |mesh, mode| {
            edges = mesh.knife(&choice.cut.points, mode)?;
            Ok(())
        });
        if result.is_ok() {
            self.state.message = format!("Cut {edges} faces.");
        }
        self.finish_edit(result);
    }

    fn handle_knife_event(&mut self, event: &Event) {
        self.request_frames();
        match *event {
            Event::MouseMotion { .. } => self.update_knife(),
            Event::MouseButtonDown {
                mouse_btn: MouseButton::Left,
                clicks,
                ..
            } => {
                let pointer = self.pointer;
                if let Mouse::Knife(choice) = &mut self.mouse_state {
                    // A click on a corner goes exactly through it.
                    let mesh = self.editor.document().mesh();
                    let sight = &choice.sight;
                    let through = choice.through;
                    let at = snapped_vertex(mesh, &sight.view, pointer, &|p| {
                        through || sight.visible(p)
                    })
                    .map_or(pointer, |(_, at)| at);
                    if clicks < 2 || choice.clicks.is_empty() {
                        choice.clicks.push(at);
                    }
                }
                self.update_knife();
                // A double click ends the path.
                if clicks >= 2 {
                    self.finish_knife();
                }
            }
            Event::MouseButtonDown {
                mouse_btn: MouseButton::Right,
                ..
            }
            | Event::KeyDown {
                keycode: Some(Keycode::Escape),
                ..
            } => self.mouse_state = Mouse::Idle,
            Event::KeyDown {
                keycode: Some(key), ..
            } => match key {
                Keycode::Return | Keycode::KpEnter | Keycode::Space => self.finish_knife(),
                Keycode::Backspace => {
                    if let Mouse::Knife(choice) = &mut self.mouse_state {
                        choice.clicks.pop();
                    }
                    self.update_knife();
                }
                Keycode::Z => {
                    if let Mouse::Knife(choice) = &mut self.mouse_state {
                        choice.through = !choice.through;
                    }
                    self.update_knife();
                }
                _ => {}
            },
            _ => {}
        }
    }

    /// Starts dragging a line to bisect the selected faces along.
    fn start_bisect(&mut self) {
        if !matches!(self.mouse_state, Mouse::Idle) {
            return;
        }
        let mesh = self.editor.document().mesh();
        if !mesh.faces().any(|(_, f)| f.is_selected()) {
            self.state.message = "Select the faces to cut first.".to_string();
            return;
        }
        self.mouse_state = Mouse::Bisect(Box::new(BisectChoice {
            start: None,
            end: self.pointer,
        }));
        self.request_frames();
    }

    fn handle_bisect_event(&mut self, event: &Event) {
        self.request_frames();
        let pointer = self.pointer;
        match *event {
            Event::MouseMotion { .. } => {
                if let Mouse::Bisect(choice) = &mut self.mouse_state {
                    choice.end = pointer;
                }
            }
            Event::MouseButtonDown {
                mouse_btn: MouseButton::Left,
                ..
            } => {
                if let Mouse::Bisect(choice) = &mut self.mouse_state {
                    choice.start = Some(pointer);
                    choice.end = pointer;
                }
            }
            Event::MouseButtonUp {
                mouse_btn: MouseButton::Left,
                ..
            } => {
                let Mouse::Bisect(choice) = std::mem::replace(&mut self.mouse_state, Mouse::Idle)
                else {
                    return;
                };
                let Some(start) = choice.start else {
                    return;
                };
                let end = pointer;
                if (end[0] - start[0]).hypot(end[1] - start[1]) < 6.0 {
                    self.state.message =
                        "Drag a line across the shape to cut along it.".to_string();
                    return;
                }
                self.bisect_along(start, end);
            }
            Event::MouseButtonDown {
                mouse_btn: MouseButton::Right,
                ..
            }
            | Event::KeyDown {
                keycode: Some(Keycode::Escape),
                ..
            } => self.mouse_state = Mouse::Idle,
            _ => {}
        }
    }

    /// Cuts the selected faces with the plane that looks like this line on screen.
    fn bisect_along(&mut self, start: [f32; 2], end: [f32; 2]) {
        let (width, height) = self.viewport();
        let (o0, d0) = self.camera.ray(start[0], start[1], width, height);
        let (o1, d1) = self.camera.ray(end[0], end[1], width, height);
        let b: [f32; 3] = std::array::from_fn(|i| d0[i]);
        let c: [f32; 3] = std::array::from_fn(|i| o1[i] + d1[i] - o0[i]);
        let normal = [
            b[1] * c[2] - b[2] * c[1],
            b[2] * c[0] - b[0] * c[2],
            b[0] * c[1] - b[1] * c[0],
        ];
        let mut edges = 0;
        let result = self.editor.run("Bisect", |mesh, mode| {
            edges = mesh.bisect_selected(o0, normal, mode)?;
            Ok(())
        });
        if result.is_ok() {
            self.state.message = format!("Cut {edges} faces.");
        }
        self.finish_edit(result);
    }

    fn dragging_handle(&self) -> bool {
        matches!(&self.mouse_state, Mouse::Transforming { transform, .. }
            if transform.release_confirms && !transform.waiting)
    }

    fn bevel_waiting(&self) -> bool {
        matches!(&self.mouse_state, Mouse::Transforming { transform, .. } if transform.waiting)
    }

    fn bevelling(&self) -> bool {
        matches!(&self.mouse_state, Mouse::Transforming { transform, .. } if transform.kind == Kind::Bevel)
    }

    /// Redoes the bevel with more or fewer segments, keeping the width the mouse gives.
    fn change_bevel_segments(&mut self, by: i32, snap: bool) {
        let Mouse::Transforming { transform, .. } = &self.mouse_state else {
            return;
        };
        let (start, waiting) = (transform.start_mouse, transform.waiting);
        let segments =
            (self.state.bevel.segments as i32 + by).clamp(1, MAX_BEVEL_SEGMENTS as i32) as usize;
        if segments == self.state.bevel.segments {
            return;
        }
        self.state.bevel.segments = segments;
        self.finish_transform(false);
        self.start_transform(Kind::Bevel, Prepare::Bevel(Box::new(self.state.bevel)));
        if let Mouse::Transforming { transform, .. } = &mut self.mouse_state {
            transform.start_mouse = start;
            transform.waiting = waiting;
        }
        self.update_transform(snap);
        self.request_frames();
    }

    /// Selects the loop through the edge under the pointer, for a double-click.
    fn select_loop_under_pointer(&mut self, add: bool) {
        let [x, y] = self.pointer;
        let hit = self.with_visibility(|mesh, _, view, visible| {
            pick_edge(mesh, view, x, y, LOOP_SELECT_REACH, visible)
        });
        let Some(edge) = hit else {
            return;
        };
        let op = if add {
            SelectOp::Add
        } else {
            SelectOp::Replace
        };
        let result = self
            .editor
            .run("Select loop", |mesh, mode| mesh.select_loop(mode, edge, op));
        self.finish_edit(result);
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
            if let Some(Element::Face(face)) = hit
                && op != SelectOp::Subtract
            {
                self.last_clicked = Some(face);
            }
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
        // The settings and start windows take everything while they show.
        if self.settings_window.is_some() {
            if let Event::KeyDown {
                keycode: Some(Keycode::Escape),
                ..
            } = event
            {
                self.settings_window = None;
            } else {
                self.feed_imgui(event);
            }
            self.request_frames();
            return;
        }
        if self.start.is_some() {
            self.feed_imgui(event);
            self.request_frames();
            return;
        }
        let snap = modifiers(mods).ctrl;
        self.snapping = snap;

        // A transform takes all input until it ends, so nothing behind it can change.
        if matches!(self.mouse_state, Mouse::Transforming { .. }) {
            self.imgui_pointer_only(event);
            self.handle_transform_event(event, snap);
            return;
        }
        if matches!(self.mouse_state, Mouse::LoopCut(_)) {
            self.imgui_pointer_only(event);
            self.handle_loop_cut_event(event);
            return;
        }
        if matches!(self.mouse_state, Mouse::Knife(_)) {
            self.imgui_pointer_only(event);
            self.handle_knife_event(event);
            return;
        }
        if matches!(self.mouse_state, Mouse::Bisect(_)) {
            self.imgui_pointer_only(event);
            self.handle_bisect_event(event);
            return;
        }

        self.feed_imgui(event);
        let io = self.imgui.io();
        let (ui_mouse, ui_keyboard) = (io.want_capture_mouse, io.want_capture_keyboard);

        match *event {
            Event::MouseButtonDown {
                mouse_btn, clicks, ..
            } => {
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
                // A handle under the mouse takes the click.
                let handle = if button == Button::Left {
                    gizmo::hit(&self.gizmo_parts(), self.pointer)
                } else {
                    None
                };
                if let (Some(handle), Some(kind)) = (handle, self.gizmo_tool) {
                    self.start_transform(kind, Prepare::Nothing);
                    if let Mouse::Transforming { transform, .. } = &mut self.mouse_state {
                        transform.axis = handle.axis();
                        transform.release_confirms = true;
                    }
                    self.gizmo_hover = None;
                    self.update_transform(mods.ctrl);
                    return;
                }
                if let Some(drag) = drag_for(button, mods) {
                    self.mouse_state = Mouse::Camera(drag);
                    // Keeps the drag going if the mouse leaves the window.
                    mouse.capture(true);
                } else if button == Button::Left && clicks >= 2 && !self.editor.whole_parts() {
                    // The first click already selected; the second widens it to a loop.
                    self.select_loop_under_pointer(mods.shift);
                } else if button == Button::Left && !self.view_only {
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
                    Mouse::Camera(Drag::Orbit) => {
                        let speed = self.settings.turn_speed as f32 / 100.0;
                        self.camera.orbit_pixels(dx * speed, dy * speed);
                    }
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
                // Lights up the handle under the mouse.
                let hover = gizmo::hit(&self.gizmo_parts(), self.pointer);
                if hover != self.gizmo_hover {
                    self.gizmo_hover = hover;
                    self.request_frames();
                }
                // Moving over empty viewport needs no drawing. The panel highlights
                // its buttons, so it does, as does leaving it or using a slider.
                let s = self.ui_scale;
                let over = in_rects(&self.ui_rects, x as f32 / s, y as f32 / s);
                if over || self.pointer_over_panel || ui_mouse {
                    self.request_frames();
                }
                self.pointer_over_panel = over;
            }
            Event::MouseWheel {
                precise_x,
                precise_y,
                direction,
                ..
            } => {
                self.request_frames();
                if !ui_mouse && !self.pointer_over_panel {
                    let flip = if direction == MouseWheelDirection::Flipped {
                        -1.0
                    } else {
                        1.0
                    };
                    self.wheel_view(precise_x * flip, precise_y * flip, modifiers(mods));
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
            Event::DropFile { ref filename, .. } => {
                self.request(Pending::Open(Some(PathBuf::from(filename))));
            }
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
            // A handle drag ends when the button comes up.
            Event::MouseButtonUp {
                mouse_btn: MouseButton::Left,
                ..
            } if self.dragging_handle() => self.finish_transform(true),
            // A waiting bevel starts following the mouse from where the button went down.
            Event::MouseButtonDown {
                mouse_btn: MouseButton::Left,
                ..
            } if self.bevel_waiting() => {
                let pointer = self.pointer;
                if let Mouse::Transforming { transform, .. } = &mut self.mouse_state {
                    transform.waiting = false;
                    transform.start_mouse = pointer;
                }
                self.request_frames();
            }
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
                if self.bevelling() {
                    self.change_bevel_segments(steps.signum(), snap);
                } else {
                    self.camera.zoom_steps(steps as f32);
                    self.update_transform(snap);
                }
            }
            Event::KeyDown {
                keycode: Some(key), ..
            } => match key {
                Keycode::Escape => self.finish_transform(false),
                // A bevel not dragged yet has no width: Enter leaves it undone.
                Keycode::Return | Keycode::KpEnter | Keycode::Space => {
                    let confirm = !self.bevel_waiting();
                    self.finish_transform(confirm);
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
                Keycode::Plus | Keycode::KP_PLUS | Keycode::Equals if self.bevelling() => {
                    self.change_bevel_segments(1, snap);
                }
                Keycode::Minus | Keycode::KP_MINUS if self.bevelling() => {
                    self.change_bevel_segments(-1, snap);
                }
                // G again while moving slides instead, as in Blender.
                Keycode::G => {
                    let plain_move = matches!(&self.mouse_state,
                        Mouse::Transforming { transform, .. }
                            if transform.kind == Kind::Move && transform.plain);
                    if plain_move {
                        self.finish_transform(false);
                        self.start_transform(Kind::Slide, Prepare::Nothing);
                    }
                }
                _ => {}
            },
            Event::KeyUp {
                keycode: Some(Keycode::LCtrl | Keycode::RCtrl),
                ..
            } => self.update_transform(false),
            _ => {}
        }
    }

    /// While a drag or loop cut takes the input, the interface still follows the
    /// pointer, and hears of button releases so that no button stays held down.
    fn imgui_pointer_only(&mut self, event: &Event) {
        match event {
            // Releases always go through, or Dear ImGui would think Ctrl was still held
            // after Ctrl+B, and the next click on a slider would start typing in it.
            Event::MouseMotion { .. } | Event::MouseButtonUp { .. } | Event::KeyUp { .. } => {
                self.feed_imgui(event)
            }
            _ => {}
        }
    }

    fn feed_imgui(&mut self, event: &Event) {
        let io = self.imgui.io_mut();
        match event {
            Event::MouseMotion { x, y, .. } => {
                let s = self.ui_scale;
                io.add_mouse_pos_event([*x as f32 / s, *y as f32 / s]);
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

fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .map_or("Untitled".to_string(), |n| n.to_string_lossy().into_owned())
}

fn file_stem(path: &std::path::Path) -> String {
    path.file_stem()
        .map_or("model".to_string(), |n| n.to_string_lossy().into_owned())
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
            Keycode::R => Some(Action::LoopCut),
            Keycode::J => Some(Action::Join),
            Keycode::Comma => Some(Action::Settings),
            Keycode::F => Some(Action::Search),
            Keycode::B if mods.shift => Some(Action::BevelVertices),
            Keycode::B => Some(Action::Bevel),
            Keycode::S if mods.shift => Some(Action::SaveAs),
            Keycode::S => Some(Action::Save),
            Keycode::N => Some(Action::New),
            Keycode::O => Some(Action::Open),
            Keycode::E => Some(Action::Export),
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
        Keycode::K => Some(Action::Knife),
        Keycode::P => Some(Action::Separate),
        Keycode::Tab => Some(Action::ToggleView),
        _ => None,
    }
}

/// True for actions that change the shape or what is selected, which View mode hides.
fn changes_the_shape(action: Action) -> bool {
    !matches!(
        action,
        Action::Undo
            | Action::Redo
            | Action::Export
            | Action::View(_)
            | Action::ToggleOrtho
            | Action::ResetView
            | Action::ToggleView
            | Action::Settings
            | Action::New
            | Action::Open
            | Action::Save
            | Action::SaveAs
            | Action::PathChosen
            | Action::PathCancelled
            | Action::ConfirmSave
            | Action::ConfirmDiscard
            | Action::ConfirmCancel
    ) && !is_export_folder(action)
}

#[cfg(windows)]
fn is_export_folder(action: Action) -> bool {
    action == Action::OpenExportFolder
}

#[cfg(not(windows))]
fn is_export_folder(_: Action) -> bool {
    false
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
        assert_eq!(shortcut(Keycode::R, CTRL), Some(Action::LoopCut));
        assert_eq!(shortcut(Keycode::B, CTRL), Some(Action::Bevel));
        let ctrl_shift = Modifiers {
            shift: true,
            ..CTRL
        };
        assert_eq!(
            shortcut(Keycode::B, ctrl_shift),
            Some(Action::BevelVertices)
        );
        let ctrl_shift = Modifiers {
            shift: true,
            ..CTRL
        };
        assert_eq!(shortcut(Keycode::S, CTRL), Some(Action::Save));
        assert_eq!(shortcut(Keycode::S, ctrl_shift), Some(Action::SaveAs));
        assert_eq!(shortcut(Keycode::N, CTRL), Some(Action::New));
        assert_eq!(shortcut(Keycode::O, CTRL), Some(Action::Open));
        assert_eq!(shortcut(Keycode::E, CTRL), Some(Action::Export));
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
