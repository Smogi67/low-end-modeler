// Shaders for the newer rendering engines (Vulkan, DirectX 12, OpenGL 3.3 through
// wgpu). They draw the same picture as the OpenGL 2.1 shaders in renderer.rs.

struct Uniforms {
    matrix: mat4x4<f32>,
    // A color for every vertex instead of their own, when w is 1.
    color: vec4<f32>,
    // The grid fades out around xyz, and is gone at distance w (0: no fading).
    fade: vec4<f32>,
    // The key light's direction in xyz; w is 1 for shadows, 0 for even light.
    light: vec4<f32>,
    // x: depth bias, y: 1 to show selection, zw: the view's size in pixels.
    extra: vec4<f32>,
    // x: glow width in pixels, y: glow strength.
    glow: vec4<f32>,
}

@group(0) @binding(0) var<uniform> u: Uniforms;

// The camera matrices are made for OpenGL, whose depth runs from -1 to 1;
// here it runs from 0 to 1. The bias pushes things back, as in renderer.rs.
fn to_clip(p: vec3<f32>) -> vec4<f32> {
    var q = u.matrix * vec4<f32>(p, 1.0);
    q.z = (q.z + u.extra.x * q.w + q.w) * 0.5;
    return q;
}

struct LineOut {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) world: vec3<f32>,
}

fn line_color(color: vec3<f32>) -> vec3<f32> {
    return select(color, u.color.rgb, u.color.w > 0.5);
}

@vertex
fn line_vs(@location(0) position: vec3<f32>, @location(1) color: vec3<f32>) -> LineOut {
    var out: LineOut;
    out.position = to_clip(position);
    out.color = vec4<f32>(line_color(color), 1.0);
    out.world = position;
    return out;
}

@fragment
fn line_fs(in: LineOut) -> @location(0) vec4<f32> {
    // The grid fades out with distance, which hides the flicker of far, dense lines.
    var alpha = in.color.a;
    if u.fade.w > 0.0 {
        let d = distance(in.world, u.fade.xyz);
        alpha = alpha * (1.0 - smoothstep(u.fade.w * 0.3, u.fade.w, d));
    }
    return vec4<f32>(in.color.rgb, alpha);
}

fn corner(k: u32) -> vec2<f32> {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    return corners[k];
}

// Points are small squares, 5 pixels wide, as OpenGL's points are.
@vertex
fn point_vs(
    @builtin(vertex_index) k: u32,
    @location(0) position: vec3<f32>,
    @location(1) color: vec3<f32>,
) -> LineOut {
    var out: LineOut;
    var p = to_clip(position);
    let c = corner(k);
    p.x += c.x * 5.0 / u.extra.z * p.w;
    p.y += c.y * 5.0 / u.extra.w * p.w;
    out.position = p;
    out.color = vec4<f32>(line_color(color), 1.0);
    out.world = position;
    return out;
}

// The soft glow around bright edges: each edge becomes a band a few pixels wide.
// Dark edges get none. Where glowing edges meet, the glow adds up at the corner.
@vertex
fn glow_vs(
    @builtin(vertex_index) k: u32,
    @location(0) a: vec3<f32>,
    @location(1) a_color: vec3<f32>,
    @location(2) b: vec3<f32>,
    @location(3) b_color: vec3<f32>,
) -> LineOut {
    var out: LineOut;
    let pa = to_clip(a);
    let pb = to_clip(b);
    let c = corner(k);
    let use_b = c.x > 0.0;
    var p = select(pa, pb, use_b);
    let color = select(a_color, b_color, use_b);
    if pa.w <= 0.0 || pb.w <= 0.0 {
        // Behind the camera: drawn as nothing.
        out.position = vec4<f32>(0.0, 0.0, 2.0, 1.0);
        out.color = vec4<f32>(0.0);
        out.world = a;
        return out;
    }
    let size = u.extra.zw;
    let screen_a = pa.xy / pa.w * size;
    let screen_b = pb.xy / pb.w * size;
    var along = screen_b - screen_a;
    if dot(along, along) < 1e-6 {
        along = vec2<f32>(1.0, 0.0);
    }
    along = normalize(along);
    let across = vec2<f32>(-along.y, along.x);
    // Half the width each way, in pixels, then into clip units.
    let shift = (across * c.y + along * c.x) * u.glow.x * 0.5;
    p.x += shift.x / size.x * 2.0 * p.w;
    p.y += shift.y / size.y * 2.0 * p.w;
    out.position = p;
    let light = dot(color, vec3<f32>(0.3, 0.55, 0.15));
    out.color = vec4<f32>(color, u.glow.y * smoothstep(0.3, 0.55, light));
    out.world = a;
    return out;
}

struct MeshOut {
    @builtin(position) position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) selected: f32,
}

@vertex
fn mesh_vs(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) selected: f32,
) -> MeshOut {
    var out: MeshOut;
    out.position = to_clip(position);
    out.normal = normal;
    out.selected = selected * u.extra.y;
    return out;
}

@fragment
fn mesh_fs(in: MeshOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    // A face seen from behind turns red, which shows inside-out faces at a glance.
    var color = select(vec3<f32>(0.62, 0.20, 0.20), vec3<f32>(0.74, 0.75, 0.77), front);
    color = mix(color, SELECTED_FACE, in.selected * SELECTED_FACE_AMOUNT);
    let side = select(-1.0, 1.0, front);
    let n = normalize(in.normal) * side;
    // A key light near the camera, plus soft light from the sky above.
    let key = max(dot(n, u.light.xyz), 0.0);
    let sky = 0.5 + 0.5 * n.z;
    let lit = mix(0.92, 0.20 + 0.55 * key + 0.30 * sky, u.light.w);
    return vec4<f32>(color * lit, 1.0);
}

// Picking: each pixel's depth, written as a color so it can be read back everywhere.
@fragment
fn depth_fs(in: MeshOut) -> @location(0) vec4<f32> {
    return vec4<f32>(in.position.z, 0.0, 0.0, 1.0);
}

// Dear ImGui.
struct UiUniforms {
    screen: vec4<f32>,
}

@group(0) @binding(0) var<uniform> ui: UiUniforms;
@group(1) @binding(0) var ui_texture: texture_2d<f32>;
@group(1) @binding(1) var ui_sampler: sampler;

struct UiOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
}

@vertex
fn ui_vs(
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
) -> UiOut {
    var out: UiOut;
    out.position = vec4<f32>(
        position.x / ui.screen.x * 2.0 - 1.0,
        1.0 - position.y / ui.screen.y * 2.0,
        0.0,
        1.0,
    );
    out.uv = uv;
    out.color = color;
    return out;
}

@fragment
fn ui_fs(in: UiOut) -> @location(0) vec4<f32> {
    // The font texture holds coverage in one channel only, which saves memory.
    return vec4<f32>(in.color.rgb, in.color.a * textureSample(ui_texture, ui_sampler, in.uv).r);
}
