//! The newer rendering engines: Vulkan, DirectX 12 and OpenGL 3.3, all through wgpu
//! (Stage 10 step 4). Draws the same picture as `renderer.rs` and `imgui_gl.rs`, which
//! stay as the OpenGL 2.1 engine for older computers such as the owner's laptop.

use std::sync::{Arc, Mutex};

use imgui::{DrawCmd, DrawVert};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use wgpu::util::DeviceExt;

use crate::camera::Mat4;
use crate::grid::{FLOATS_PER_VERTEX, GridPlane, grid_vertices};
use crate::palette;
use crate::renderer::{EDGE_GLOW, View, background_corners};
use crate::scene::{Changed, FLOATS_PER_LINE_VERTEX, FLOATS_PER_TRIANGLE_VERTEX, Scene};

/// The engines wgpu offers here, by their key in settings.txt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Vulkan,
    DirectX12,
    OpenGl33,
}

impl Engine {
    pub const ALL: [Engine; 3] = [Engine::Vulkan, Engine::DirectX12, Engine::OpenGl33];

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|e| e.key() == key)
    }

    pub fn key(self) -> &'static str {
        match self {
            Engine::Vulkan => "vulkan",
            Engine::DirectX12 => "directx12",
            Engine::OpenGl33 => "opengl3.3",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Engine::Vulkan => "Vulkan",
            Engine::DirectX12 => "DirectX 12",
            Engine::OpenGl33 => "OpenGL 3.3",
        }
    }

    fn backends(self) -> wgpu::Backends {
        match self {
            Engine::Vulkan => wgpu::Backends::VULKAN,
            Engine::DirectX12 => wgpu::Backends::DX12,
            Engine::OpenGl33 => wgpu::Backends::GL,
        }
    }
}

/// How well an engine works on this computer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Support {
    /// A graphics card offers it.
    Yes,
    /// Only a software stand-in offers it: it works, but slowly.
    Software,
    No,
}

impl Support {
    pub fn word(self) -> &'static str {
        match self {
            Support::Yes => "yes",
            Support::Software => "software",
            Support::No => "no",
        }
    }

    pub fn from_word(word: &str) -> Option<Self> {
        [Support::Yes, Support::Software, Support::No]
            .into_iter()
            .find(|s| s.word() == word)
    }
}

/// Asks the graphics drivers which engines they offer. Can take a moment, and a bad
/// driver could even crash, so Mixer runs this in a separate process
/// (`mixer --check-engines`).
pub fn check(engine: Engine) -> (Support, String) {
    let instance = wgpu::Instance::new(instance_descriptor(engine, None));
    let adapters = pollster::block_on(instance.enumerate_adapters(engine.backends()));
    let mut best = (Support::No, String::new());
    for adapter in adapters {
        let info = adapter.get_info();
        if engine == Engine::OpenGl33 && !opengl_is_new_enough(&adapter) {
            continue;
        }
        let support = if info.device_type == wgpu::DeviceType::Cpu {
            Support::Software
        } else {
            Support::Yes
        };
        if best.0 != Support::Yes {
            best = (support, info.name);
        }
    }
    best
}

/// wgpu's OpenGL path needs OpenGL 3.3, but will start on some older drivers too, which
/// then draw wrongly. Their version is in the driver text.
fn opengl_is_new_enough(adapter: &wgpu::Adapter) -> bool {
    let info = adapter.get_info();
    opengl_text_is_new_enough(&format!("{} {}", info.driver, info.driver_info))
}

/// Desktop OpenGL must be 3.3; OpenGL ES (used on Linux) 3.0, which is the same level.
fn opengl_text_is_new_enough(text: &str) -> bool {
    let needed = if text.contains("OpenGL ES") {
        (3, 0)
    } else {
        (3, 3)
    };
    match text
        .split_whitespace()
        .find_map(crate::renderer::parse_version)
    {
        Some(version) => version >= needed,
        None => true,
    }
}

/// wgpu needs the window system's connection for OpenGL; this hands it over.
#[derive(Debug)]
struct DisplayLink(raw_window_handle::RawDisplayHandle);

// SAFETY: the handle is the window system's connection, which SDL keeps open for as long
// as Mixer runs, and which may be used from any thread.
unsafe impl Send for DisplayLink {}
unsafe impl Sync for DisplayLink {}

impl HasDisplayHandle for DisplayLink {
    fn display_handle(
        &self,
    ) -> Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
        // SAFETY: see above; the connection outlives every use.
        Ok(unsafe { raw_window_handle::DisplayHandle::borrow_raw(self.0) })
    }
}

fn instance_descriptor(
    engine: Engine,
    display: Option<raw_window_handle::RawDisplayHandle>,
) -> wgpu::InstanceDescriptor {
    let mut descriptor = match display {
        Some(display) => {
            wgpu::InstanceDescriptor::new_with_display_handle(Box::new(DisplayLink(display)))
        }
        None => wgpu::InstanceDescriptor::new_without_display_handle(),
    };
    descriptor.backends = engine.backends();
    descriptor
}

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24Plus;
const PICK_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
/// Each draw's settings sit in their own slot of one buffer.
const SLOT: u64 = 256;
/// The shader's `Uniforms`: a matrix and eight groups of four numbers.
const UNIFORM_SIZE: u64 = 192;
const SLOTS: u64 = 9;
const BACKGROUND_SLOT: u64 = 0;
const GRID_SLOT: u64 = 1;
const MESH_SLOT: u64 = 2;
const EDGE_SLOT: u64 = 3;
const GLOW_SLOT: u64 = 4;
const PLAIN_EDGE_SLOT: u64 = 5;
const POINT_SLOT: u64 = 6;
const PICK_SLOT: u64 = 7;
const CORE_SLOT: u64 = 8;
/// Grid lines are pushed back slightly, as in renderer.rs.
const GRID_DEPTH_BIAS: f32 = 0.0002;

/// One slot's values, laid out as the shader's `Uniforms`.
#[derive(Clone, Copy, Default)]
struct Uniforms {
    matrix: Mat4,
    color: [f32; 4],
    fade: [f32; 4],
    light: [f32; 4],
    extra: [f32; 4],
    glow: [f32; 4],
    face: [f32; 4],
    back: [f32; 4],
    selected_face: [f32; 4],
}

impl Uniforms {
    fn bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(SLOT as usize);
        for value in self
            .matrix
            .iter()
            .chain(&self.color)
            .chain(&self.fade)
            .chain(&self.light)
            .chain(&self.extra)
            .chain(&self.glow)
            .chain(&self.face)
            .chain(&self.back)
            .chain(&self.selected_face)
        {
            out.extend_from_slice(&value.to_le_bytes());
        }
        out.resize(SLOT as usize, 0);
        out
    }
}

const IDENTITY: Mat4 = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

fn as_bytes(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// A vertex buffer that grows when the scene does.
struct Growing {
    buffer: wgpu::Buffer,
    /// Vertices in it now.
    count: u32,
}

impl Growing {
    fn new(device: &wgpu::Device, label: &str) -> Self {
        Self {
            buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: 1024,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            count: 0,
        }
    }

    fn replace(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, data: &[f32], per: usize) {
        let bytes = as_bytes(data);
        if bytes.len() as u64 > self.buffer.size() {
            let size = (bytes.len() as u64).next_power_of_two().max(1024);
            self.buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        if !bytes.is_empty() {
            queue.write_buffer(&self.buffer, 0, &bytes);
        }
        self.count = (data.len() / per) as u32;
    }

    fn update(&self, queue: &wgpu::Queue, data: &[f32], ranges: &[std::ops::Range<usize>]) {
        for range in ranges {
            if range.end * 4 <= self.buffer.size() as usize && !range.is_empty() {
                queue.write_buffer(
                    &self.buffer,
                    (range.start * 4) as u64,
                    &as_bytes(&data[range.clone()]),
                );
            }
        }
    }
}

/// Textures that follow the window's size.
struct Targets {
    size: (u32, u32),
    /// With anti-aliasing: the picture is drawn here, then smoothed into the window.
    multisampled: Option<wgpu::TextureView>,
    depth: wgpu::TextureView,
}

/// Picking's depth image and where it is read back.
struct PickTargets {
    size: (u32, u32),
    color: wgpu::Texture,
    color_view: wgpu::TextureView,
    depth: wgpu::TextureView,
    readback: wgpu::Buffer,
    row_bytes: u32,
}

pub struct WgpuRenderer {
    engine: Engine,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    /// The format drawn in, the window's own without an sRGB conversion.
    view_format: wgpu::TextureFormat,
    samples: u32,
    uniforms: wgpu::Buffer,
    uniform_group: wgpu::BindGroup,
    background_pipeline: wgpu::RenderPipeline,
    grid_pipeline: wgpu::RenderPipeline,
    mesh_pipeline: wgpu::RenderPipeline,
    line_pipeline: wgpu::RenderPipeline,
    glow_pipeline: wgpu::RenderPipeline,
    point_pipeline: wgpu::RenderPipeline,
    pick_pipeline: wgpu::RenderPipeline,
    ui_pipeline: wgpu::RenderPipeline,
    background: wgpu::Buffer,
    grid: wgpu::Buffer,
    /// The theme the grid and background were made with.
    theme_version: u64,
    grid_vertex_count: u32,
    triangles: Growing,
    edges: Growing,
    points: Growing,
    ui_uniforms: wgpu::Buffer,
    ui_uniform_group: wgpu::BindGroup,
    ui_texture_layout: wgpu::BindGroupLayout,
    ui_sampler: wgpu::Sampler,
    ui_texture_group: Option<wgpu::BindGroup>,
    ui_vertices: wgpu::Buffer,
    ui_indices: wgpu::Buffer,
    targets: Option<Targets>,
    pick: std::cell::RefCell<Option<PickTargets>>,
    /// The frame drawn but not yet shown.
    pending: Option<wgpu::SurfaceTexture>,
    /// Errors the graphics driver reported, for the log.
    errors: Arc<Mutex<Vec<String>>>,
}

/// What the new engine reports, for the log and the title bar.
pub struct WgpuInfo {
    pub adapter: String,
    pub driver: String,
    pub software: bool,
}

impl WgpuRenderer {
    /// Starts `engine` for `window`. Any failure is returned as words for the owner, so
    /// that Mixer can fall back to OpenGL 2.1.
    pub fn new(
        window: &sdl2::video::Window,
        engine: Engine,
        antialiasing: bool,
    ) -> Result<(Self, WgpuInfo), String> {
        let display = window
            .display_handle()
            .map_err(|e| format!("no window system handle: {e}"))?
            .as_raw();
        let window_handle = window
            .window_handle()
            .map_err(|e| format!("no window handle: {e}"))?
            .as_raw();
        let instance = wgpu::Instance::new(instance_descriptor(engine, Some(display)));
        // SAFETY: the window outlives the renderer: main drops the app, which owns this,
        // before the window.
        let surface = unsafe {
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: Some(display),
                raw_window_handle: window_handle,
            })
        }
        .map_err(|e| format!("could not draw into the window: {e}"))?;

        let mut adapter = None;
        for fallback in [false, true] {
            if let Ok(found) =
                pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    force_fallback_adapter: fallback,
                    compatible_surface: Some(&surface),
                    apply_limit_buckets: false,
                }))
            {
                adapter = Some(found);
                break;
            }
        }
        let adapter = adapter.ok_or("no graphics card or driver offers it")?;
        if engine == Engine::OpenGl33 && !opengl_is_new_enough(&adapter) {
            let info = adapter.get_info();
            return Err(format!(
                "the graphics driver offers an OpenGL older than 3.3 ({} {})",
                info.driver, info.driver_info
            ));
        }
        let info = adapter.get_info();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("Mixer"),
            required_features: wgpu::Features::empty(),
            required_limits:
                wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .map_err(|e| format!("the graphics driver refused: {e}"))?;
        let errors = Arc::new(Mutex::new(Vec::new()));
        {
            let errors = errors.clone();
            device.on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
                if let Ok(mut list) = errors.lock()
                    && list.len() < 20
                {
                    list.push(error.to_string());
                }
            }));
        }

        let (width, height) = window.drawable_size();
        let mut config = surface
            .get_default_config(&adapter, width.max(1), height.max(1))
            .ok_or("the window cannot be drawn into with it")?;
        let capabilities = surface.get_capabilities(&adapter);
        // Mixer's colors are worked out without sRGB conversion, as in OpenGL 2.1.
        if let Some(&plain) = capabilities.formats.iter().find(|f| !f.is_srgb()) {
            config.format = plain;
        }
        let view_format = config.format.remove_srgb_suffix();
        if view_format != config.format {
            config.view_formats = vec![view_format];
        }
        if capabilities.usages.contains(wgpu::TextureUsages::COPY_SRC) {
            config.usage |= wgpu::TextureUsages::COPY_SRC;
        }
        config.present_mode = wgpu::PresentMode::AutoVsync;
        config.desired_maximum_frame_latency = 2;
        surface.configure(&device, &config);

        let samples = if antialiasing
            && adapter
                .get_texture_format_features(view_format)
                .flags
                .sample_count_supported(4)
            && adapter
                .get_texture_format_features(DEPTH_FORMAT)
                .flags
                .sample_count_supported(4)
        {
            4
        } else {
            1
        };

        let source = include_str!("wgpu_shaders.wgsl");
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Mixer shaders"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });

        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(UNIFORM_SIZE),
                },
                count: None,
            }],
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Mixer settings per draw"),
            size: SLOT * SLOTS,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &uniforms,
                    offset: 0,
                    size: wgpu::BufferSize::new(UNIFORM_SIZE),
                }),
            }],
        });
        let scene_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&uniform_layout)],
            immediate_size: 0,
        });

        let f = |offset: u64, location: u32, format| wgpu::VertexAttribute {
            format,
            offset,
            shader_location: location,
        };
        use wgpu::VertexFormat::{Float32, Float32x3};
        let line_attributes = [f(0, 0, Float32x3), f(12, 1, Float32x3)];
        let line_layout = wgpu::VertexBufferLayout {
            array_stride: (FLOATS_PER_LINE_VERTEX * 4) as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &line_attributes,
        };
        let point_layout = wgpu::VertexBufferLayout {
            step_mode: wgpu::VertexStepMode::Instance,
            ..line_layout.clone()
        };
        let glow_attributes = [
            f(0, 0, Float32x3),
            f(12, 1, Float32x3),
            f(24, 2, Float32x3),
            f(36, 3, Float32x3),
        ];
        let glow_layout = wgpu::VertexBufferLayout {
            array_stride: (2 * FLOATS_PER_LINE_VERTEX * 4) as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &glow_attributes,
        };
        let mesh_attributes = [f(0, 0, Float32x3), f(12, 1, Float32x3), f(24, 2, Float32)];
        let mesh_layout = wgpu::VertexBufferLayout {
            array_stride: (FLOATS_PER_TRIANGLE_VERTEX * 4) as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &mesh_attributes,
        };

        let blend = Some(wgpu::BlendState::ALPHA_BLENDING);
        let pipeline = |label: &str,
                        layout: &wgpu::PipelineLayout,
                        vs: &str,
                        fs: &str,
                        buffer: wgpu::VertexBufferLayout,
                        topology: wgpu::PrimitiveTopology,
                        depth: wgpu::DepthStencilState,
                        blend: Option<wgpu::BlendState>,
                        target: wgpu::TextureFormat,
                        samples: u32| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers: &[Some(buffer)],
                },
                primitive: wgpu::PrimitiveState {
                    topology,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(depth),
                multisample: wgpu::MultisampleState {
                    count: samples,
                    ..Default::default()
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: target,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let depth = |compare, write: bool| wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(write),
            depth_compare: Some(compare),
            stencil: Default::default(),
            bias: Default::default(),
        };
        use wgpu::CompareFunction::{Always, Less, LessEqual};
        use wgpu::PrimitiveTopology::{LineList, TriangleList, TriangleStrip};
        let background_pipeline = pipeline(
            "background",
            &scene_layout,
            "line_vs",
            "line_fs",
            line_layout.clone(),
            TriangleStrip,
            depth(Always, false),
            None,
            view_format,
            samples,
        );
        let grid_pipeline = pipeline(
            "grid",
            &scene_layout,
            "line_vs",
            "line_fs",
            line_layout.clone(),
            LineList,
            depth(Less, false),
            blend,
            view_format,
            samples,
        );
        // Faces are pushed back slightly so that their edges show cleanly on top.
        let mut face_depth = depth(Less, true);
        face_depth.bias = wgpu::DepthBiasState {
            constant: 2,
            slope_scale: 1.0,
            clamp: 0.0,
        };
        let mesh_pipeline = pipeline(
            "faces",
            &scene_layout,
            "mesh_vs",
            "mesh_fs",
            mesh_layout.clone(),
            TriangleList,
            face_depth.clone(),
            None,
            view_format,
            samples,
        );
        let line_pipeline = pipeline(
            "edges",
            &scene_layout,
            "line_vs",
            "line_fs",
            line_layout.clone(),
            LineList,
            depth(LessEqual, true),
            None,
            view_format,
            samples,
        );
        let glow_pipeline = pipeline(
            "edge glow",
            &scene_layout,
            "glow_vs",
            "line_fs",
            glow_layout,
            TriangleList,
            depth(LessEqual, false),
            blend,
            view_format,
            samples,
        );
        let point_pipeline = pipeline(
            "points",
            &scene_layout,
            "point_vs",
            "line_fs",
            point_layout,
            TriangleList,
            depth(LessEqual, true),
            None,
            view_format,
            samples,
        );
        let pick_pipeline = pipeline(
            "picking",
            &scene_layout,
            "mesh_vs",
            "depth_fs",
            mesh_layout,
            TriangleList,
            depth(Less, true),
            None,
            PICK_FORMAT,
            1,
        );

        // Dear ImGui.
        let ui_uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(16),
                },
                count: None,
            }],
        });
        let ui_texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let ui_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&ui_uniform_layout), Some(&ui_texture_layout)],
            immediate_size: 0,
        });
        let ui_attributes = [
            f(0, 0, wgpu::VertexFormat::Float32x2),
            f(8, 1, wgpu::VertexFormat::Float32x2),
            f(16, 2, wgpu::VertexFormat::Unorm8x4),
        ];
        let ui_pipeline = pipeline(
            "interface",
            &ui_layout,
            "ui_vs",
            "ui_fs",
            wgpu::VertexBufferLayout {
                array_stride: size_of::<DrawVert>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &ui_attributes,
            },
            TriangleList,
            depth(Always, false),
            blend,
            view_format,
            samples,
        );
        let ui_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let ui_uniform_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &ui_uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: ui_uniforms.as_entire_binding(),
            }],
        });
        let ui_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let background = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("background"),
            contents: &as_bytes(&background_corners()),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });
        let grid_floats = grid_lines();
        let grid = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("grid"),
            contents: &as_bytes(&grid_floats),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let grid_vertex_count =
            (grid_floats.len() / FLOATS_PER_VERTEX / GridPlane::ALL.len()) as u32;
        let empty = |label| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: 4,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let ui_vertices = empty("interface vertices");
        let ui_indices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("interface indices"),
            size: 4,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let report = WgpuInfo {
            adapter: info.name.clone(),
            driver: format!("{} {}", info.driver, info.driver_info)
                .trim()
                .to_string(),
            software: info.device_type == wgpu::DeviceType::Cpu,
        };
        let renderer = Self {
            engine,
            triangles: Growing::new(&device, "faces"),
            edges: Growing::new(&device, "edges"),
            points: Growing::new(&device, "points"),
            surface,
            config,
            view_format,
            samples,
            uniforms,
            uniform_group,
            background_pipeline,
            grid_pipeline,
            mesh_pipeline,
            line_pipeline,
            glow_pipeline,
            point_pipeline,
            pick_pipeline,
            ui_pipeline,
            background,
            grid,
            theme_version: palette::version(),
            grid_vertex_count,
            ui_uniforms,
            ui_uniform_group,
            ui_texture_layout,
            ui_sampler,
            ui_texture_group: None,
            ui_vertices,
            ui_indices,
            targets: None,
            pick: std::cell::RefCell::new(None),
            pending: None,
            errors,
            device,
            queue,
        };
        Ok((renderer, report))
    }

    pub fn engine(&self) -> Engine {
        self.engine
    }

    pub fn samples(&self) -> u32 {
        self.samples
    }

    /// The first error the driver reported since the last call, if any.
    pub fn take_error(&self) -> Option<String> {
        let mut list = self.errors.lock().ok()?;
        if list.is_empty() {
            None
        } else {
            Some(list.remove(0))
        }
    }

    /// Builds Dear ImGui's font texture, again after the fonts changed.
    pub fn load_fonts(&mut self, context: &mut imgui::Context) {
        let fonts = context.fonts();
        let atlas = fonts.build_alpha8_texture();
        let size = wgpu::Extent3d {
            width: atlas.width.max(1),
            height: atlas.height.max(1),
            depth_or_array_layers: 1,
        };
        let texture = self.device.create_texture_with_data(
            &self.queue,
            &wgpu::TextureDescriptor {
                label: Some("letters"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            atlas.data,
        );
        let view = texture.create_view(&Default::default());
        self.ui_texture_group = Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.ui_texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.ui_sampler),
                },
            ],
        }));
    }

    /// Replaces all the mesh data on the graphics card.
    pub fn upload_scene(&mut self, scene: &Scene) {
        let (device, queue) = (&self.device, &self.queue);
        self.triangles
            .replace(device, queue, &scene.triangles, FLOATS_PER_TRIANGLE_VERTEX);
        self.edges
            .replace(device, queue, &scene.edges, FLOATS_PER_LINE_VERTEX);
        self.points
            .replace(device, queue, &scene.points, FLOATS_PER_LINE_VERTEX);
    }

    /// Sends only the parts of the scene that changed.
    pub fn update_scene(&self, scene: &Scene, changed: &Changed) {
        self.triangles
            .update(&self.queue, &scene.triangles, &changed.triangles);
        self.edges.update(&self.queue, &scene.edges, &changed.edges);
        self.points
            .update(&self.queue, &scene.points, &changed.points);
    }

    fn write_slot(&self, slot: u64, values: &Uniforms) {
        self.queue
            .write_buffer(&self.uniforms, slot * SLOT, &values.bytes());
    }

    fn resize(&mut self, width: u32, height: u32) {
        let size = (width.max(1), height.max(1));
        if self.config.width != size.0 || self.config.height != size.1 {
            self.config.width = size.0;
            self.config.height = size.1;
            self.surface.configure(&self.device, &self.config);
        }
        if self.targets.as_ref().is_some_and(|t| t.size == size) {
            return;
        }
        let texture = |format, samples| {
            self.device
                .create_texture(&wgpu::TextureDescriptor {
                    label: None,
                    size: wgpu::Extent3d {
                        width: size.0,
                        height: size.1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        self.targets = Some(Targets {
            size,
            multisampled: (self.samples > 1).then(|| texture(self.view_format, self.samples)),
            depth: texture(DEPTH_FORMAT, self.samples),
        });
    }

    /// Draws one frame: the 3D view, then the interface on top. It shows on `present`.
    pub fn render(&mut self, width: i32, height: i32, view: &View, draw_data: &imgui::DrawData) {
        let (width, height) = (width.max(1) as u32, height.max(1) as u32);
        self.resize(width, height);
        self.pending = None;
        if self.theme_version != palette::version() {
            self.theme_version = palette::version();
            self.grid = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("grid"),
                    contents: &as_bytes(&grid_lines()),
                    usage: wgpu::BufferUsages::VERTEX,
                });
            self.queue
                .write_buffer(&self.background, 0, &as_bytes(&background_corners()));
        }
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            // Hidden or busy: this frame is skipped.
            _ => return,
        };
        let frame_view = frame.texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.view_format),
            ..Default::default()
        });

        let size = [width as f32, height as f32];
        let base = Uniforms {
            matrix: view.view_projection,
            extra: [0.0, 1.0, size[0], size[1]],
            face: palette::get(palette::FACE),
            back: palette::get(palette::FACE_BACK),
            selected_face: palette::get(palette::FACE_SELECTED),
            ..Default::default()
        };
        self.write_slot(
            BACKGROUND_SLOT,
            &Uniforms {
                matrix: IDENTITY,
                ..base
            },
        );
        self.write_slot(
            GRID_SLOT,
            &Uniforms {
                fade: [
                    view.fade_center[0],
                    view.fade_center[1],
                    view.fade_center[2],
                    view.fade_radius,
                ],
                extra: [GRID_DEPTH_BIAS, 1.0, size[0], size[1]],
                ..base
            },
        );
        let [lx, ly, lz] = view.light;
        self.write_slot(
            MESH_SLOT,
            &Uniforms {
                light: [lx, ly, lz, if view.shadows { 1.0 } else { 0.0 }],
                extra: [0.0, if view.overlays { 1.0 } else { 0.0 }, size[0], size[1]],
                ..base
            },
        );
        self.write_slot(EDGE_SLOT, &base);
        for (slot, (width, strength)) in [GLOW_SLOT, CORE_SLOT].into_iter().zip(EDGE_GLOW) {
            self.write_slot(
                slot,
                &Uniforms {
                    glow: [width, strength, 0.0, 0.0],
                    ..base
                },
            );
        }
        let [r, g, b] = palette::rgb(palette::EDGE_PLAIN);
        self.write_slot(
            PLAIN_EDGE_SLOT,
            &Uniforms {
                color: [r, g, b, 1.0],
                ..base
            },
        );
        self.write_slot(
            POINT_SLOT,
            &Uniforms {
                extra: [view.point_depth_bias, 1.0, size[0], size[1]],
                ..base
            },
        );
        let ui_ready = self.prepare_ui(draw_data);

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        let Some(targets) = &self.targets else {
            return;
        };
        let (color_view, resolve) = match &targets.multisampled {
            Some(multisampled) => (multisampled, Some(&frame_view)),
            None => (&frame_view, None),
        };
        {
            let [r, g, b] = palette::rgb(palette::BACKGROUND_BOTTOM);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("frame"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color_view,
                    depth_slice: None,
                    resolve_target: resolve,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: r as f64,
                            g: g as f64,
                            b: b as f64,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let slot = |pass: &mut wgpu::RenderPass, slot: u64| {
                pass.set_bind_group(0, &self.uniform_group, &[(slot * SLOT) as u32]);
            };

            // The background: a gradient on a square covering the whole view.
            pass.set_pipeline(&self.background_pipeline);
            slot(&mut pass, BACKGROUND_SLOT);
            pass.set_vertex_buffer(0, self.background.slice(..));
            pass.draw(0..4, 0..1);

            // The grid, blended so that it can fade out.
            if view.grid {
                let first = GridPlane::ALL
                    .iter()
                    .position(|&p| p == view.plane)
                    .unwrap_or(0) as u32;
                pass.set_pipeline(&self.grid_pipeline);
                slot(&mut pass, GRID_SLOT);
                pass.set_vertex_buffer(0, self.grid.slice(..));
                let start = first * self.grid_vertex_count;
                pass.draw(start..start + self.grid_vertex_count, 0..1);
            }

            if self.triangles.count > 0 {
                pass.set_pipeline(&self.mesh_pipeline);
                slot(&mut pass, MESH_SLOT);
                pass.set_vertex_buffer(0, self.triangles.buffer.slice(..));
                pass.draw(0..self.triangles.count, 0..1);
            }

            let edge_count = self.edges.count;
            if edge_count > 0 && (view.overlays || view.plain_edges) {
                pass.set_pipeline(&self.line_pipeline);
                slot(
                    &mut pass,
                    if view.overlays {
                        EDGE_SLOT
                    } else {
                        PLAIN_EDGE_SLOT
                    },
                );
                pass.set_vertex_buffer(0, self.edges.buffer.slice(..));
                pass.draw(0..edge_count, 0..1);
            }
            if edge_count > 1 && view.overlays {
                // Bright edges again: a faint glow, then a thicker core.
                pass.set_pipeline(&self.glow_pipeline);
                pass.set_vertex_buffer(0, self.edges.buffer.slice(..));
                for glow in [GLOW_SLOT, CORE_SLOT] {
                    slot(&mut pass, glow);
                    pass.draw(0..6, 0..edge_count / 2);
                }
            }
            if self.points.count > 0 && view.overlays {
                pass.set_pipeline(&self.point_pipeline);
                slot(&mut pass, POINT_SLOT);
                pass.set_vertex_buffer(0, self.points.buffer.slice(..));
                pass.draw(0..6, 0..self.points.count);
            }

            if ui_ready {
                self.draw_ui(&mut pass, draw_data, width, height);
            }
        }
        self.queue.submit([encoder.finish()]);
        self.pending = Some(frame);
    }

    /// Puts the interface's vertices on the graphics card. False if there is nothing
    /// to draw.
    fn prepare_ui(&mut self, draw_data: &imgui::DrawData) -> bool {
        // Dear ImGui hides a new window on its first frame, leaving nothing to draw.
        if draw_data.draw_lists_count() == 0 || self.ui_texture_group.is_none() {
            return false;
        }
        let mut vertices: Vec<u8> = Vec::new();
        let mut indices: Vec<u8> = Vec::new();
        for list in draw_data.draw_lists() {
            let v = list.vtx_buffer();
            let i = list.idx_buffer();
            // SAFETY: both are plain Copy arrays, viewed as bytes only for the copy.
            unsafe {
                vertices.extend_from_slice(std::slice::from_raw_parts(
                    v.as_ptr().cast::<u8>(),
                    size_of_val(v),
                ));
                indices.extend_from_slice(std::slice::from_raw_parts(
                    i.as_ptr().cast::<u8>(),
                    size_of_val(i),
                ));
            }
        }
        // Copies to the graphics card come in steps of 4 bytes.
        vertices.resize(vertices.len().next_multiple_of(4), 0);
        indices.resize(indices.len().next_multiple_of(4), 0);
        let grow = |buffer: &mut wgpu::Buffer, bytes: &[u8], usage| {
            if bytes.len() as u64 > buffer.size() {
                *buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: None,
                    size: (bytes.len() as u64).next_power_of_two(),
                    usage: usage | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
            }
            if !bytes.is_empty() {
                self.queue.write_buffer(buffer, 0, bytes);
            }
        };
        grow(&mut self.ui_vertices, &vertices, wgpu::BufferUsages::VERTEX);
        grow(&mut self.ui_indices, &indices, wgpu::BufferUsages::INDEX);
        let screen = [
            draw_data.display_size[0].max(1.0),
            draw_data.display_size[1].max(1.0),
            0.0,
            0.0,
        ];
        self.queue
            .write_buffer(&self.ui_uniforms, 0, &as_bytes(&screen));
        true
    }

    fn draw_ui(
        &self,
        pass: &mut wgpu::RenderPass,
        draw_data: &imgui::DrawData,
        width: u32,
        height: u32,
    ) {
        let Some(texture_group) = &self.ui_texture_group else {
            return;
        };
        pass.set_pipeline(&self.ui_pipeline);
        pass.set_bind_group(0, &self.ui_uniform_group, &[]);
        pass.set_bind_group(1, texture_group, &[]);
        pass.set_vertex_buffer(0, self.ui_vertices.slice(..));
        pass.set_index_buffer(self.ui_indices.slice(..), wgpu::IndexFormat::Uint16);
        let (mut first_vertex, mut first_index) = (0i32, 0u32);
        for list in draw_data.draw_lists() {
            for command in list.commands() {
                let DrawCmd::Elements { count, cmd_params } = command else {
                    continue;
                };
                // The interface may be drawn bigger than its own units.
                let scale = draw_data.framebuffer_scale;
                let clip = cmd_params.clip_rect;
                let x0 = (clip[0] * scale[0]).clamp(0.0, width as f32) as u32;
                let y0 = (clip[1] * scale[1]).clamp(0.0, height as f32) as u32;
                let x1 = (clip[2] * scale[0]).clamp(0.0, width as f32) as u32;
                let y1 = (clip[3] * scale[1]).clamp(0.0, height as f32) as u32;
                if x1 <= x0 || y1 <= y0 {
                    continue;
                }
                pass.set_scissor_rect(x0, y0, x1 - x0, y1 - y0);
                let start = first_index + cmd_params.idx_offset as u32;
                pass.draw_indexed(
                    start..start + count as u32,
                    first_vertex + cmd_params.vtx_offset as i32,
                    0..1,
                );
            }
            first_vertex += list.vtx_buffer().len() as i32;
            first_index += list.idx_buffer().len() as u32;
        }
        // OpenGL 3.3 shows the frame by copying it, which a scissor left set would cut.
        pass.set_scissor_rect(0, 0, width, height);
    }

    /// Shows the frame drawn last.
    pub fn present(&mut self) {
        if let Some(frame) = self.pending.take() {
            self.queue.present(frame);
        }
    }

    fn pick_targets(&self, width: u32, height: u32) {
        if self
            .pick
            .borrow()
            .as_ref()
            .is_some_and(|p| p.size == (width, height))
        {
            return;
        }
        let texture = |format, usage| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("picking"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let color = texture(
            PICK_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let depth = texture(DEPTH_FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT);
        let row_bytes = (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        *self.pick.borrow_mut() = Some(PickTargets {
            size: (width, height),
            color_view: color.create_view(&Default::default()),
            color,
            depth: depth.create_view(&Default::default()),
            readback: self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("picking readback"),
                size: row_bytes as u64 * height as u64,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            row_bytes,
        });
    }

    /// Draws only the faces' depth and reads it back, bottom row first, as OpenGL does.
    pub fn read_depth(&self, width: i32, height: i32, view_projection: &Mat4) -> Vec<f32> {
        let (width, height) = (width.max(1) as u32, height.max(1) as u32);
        self.pick_targets(width, height);
        self.write_slot(
            PICK_SLOT,
            &Uniforms {
                matrix: *view_projection,
                extra: [0.0, 0.0, width as f32, height as f32],
                ..Default::default()
            },
        );
        let pick = self.pick.borrow();
        let Some(pick) = pick.as_ref() else {
            return vec![1.0; (width * height) as usize];
        };
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("picking"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &pick.color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 1.0,
                            g: 0.0,
                            b: 0.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &pick.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if self.triangles.count > 0 {
                pass.set_pipeline(&self.pick_pipeline);
                pass.set_bind_group(0, &self.uniform_group, &[(PICK_SLOT * SLOT) as u32]);
                pass.set_vertex_buffer(0, self.triangles.buffer.slice(..));
                pass.draw(0..self.triangles.count, 0..1);
            }
        }
        let rows = read_back(
            &self.device,
            &self.queue,
            encoder,
            &pick.color,
            &pick.readback,
            pick.row_bytes,
            width,
            height,
        );
        let mut depth = Vec::with_capacity((width * height) as usize);
        // Bottom row first.
        for row in rows.chunks_exact(pick.row_bytes as usize).rev() {
            depth.extend(
                row[..width as usize * 4]
                    .chunks_exact(4)
                    .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
            );
        }
        depth.resize((width * height) as usize, 1.0);
        depth
    }

    /// The frame drawn last as RGB bytes, bottom row first, for automated checks.
    /// Empty if this window does not allow reading it.
    pub fn read_pixels(&self, width: i32, height: i32) -> Vec<u8> {
        let (width, height) = (width.max(1) as u32, height.max(1) as u32);
        let Some(frame) = &self.pending else {
            return Vec::new();
        };
        if !self.config.usage.contains(wgpu::TextureUsages::COPY_SRC)
            || (self.config.width, self.config.height) != (width, height)
        {
            return Vec::new();
        }
        let row_bytes = (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: row_bytes as u64 * height as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        let rows = read_back(
            &self.device,
            &self.queue,
            encoder,
            &frame.texture,
            &readback,
            row_bytes,
            width,
            height,
        );
        let bgra = matches!(
            self.config.format,
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
        );
        let mut out = Vec::with_capacity((width * height * 3) as usize);
        for row in rows.chunks_exact(row_bytes as usize).rev() {
            for pixel in row[..width as usize * 4].chunks_exact(4) {
                if bgra {
                    out.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
                } else {
                    out.extend_from_slice(&pixel[..3]);
                }
            }
        }
        out
    }
}

/// The grid's three planes, one after another, in short pieces.
fn grid_lines() -> Vec<f32> {
    GridPlane::ALL
        .into_iter()
        .flat_map(|plane| split_lines(&grid_vertices(plane), 2.0))
        .collect()
}

/// Cuts each line (two vertices of position and color) into pieces about `step` long.
/// Some drivers draw a line wrongly when one end is far behind the camera; short pieces
/// never are.
fn split_lines(vertices: &[f32], step: f32) -> Vec<f32> {
    let mut out = Vec::new();
    for line in vertices.chunks_exact(2 * FLOATS_PER_VERTEX) {
        let (a, b) = line.split_at(FLOATS_PER_VERTEX);
        let length = (0..3).map(|i| (b[i] - a[i]).powi(2)).sum::<f32>().sqrt();
        let pieces = ((length / step).ceil() as usize).max(1);
        let at = |t: f32| -> [f32; 6] {
            [
                a[0] + (b[0] - a[0]) * t,
                a[1] + (b[1] - a[1]) * t,
                a[2] + (b[2] - a[2]) * t,
                a[3],
                a[4],
                a[5],
            ]
        };
        for k in 0..pieces {
            out.extend_from_slice(&at(k as f32 / pieces as f32));
            out.extend_from_slice(&at((k + 1) as f32 / pieces as f32));
        }
    }
    out
}

/// Copies `texture` into `buffer` and waits for the bytes.
#[allow(clippy::too_many_arguments)]
fn read_back(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    mut encoder: wgpu::CommandEncoder,
    texture: &wgpu::Texture,
    buffer: &wgpu::Buffer,
    row_bytes: u32,
    width: u32,
    height: u32,
) -> Vec<u8> {
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row_bytes),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    let slice = buffer.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    let bytes = match receiver.recv() {
        Ok(Ok(())) => match slice.get_mapped_range() {
            Ok(view) => view.to_vec(),
            Err(_) => vec![0; row_bytes as usize * height as usize],
        },
        _ => vec![0; row_bytes as usize * height as usize],
    };
    buffer.unmap();
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_keys_match_the_settings_list() {
        for engine in Engine::ALL {
            assert_eq!(Engine::from_key(engine.key()), Some(engine));
            assert!(
                crate::settings::RENDERERS
                    .iter()
                    .any(|r| r.0 == engine.key() && r.1 == engine.name()),
                "{engine:?}"
            );
        }
        assert_eq!(Engine::from_key("opengl2.1"), None);
    }

    #[test]
    fn opengl_versions_are_checked_from_the_driver_text() {
        assert!(opengl_text_is_new_enough("OpenGL ES 3.2 Mesa 25.2.8"));
        assert!(opengl_text_is_new_enough("4.6.0 NVIDIA 560.94"));
        assert!(opengl_text_is_new_enough("3.3.0 - Build 31.0.101.2111"));
        assert!(!opengl_text_is_new_enough("3.1.0 - Build 9.17.10.4459"));
        assert!(!opengl_text_is_new_enough("OpenGL ES 2.0 Mesa"));
        assert!(opengl_text_is_new_enough(""));
    }

    #[test]
    fn support_words_read_back() {
        for support in [Support::Yes, Support::Software, Support::No] {
            assert_eq!(Support::from_word(support.word()), Some(support));
        }
    }

    #[test]
    fn uniforms_fill_one_slot_in_the_shaders_order() {
        let values = Uniforms {
            matrix: [1.0; 16],
            glow: [9.0, 0.0, 0.0, 0.0],
            ..Default::default()
        };
        let bytes = values.bytes();
        assert_eq!(bytes.len(), SLOT as usize);
        // matrix (64 bytes) and four vec4s (64 bytes) come before the glow.
        assert_eq!(UNIFORM_SIZE, 64 + 8 * 16);
        assert_eq!(&bytes[128..132], &9.0f32.to_le_bytes());
    }

    /// Runs the WGSL through wgpu's shader compiler, as the graphics driver would get it.
    #[test]
    fn the_shaders_compile() {
        let source = include_str!("wgpu_shaders.wgsl");
        // naga is wgpu's compiler; parsing and validating catch mistakes without a GPU.
        let module = naga_check(source);
        assert!(module.is_ok(), "{module:?}");
    }

    fn naga_check(source: &str) -> Result<(), String> {
        let module = naga::front::wgsl::parse_str(source).map_err(|e| e.emit_to_string(source))?;
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .map(|_| ())
        .map_err(|e| format!("{e:?}"))
    }
}
