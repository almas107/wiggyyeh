//! The 3D view on the GPU, with a depth buffer: curves on a shared surface layer in drawing
//! order, and solid paint hides what is behind it exactly (no sorting artefacts).
//!
//! Each picture (one per boil frame) is drawn into an offscreen colour + reverse-Z depth target
//! in two passes, the same as `wobbleworks_3d::raster` does on the CPU:
//! 1. solid paint (fully opaque curves, at least half covered texels) with depth test + write;
//! 2. soft edges and see-through paint (translucent curves, glow, guides, the grid) blended
//!    back to front, depth-tested against the solid paint.
//!
//! The target is then drawn into egui's pass, through the post-process pass in Render mode
//! (depth of field from the depth buffer, bloom, pixelation and grain: the same effects as
//! exports, so the live view matches them).
//! Pictures' vertex buffers stay on the GPU (one per
//! boil frame), so a boiling still view uploads nothing.

use std::collections::HashMap;
use std::sync::Arc;

use eframe::egui_wgpu::{self, CallbackResources, CallbackTrait, RenderState, ScreenDescriptor};
use eframe::wgpu;
use wobbleworks_3d::render::{Frame, Tex};

/// Bytes per vertex: pos (2 × f32), uv (2 × f32), colour (4 × u8), z (f32), solid (f32).
const STRIDE: u64 = 28;
/// Pictures kept on the GPU (one per boil frame).
const KEEP: usize = 12;
const COLOR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

const WGSL: &str = r#"
struct VsIn {
    @location(0) pos: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) z: f32,
    @location(4) solid: f32,
};
struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) solid: f32,
};
@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;

@vertex fn vs(v: VsIn) -> VsOut {
    var o: VsOut;
    o.pos = vec4<f32>(v.pos, v.z, 1.0);
    o.uv = v.uv;
    o.color = v.color;
    o.solid = v.solid;
    return o;
}

// Textures are premultiplied, vertex colours too: paint = texel * colour.
@fragment fn fs_solid(i: VsOut) -> @location(0) vec4<f32> {
    let c = textureSample(tex, samp, i.uv) * i.color;
    if (i.solid < 0.5 || c.a < 0.5) {
        discard;
    }
    return vec4<f32>(c.rgb / c.a, 1.0);
}

@fragment fn fs_blend(i: VsOut) -> @location(0) vec4<f32> {
    let c = textureSample(tex, samp, i.uv) * i.color;
    if (i.solid > 0.5 && c.a >= 0.5) {
        discard;
    }
    return c;
}

struct FullOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex fn vs_full(@builtin(vertex_index) k: u32) -> FullOut {
    let x = f32((k << 1u) & 2u);
    let y = f32(k & 2u);
    var o: FullOut;
    o.pos = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    o.uv = vec2<f32>(x, y);
    return o;
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

@fragment fn fs_full(i: FullOut) -> @location(0) vec4<f32> {
    return textureSample(tex, samp, i.uv);
}

// Post-process (Render mode effects). Sizes are in target pixels.
struct Post {
    size: vec2<f32>,
    // Depth of field: largest blur radius (0 = off) and the focus depth (reverse-Z).
    dof_radius: f32,
    focus_z: f32,
    bloom: f32,
    bloom_radius: f32,
    pixelate: f32,
    grain: f32,
    background: vec4<f32>,
    seed: f32,
    linear_out: f32,
    _pad: vec2<f32>,
};
@group(1) @binding(0) var<uniform> post: Post;
@group(1) @binding(1) var depth_tex: texture_2d<f32>;

fn hash(p: vec2<f32>) -> f32 {
    let q = fract(p * vec2<f32>(0.1031, 0.1030));
    let r = q + dot(q, q.yx + 33.33);
    return fract((r.x + r.y) * r.x);
}

// Premultiplied scene colour over the background, at a target pixel position.
fn scene_at(px: vec2<f32>) -> vec4<f32> {
    let c = textureSampleLevel(tex, samp, px / post.size, 0.0);
    return c + post.background * (1.0 - c.a);
}

fn depth_at(px: vec2<f32>) -> f32 {
    let m = vec2<i32>(post.size) - vec2<i32>(1, 1);
    return textureLoad(depth_tex, clamp(vec2<i32>(px), vec2<i32>(0, 0), m), 0).r;
}

@fragment fn fs_post(i: FullOut) -> @location(0) vec4<f32> {
    var px = i.uv * post.size;
    var c = vec4<f32>(0.0);
    if (post.pixelate > 1.0) {
        // Pixelation: the block's average (4 × 4 taps), like the exports.
        let origin = floor(px / post.pixelate) * post.pixelate;
        for (var k = 0; k < 16; k = k + 1) {
            let o = (vec2<f32>(f32(k % 4), f32(k / 4)) + 0.5) / 4.0;
            c = c + scene_at(origin + o * post.pixelate);
        }
        c = c / 16.0;
        px = origin + 0.5 * post.pixelate;
    } else {
        c = scene_at(px);
    }
    // Depth of field: a disc of taps (golden-angle spiral) sized by the distance from focus.
    if (post.dof_radius > 0.0 && post.focus_z > 0.0) {
        let z = depth_at(px);
        var off = 1.0;
        if (z > 0.0) {
            off = min(abs(1.0 - z / post.focus_z), 1.0);
        }
        let r = post.dof_radius * off;
        if (r >= 0.5) {
            var acc = c;
            for (var k = 1; k < 24; k = k + 1) {
                let t = f32(k) / 24.0;
                let a = f32(k) * 2.39996;
                acc = acc + scene_at(px + vec2<f32>(cos(a), sin(a)) * sqrt(t) * r);
            }
            c = acc / 24.0;
        }
    }
    // Bloom: the bright parts, blurred, added back.
    if (post.bloom > 0.0) {
        var glow = vec3<f32>(0.0);
        for (var k = 0; k < 24; k = k + 1) {
            let t = (f32(k) + 0.5) / 24.0;
            let a = f32(k) * 2.39996;
            let s = scene_at(px + vec2<f32>(cos(a), sin(a)) * sqrt(t) * post.bloom_radius * 2.0);
            let l = (s.r + s.g + s.b) / 3.0;
            glow = glow + s.rgb * clamp((l - 0.65) / 0.35, 0.0, 1.0);
        }
        glow = glow / 24.0;
        c = vec4<f32>(min(c.rgb + glow * post.bloom * 1.5, vec3<f32>(max(c.a, 1.0))), c.a);
    }
    if (post.grain > 0.0) {
        let n = (hash(floor(i.uv * post.size) + vec2<f32>(post.seed * 17.0, post.seed * 31.0)) - 0.5) * post.grain * 0.25;
        c = vec4<f32>(clamp(c.rgb + vec3<f32>(n * c.a), vec3<f32>(0.0), vec3<f32>(c.a)), c.a);
    }
    if (post.linear_out > 0.5 && c.a > 0.0) {
        return vec4<f32>(to_linear(c.rgb / c.a) * c.a, c.a);
    }
    return c;
}

@fragment fn fs_full_linear(i: FullOut) -> @location(0) vec4<f32> {
    let c = textureSample(tex, samp, i.uv);
    if (c.a <= 0.0) {
        return c;
    }
    return vec4<f32>(to_linear(c.rgb / c.a) * c.a, c.a);
}
"#;

/// Render mode effects for the live view, in target pixels (see `fs_post`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Post {
    pub dof_radius: f32,
    pub focus_z: f32,
    pub bloom: f32,
    pub bloom_radius: f32,
    pub pixelate: f32,
    pub grain: f32,
    /// Straight RGBA 0..1 (the shader premultiplies it).
    pub background: [f32; 4],
    pub seed: f32,
}

impl Post {
    /// The effects of an environment for a target of `size` pixels; `None` when there is
    /// nothing to do (not Render mode, or every effect off). Same sizes as the exports.
    pub fn of(env: &wobbleworks_3d::model::Environment, focus_z: f32, frame: u32, size: [u32; 2]) -> Option<Post> {
        let e = &env.effects;
        let short = size[0].min(size[1]) as f32;
        let dof_radius = if e.dof > 0.0 { (short * 0.02 * (2.8 / e.dof.clamp(0.7, 22.0))).clamp(0.0, 40.0) } else { 0.0 };
        let ok = |v: f32| if v.is_finite() { v } else { 0.0 };
        let post = Post {
            dof_radius: ok(dof_radius),
            focus_z: ok(focus_z),
            bloom: ok(e.bloom).clamp(0.0, 1.0),
            bloom_radius: (short * 0.012).max(2.0),
            pixelate: if e.pixelate > 1.0 { ok(e.pixelate).clamp(1.0, 32.0).floor() } else { 0.0 },
            grain: ok(e.grain).clamp(0.0, 1.0),
            background: env.background.0.map(|c| f32::from(c) / 255.0),
            seed: (frame % 64) as f32,
        };
        let any = post.dof_radius > 0.0 && post.focus_z > 0.0 || post.bloom > 0.0 || post.pixelate > 1.0 || post.grain > 0.0;
        (env.render_mode && any).then_some(post)
    }

    fn bytes(&self, size: [u32; 2], linear_out: bool) -> Vec<u8> {
        let mut v = vec![size[0] as f32, size[1] as f32, self.dof_radius, self.focus_z, self.bloom, self.bloom_radius, self.pixelate, self.grain];
        let b = self.background;
        v.extend([b[0] * b[3], b[1] * b[3], b[2] * b[3], b[3], self.seed, if linear_out { 1.0 } else { 0.0 }, 0.0, 0.0]);
        v.iter().flat_map(|f| f.to_le_bytes()).collect()
    }
}

/// Bytes in the `Post` uniform.
const POST_BYTES: u64 = 64;

/// Cutout paint over the background image: the shape from the atlas (group 0), the colour from
/// the image (group 1) at the pixel's place on screen through the cover rectangle (group 2).
/// Its own module (with the scene's vertex stage) so its bindings never meet the post pass's.
const CUT_WGSL: &str = r#"
@group(1) @binding(0) var cut_tex: texture_2d<f32>;
@group(1) @binding(1) var cut_samp: sampler;
struct Cut {
    cover: vec4<f32>,
    size: vec2<f32>,
    _pad: vec2<f32>,
};
@group(2) @binding(0) var<uniform> cut: Cut;

fn cut_paint(i: VsOut) -> vec4<f32> {
    let a = (textureSample(tex, samp, i.uv) * i.color).a;
    let f = i.pos.xy / cut.size;
    let img = textureSample(cut_tex, cut_samp, mix(cut.cover.xy, cut.cover.zw, f));
    return img * a;
}

@fragment fn fs_solid_cut(i: VsOut) -> @location(0) vec4<f32> {
    let c = cut_paint(i);
    if (i.solid < 0.5 || c.a < 0.5) {
        discard;
    }
    return vec4<f32>(c.rgb / c.a, 1.0);
}

@fragment fn fs_blend_cut(i: VsOut) -> @location(0) vec4<f32> {
    let c = cut_paint(i);
    if (i.solid > 0.5 && c.a >= 0.5) {
        discard;
    }
    return c;
}
"#;

/// The scene's shared part (vertex stage, atlas binding) followed by the Cutout fragments.
fn cut_source() -> String {
    let scene = WGSL.split("struct FullOut").next().unwrap_or(WGSL);
    format!("{scene}\n{CUT_WGSL}")
}

/// Bytes in the `Cut` uniform.
const CUT_BYTES: u64 = 32;

/// A picture ready for the GPU: interleaved vertices, indices and draws per texture.
pub struct Prepared {
    /// Identifies the picture (editor revision, boil frame, size).
    pub id: u64,
    /// The editor revision it belongs to (older ones are dropped from the GPU).
    pub revision: u64,
    pub vertices: Vec<u8>,
    pub indices: Vec<u32>,
    pub draws: Vec<(Tex, u32, u32)>,
    /// Where the background image sits on screen (for Cutout paint).
    pub cover: [f32; 4],
}

/// Turn a frame (vertex positions in points within a `w` × `h` viewport) into GPU data.
pub fn prepare(frame: &Frame, w: f32, h: f32, id: u64, revision: u64) -> Prepared {
    let (w, h) = (w.max(1.0), h.max(1.0));
    let total: usize = frame.batches.iter().map(|b| b.vertices.len()).sum();
    let mut vertices = Vec::with_capacity(total * STRIDE as usize);
    let mut indices = Vec::new();
    let mut draws: Vec<(Tex, u32, u32)> = Vec::new();
    let mut base = 0u32;
    for b in &frame.batches {
        for v in &b.vertices {
            let x = v.pos[0] / w * 2.0 - 1.0;
            let y = 1.0 - v.pos[1] / h * 2.0;
            for f in [x, y, v.uv[0], v.uv[1]] {
                vertices.extend_from_slice(&f.to_le_bytes());
            }
            vertices.extend_from_slice(&v.color);
            vertices.extend_from_slice(&v.z.to_le_bytes());
            vertices.extend_from_slice(&(if v.solid { 1.0f32 } else { 0.0 }).to_le_bytes());
        }
        let start = indices.len() as u32;
        indices.extend(b.indices.iter().map(|i| i + base));
        let count = indices.len() as u32 - start;
        match draws.last_mut() {
            Some((t, s, c)) if *t == b.tex && *s + *c == start => *c += count,
            _ => draws.push((b.tex, start, count)),
        }
        base = base.saturating_add(b.vertices.len() as u32);
    }
    Prepared { id, revision, vertices, indices, draws, cover: frame.cover }
}

/// Textures the pictures use: the atlas and image resources, premultiplied RGBA8.
pub struct TextureData {
    /// (key, width, height, pixels): uploaded when the key changes.
    pub atlas: (u64, u32, u32, Arc<Vec<u8>>),
    pub images: Vec<(u64, u32, u32, Arc<Vec<u8>>)>,
}

/// Straight → premultiplied alpha (GPU filtering then blends edges right).
pub fn premultiply(rgba: &[u8]) -> Vec<u8> {
    rgba.chunks_exact(4)
        .flat_map(|p| {
            let a = u16::from(p[3]);
            let m = |c: u8| ((u16::from(c) * a + 127) / 255) as u8;
            [m(p[0]), m(p[1]), m(p[2]), p[3]]
        })
        .collect()
}

struct Picture {
    revision: u64,
    vbuf: wgpu::Buffer,
    ibuf: wgpu::Buffer,
    draws: Vec<(Tex, u32, u32)>,
    cover: [f32; 4],
    used: u64,
}

struct Target {
    size: [u32; 2],
    color: wgpu::TextureView,
    depth: wgpu::TextureView,
    bind: wgpu::BindGroup,
    /// The post-process inputs: the uniform and the depth buffer.
    post_bind: wgpu::BindGroup,
    shows: Option<u64>,
}

struct Resources {
    solid: wgpu::RenderPipeline,
    blend: wgpu::RenderPipeline,
    full: wgpu::RenderPipeline,
    solid_cut: wgpu::RenderPipeline,
    blend_cut: wgpu::RenderPipeline,
    cut_buf: wgpu::Buffer,
    cut_bind: wgpu::BindGroup,
    post: wgpu::RenderPipeline,
    post_bgl: wgpu::BindGroupLayout,
    post_buf: wgpu::Buffer,
    linear_out: bool,
    bgl: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    atlas: Option<(u64, wgpu::BindGroup)>,
    images: HashMap<u64, wgpu::BindGroup>,
    pictures: HashMap<u64, Picture>,
    target: Option<Target>,
    tick: u64,
}

fn texture_bind(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    bgl: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    w: u32,
    h: u32,
    px: &[u8],
) -> Option<wgpu::BindGroup> {
    let max = device.limits().max_texture_dimension_2d;
    if w == 0 || h == 0 || w > max || h > max || px.len() as u64 != u64::from(w) * u64::from(h) * 4 {
        return None;
    }
    let extent = wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("w3d_texture"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: COLOR,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        px,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) },
        extent,
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("w3d_texture"),
        layout: bgl,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
        ],
    }))
}

impl Resources {
    fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Resources {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("w3d"), source: wgpu::ShaderSource::Wgsl(WGSL.into()) });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("w3d_tex"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
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
        let layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("w3d"), bind_group_layouts: &[Some(&bgl)], immediate_size: 0 });
        let vertex = Some(wgpu::VertexBufferLayout {
            array_stride: STRIDE,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 0, shader_location: 0 },
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 8, shader_location: 1 },
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Unorm8x4, offset: 16, shader_location: 2 },
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32, offset: 20, shader_location: 3 },
                wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32, offset: 24, shader_location: 4 },
            ],
        });
        let premul = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let cut_module =
            device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("w3d_cut"), source: wgpu::ShaderSource::Wgsl(cut_source().into()) });
        let cut_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("w3d_cut"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let cut_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("w3d_cut"),
            bind_group_layouts: &[Some(&bgl), Some(&bgl), Some(&cut_bgl)],
            immediate_size: 0,
        });
        let cut_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("w3d_cut"),
            size: CUT_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let cut_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("w3d_cut"),
            layout: &cut_bgl,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: cut_buf.as_entire_binding() }],
        });
        let scene = |label: &str,
                     module: &wgpu::ShaderModule,
                     layout: &wgpu::PipelineLayout,
                     fs: &str,
                     write: bool,
                     compare: wgpu::CompareFunction,
                     blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState { module, entry_point: Some("vs"), buffers: std::slice::from_ref(&vertex), compilation_options: Default::default() },
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH,
                    depth_write_enabled: Some(write),
                    depth_compare: Some(compare),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState { count: 1, mask: !0, alpha_to_coverage_enabled: false },
                fragment: Some(wgpu::FragmentState {
                    module,
                    entry_point: Some(fs),
                    targets: &[Some(wgpu::ColorTargetState { format: COLOR, blend, write_mask: wgpu::ColorWrites::ALL })],
                    compilation_options: Default::default(),
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let solid = scene("w3d_solid", &module, &layout, "fs_solid", true, wgpu::CompareFunction::Greater, None);
        let blend = scene("w3d_blend", &module, &layout, "fs_blend", false, wgpu::CompareFunction::GreaterEqual, Some(premul));
        let solid_cut = scene("w3d_solid_cut", &cut_module, &cut_layout, "fs_solid_cut", true, wgpu::CompareFunction::Greater, None);
        let blend_cut = scene("w3d_blend_cut", &cut_module, &cut_layout, "fs_blend_cut", false, wgpu::CompareFunction::GreaterEqual, Some(premul));
        let full = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("w3d_full"),
            layout: Some(&layout),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vs_full"), buffers: &[], compilation_options: Default::default() },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
            depth_stencil: None,
            multisample: wgpu::MultisampleState { count: 1, mask: !0, alpha_to_coverage_enabled: false },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some(if target_format.is_srgb() { "fs_full_linear" } else { "fs_full" }),
                targets: &[Some(wgpu::ColorTargetState { format: target_format, blend: Some(premul), write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            multiview_mask: None,
            cache: None,
        });
        let post_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("w3d_post"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        // Depth32Float read as unfilterable float: `texelFetch` on WebGL2.
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });
        let post_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("w3d_post"),
            bind_group_layouts: &[Some(&bgl), Some(&post_bgl)],
            immediate_size: 0,
        });
        let post = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("w3d_post"),
            layout: Some(&post_layout),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vs_full"), buffers: &[], compilation_options: Default::default() },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
            depth_stencil: None,
            multisample: wgpu::MultisampleState { count: 1, mask: !0, alpha_to_coverage_enabled: false },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_post"),
                targets: &[Some(wgpu::ColorTargetState { format: target_format, blend: Some(premul), write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            multiview_mask: None,
            cache: None,
        });
        let post_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("w3d_post"),
            size: POST_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("w3d"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let linear_out = target_format.is_srgb();
        Resources {
            solid,
            blend,
            full,
            solid_cut,
            blend_cut,
            cut_buf,
            cut_bind,
            post,
            post_bgl,
            post_buf,
            linear_out,
            bgl,
            sampler,
            atlas: None,
            images: HashMap::new(),
            pictures: HashMap::new(),
            target: None,
            tick: 0,
        }
    }

    fn ensure_target(&mut self, device: &wgpu::Device, size: [u32; 2]) {
        if self.target.as_ref().is_some_and(|t| t.size == size) {
            return;
        }
        let max = device.limits().max_texture_dimension_2d;
        let size = [size[0].clamp(1, max), size[1].clamp(1, max)];
        let extent = wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 };
        let make = |format, usage, label| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: extent,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let color = make(COLOR, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING, "w3d_color")
            .create_view(&wgpu::TextureViewDescriptor::default());
        let depth = make(DEPTH, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING, "w3d_depth")
            .create_view(&wgpu::TextureViewDescriptor::default());
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("w3d_target"),
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&color) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        let post_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("w3d_post"),
            layout: &self.post_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.post_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&depth) },
            ],
        });
        self.target = Some(Target { size, color, depth, bind, post_bind, shows: None });
    }

    fn bind_for(&self, tex: Tex) -> Option<&wgpu::BindGroup> {
        match tex {
            Tex::Atlas => self.atlas.as_ref().map(|(_, b)| b),
            Tex::Image(id) => self.images.get(&id),
            // The shape comes from the atlas; the image is bound beside it.
            Tex::Cutout(_) => self.atlas.as_ref().map(|(_, b)| b),
        }
    }
}

/// The GPU view: set up once with the app's wgpu state.
#[derive(Clone)]
pub struct Gpu;

impl Gpu {
    pub fn new(rs: &RenderState) -> Gpu {
        let res = Resources::new(&rs.device, rs.target_format);
        rs.renderer.write().callback_resources.insert(res);
        Gpu
    }

    /// Draw a picture: rendered offscreen with depth, then shown in `rect`.
    /// `post` adds the Render mode effects (sized for [`Gpu::size`]).
    pub fn show(&self, painter: &egui::Painter, rect: egui::Rect, ppp: f32, picture: Arc<Prepared>, textures: Arc<TextureData>, post: Option<Post>) {
        let size = Gpu::size(rect, ppp);
        painter.add(egui_wgpu::Callback::new_paint_callback(rect, SceneCallback { picture, textures, size, post }));
    }

    /// The target size in physical pixels for a view rectangle.
    pub fn size(rect: egui::Rect, ppp: f32) -> [u32; 2] {
        [(rect.width() * ppp).round().max(1.0) as u32, (rect.height() * ppp).round().max(1.0) as u32]
    }
}

struct SceneCallback {
    picture: Arc<Prepared>,
    textures: Arc<TextureData>,
    /// Target size in physical pixels.
    size: [u32; 2],
    post: Option<Post>,
}

/// Upload what changed: textures, and the picture's buffers (kept per boil frame).
fn upload(res: &mut Resources, device: &wgpu::Device, queue: &wgpu::Queue, p: &Prepared, textures: &TextureData) {
    res.tick += 1;
    let (key, w, h, px) = &textures.atlas;
    if res.atlas.as_ref().is_none_or(|(k, _)| k != key)
        && let Some(b) = texture_bind(device, queue, &res.bgl, &res.sampler, *w, *h, px)
    {
        res.atlas = Some((*key, b));
        if let Some(t) = &mut res.target {
            t.shows = None;
        }
    }
    for (id, w, h, px) in &textures.images {
        if !res.images.contains_key(id)
            && let Some(b) = texture_bind(device, queue, &res.bgl, &res.sampler, *w, *h, px)
        {
            res.images.insert(*id, b);
        }
    }
    let live: Vec<u64> = textures.images.iter().map(|i| i.0).collect();
    res.images.retain(|id, _| live.contains(id));
    res.pictures.retain(|_, pic| pic.revision == p.revision);
    if !res.pictures.contains_key(&p.id) && !p.indices.is_empty() {
        use wgpu::util::DeviceExt;
        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("w3d_vertices"),
            contents: &p.vertices,
            usage: wgpu::BufferUsages::VERTEX,
        });
        let ibytes: Vec<u8> = p.indices.iter().flat_map(|i| i.to_le_bytes()).collect();
        let ibuf =
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("w3d_indices"), contents: &ibytes, usage: wgpu::BufferUsages::INDEX });
        if res.pictures.len() >= KEEP
            && let Some(old) = res.pictures.iter().min_by_key(|(_, pic)| pic.used).map(|(k, _)| *k)
        {
            res.pictures.remove(&old);
        }
        res.pictures.insert(p.id, Picture { revision: p.revision, vbuf, ibuf, draws: p.draws.clone(), cover: p.cover, used: res.tick });
    }
    let tick = res.tick;
    if let Some(pic) = res.pictures.get_mut(&p.id) {
        pic.used = tick;
    }
}

/// Render a picture into the offscreen target: solid pass, then the blended pass.
fn draw_offscreen(res: &mut Resources, device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, id: u64, size: [u32; 2]) {
    res.ensure_target(device, size);
    let Some(target) = &res.target else { return };
    if target.shows == Some(id) {
        return;
    }
    if let Some(pic) = res.pictures.get(&id)
        && pic.draws.iter().any(|d| matches!(d.0, Tex::Cutout(_)))
    {
        let mut v = pic.cover.to_vec();
        v.extend([target.size[0] as f32, target.size[1] as f32, 0.0, 0.0]);
        let bytes: Vec<u8> = v.iter().flat_map(|f| f.to_le_bytes()).collect();
        queue.write_buffer(&res.cut_buf, 0, &bytes);
    }
    {
        let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("w3d_scene"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.color,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &target.depth,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0.0), store: wgpu::StoreOp::Store }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        if let Some(pic) = res.pictures.get(&id) {
            rp.set_vertex_buffer(0, pic.vbuf.slice(..));
            rp.set_index_buffer(pic.ibuf.slice(..), wgpu::IndexFormat::Uint32);
            for (pipeline, cut_pipeline) in [(&res.solid, &res.solid_cut), (&res.blend, &res.blend_cut)] {
                for (tex, start, count) in &pic.draws {
                    let Some(bind) = res.bind_for(*tex) else { continue };
                    if let Tex::Cutout(image) = tex {
                        let Some(image) = res.images.get(image) else { continue };
                        rp.set_pipeline(cut_pipeline);
                        rp.set_bind_group(1, image, &[]);
                        rp.set_bind_group(2, &res.cut_bind, &[]);
                    } else {
                        rp.set_pipeline(pipeline);
                    }
                    rp.set_bind_group(0, bind, &[]);
                    rp.draw_indexed(*start..start + count, 0, 0..1);
                }
            }
        }
    }
    if let Some(t) = &mut res.target {
        t.shows = Some(id);
    }
}

impl CallbackTrait for SceneCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(res) = resources.get_mut::<Resources>() {
            upload(res, device, queue, &self.picture, &self.textures);
            draw_offscreen(res, device, queue, encoder, self.picture.id, self.size);
            if let (Some(post), Some(t)) = (&self.post, &res.target) {
                queue.write_buffer(&res.post_buf, 0, &post.bytes(t.size, res.linear_out));
            }
        }
        Vec::new()
    }

    fn paint(&self, info: egui::PaintCallbackInfo, pass: &mut wgpu::RenderPass<'static>, resources: &CallbackResources) {
        let Some(res) = resources.get::<Resources>() else { return };
        let Some(t) = &res.target else { return };
        let vp = info.viewport_in_pixels();
        let clip = info.clip_rect_in_pixels();
        if vp.width_px <= 0 || vp.height_px <= 0 || clip.width_px <= 0 || clip.height_px <= 0 {
            return;
        }
        pass.set_viewport(vp.left_px as f32, vp.top_px as f32, vp.width_px as f32, vp.height_px as f32, 0.0, 1.0);
        pass.set_scissor_rect(clip.left_px.max(0) as u32, clip.top_px.max(0) as u32, clip.width_px as u32, clip.height_px as u32);
        if self.post.is_some() {
            pass.set_pipeline(&res.post);
            pass.set_bind_group(1, &t.post_bind, &[]);
        } else {
            pass.set_pipeline(&res.full);
        }
        pass.set_bind_group(0, &t.bind, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wobbleworks_3d::render::{Batch, Vtx};

    #[test]
    fn frames_flatten_into_one_buffer_with_draws_per_texture() {
        let v = |x: f32| Vtx { pos: [x, 10.0], uv: [0.0, 0.0], color: [255, 0, 0, 255], z: 0.5, solid: true };
        let frame = Frame {
            batches: vec![
                Batch { tex: Tex::Atlas, vertices: vec![v(0.0), v(10.0), v(20.0)], indices: vec![0, 1, 2] },
                Batch { tex: Tex::Atlas, vertices: vec![v(0.0), v(10.0), v(20.0)], indices: vec![0, 1, 2] },
                Batch { tex: Tex::Image(4), vertices: vec![v(0.0), v(10.0), v(20.0)], indices: vec![2, 1, 0] },
            ],
            triangles: 3,
            ..Frame::default()
        };
        let p = prepare(&frame, 20.0, 20.0, 1, 1);
        assert_eq!(p.vertices.len(), 9 * STRIDE as usize);
        assert_eq!(p.indices, vec![0, 1, 2, 3, 4, 5, 8, 7, 6]);
        assert_eq!(p.draws, vec![(Tex::Atlas, 0, 6), (Tex::Image(4), 6, 3)]);
        // x = 0 → -1, x = 20 → 1 in NDC.
        let f = |o: usize| f32::from_le_bytes([p.vertices[o], p.vertices[o + 1], p.vertices[o + 2], p.vertices[o + 3]]);
        assert_eq!(f(0), -1.0);
        assert_eq!(f(2 * STRIDE as usize), 1.0);
        assert_eq!(premultiply(&[200, 100, 0, 128]), vec![100, 50, 0, 128]);
    }

    #[test]
    fn post_runs_only_in_render_mode_with_an_effect_on() {
        use wobbleworks_3d::model::Environment;
        let mut env = Environment::default();
        env.effects.grain = 0.5;
        assert_eq!(Post::of(&env, 0.5, 0, [800, 600]), None, "not Render mode");
        env.render_mode = true;
        let p = Post::of(&env, 0.5, 3, [800, 600]).expect("grain");
        assert_eq!(p.grain, 0.5);
        assert_eq!(p.bytes([800, 600], false).len() as u64, POST_BYTES);
        env.effects.grain = 0.0;
        assert_eq!(Post::of(&env, 0.5, 0, [800, 600]), None, "nothing on");
        env.effects.dof = 2.8;
        let p = Post::of(&env, 0.5, 0, [800, 600]).expect("dof");
        assert!((p.dof_radius - 12.0).abs() < 1e-3, "{}", p.dof_radius);
        env.effects.pixelate = f32::NAN;
        env.effects.bloom = f32::INFINITY;
        let p = Post::of(&env, f32::NAN, 0, [0, 0]).unwrap_or(p);
        assert!(p.pixelate.is_finite() && p.bloom.is_finite() && p.focus_z.is_finite());
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[allow(clippy::expect_used, clippy::panic)]
mod shader_tests {
    /// The shader parses, validates, and translates to WebGL2's GLSL ES 3.0 for every entry
    /// point (a broken shader would only show at run time otherwise).
    #[test]
    fn the_shader_is_valid_for_webgpu_and_webgl2() {
        for source in [super::WGSL.to_string(), super::cut_source()] {
            check(&source);
        }
    }

    fn check(source: &str) {
        let module = naga::front::wgsl::parse_str(source).expect("parse");
        let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty()).validate(&module).expect("validate");
        for ep in &module.entry_points {
            let options = naga::back::glsl::Options { version: naga::back::glsl::Version::Embedded { version: 300, is_webgl: true }, ..Default::default() };
            let pipeline = naga::back::glsl::PipelineOptions { shader_stage: ep.stage, entry_point: ep.name.clone(), multiview: None };
            let mut out = String::new();
            let mut writer = naga::back::glsl::Writer::new(&mut out, &module, &info, &options, &pipeline, naga::proc::BoundsCheckPolicies::default())
                .unwrap_or_else(|e| panic!("{}: {e}", ep.name));
            writer.write().unwrap_or_else(|e| panic!("{}: {e}", ep.name));
        }
    }
}
