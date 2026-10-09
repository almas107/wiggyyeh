//! The 3D view on the GPU, with a depth buffer: curves on a shared surface layer in drawing
//! order, and solid paint hides what is behind it exactly (no sorting artefacts).
//!
//! Each picture (one per boil frame) is drawn into an offscreen colour + reverse-Z depth target
//! in two passes, the same as `wobbleworks_3d::raster` does on the CPU:
//! 1. solid paint (fully opaque curves, at least half covered texels) with depth test + write;
//! 2. soft edges and see-through paint (translucent curves, glow, guides, the grid) blended
//!    back to front, depth-tested against the solid paint.
//!
//! The target is then drawn into egui's pass. Pictures' vertex buffers stay on the GPU (one per
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

@fragment fn fs_full_linear(i: FullOut) -> @location(0) vec4<f32> {
    let c = textureSample(tex, samp, i.uv);
    if (c.a <= 0.0) {
        return c;
    }
    return vec4<f32>(to_linear(c.rgb / c.a) * c.a, c.a);
}
"#;

/// A picture ready for the GPU: interleaved vertices, indices and draws per texture.
pub struct Prepared {
    /// Identifies the picture (editor revision, boil frame, size).
    pub id: u64,
    /// The editor revision it belongs to (older ones are dropped from the GPU).
    pub revision: u64,
    pub vertices: Vec<u8>,
    pub indices: Vec<u32>,
    pub draws: Vec<(Tex, u32, u32)>,
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
    Prepared { id, revision, vertices, indices, draws }
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
    used: u64,
}

struct Target {
    size: [u32; 2],
    color: wgpu::TextureView,
    depth: wgpu::TextureView,
    bind: wgpu::BindGroup,
    shows: Option<u64>,
}

struct Resources {
    solid: wgpu::RenderPipeline,
    blend: wgpu::RenderPipeline,
    full: wgpu::RenderPipeline,
    bgl: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    atlas: Option<(u64, wgpu::BindGroup)>,
    images: HashMap<u64, wgpu::BindGroup>,
    pictures: HashMap<u64, Picture>,
    target: Option<Target>,
    tick: u64,
}

fn texture_bind(device: &wgpu::Device, queue: &wgpu::Queue, bgl: &wgpu::BindGroupLayout, sampler: &wgpu::Sampler, w: u32, h: u32, px: &[u8]) -> Option<wgpu::BindGroup> {
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
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }, wgpu::BindGroupEntry {
            binding: 1,
            resource: wgpu::BindingResource::Sampler(sampler),
        }],
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
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("w3d"), bind_group_layouts: &[Some(&bgl)], immediate_size: 0 });
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
            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add },
            alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add },
        };
        let scene = |label: &str, fs: &str, write: bool, compare: wgpu::CompareFunction, blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), buffers: std::slice::from_ref(&vertex), compilation_options: Default::default() },
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
                    module: &module,
                    entry_point: Some(fs),
                    targets: &[Some(wgpu::ColorTargetState { format: COLOR, blend, write_mask: wgpu::ColorWrites::ALL })],
                    compilation_options: Default::default(),
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let solid = scene("w3d_solid", "fs_solid", true, wgpu::CompareFunction::Greater, None);
        let blend = scene("w3d_blend", "fs_blend", false, wgpu::CompareFunction::GreaterEqual, Some(premul));
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
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("w3d"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Resources { solid, blend, full, bgl, sampler, atlas: None, images: HashMap::new(), pictures: HashMap::new(), target: None, tick: 0 }
    }

    fn ensure_target(&mut self, device: &wgpu::Device, size: [u32; 2]) {
        if self.target.as_ref().is_some_and(|t| t.size == size) {
            return;
        }
        let max = device.limits().max_texture_dimension_2d;
        let size = [size[0].clamp(1, max), size[1].clamp(1, max)];
        let extent = wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 };
        let make = |format, usage, label| {
            device.create_texture(&wgpu::TextureDescriptor { label: Some(label), size: extent, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format, usage, view_formats: &[] })
        };
        let color = make(COLOR, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING, "w3d_color").create_view(&wgpu::TextureViewDescriptor::default());
        let depth = make(DEPTH, wgpu::TextureUsages::RENDER_ATTACHMENT, "w3d_depth").create_view(&wgpu::TextureViewDescriptor::default());
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("w3d_target"),
            layout: &self.bgl,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&color) }, wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&self.sampler),
            }],
        });
        self.target = Some(Target { size, color, depth, bind, shows: None });
    }

    fn bind_for(&self, tex: Tex) -> Option<&wgpu::BindGroup> {
        match tex {
            Tex::Atlas => self.atlas.as_ref().map(|(_, b)| b),
            Tex::Image(id) => self.images.get(&id),
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
    pub fn show(&self, painter: &egui::Painter, rect: egui::Rect, ppp: f32, picture: Arc<Prepared>, textures: Arc<TextureData>) {
        let size = [(rect.width() * ppp).round().max(1.0) as u32, (rect.height() * ppp).round().max(1.0) as u32];
        painter.add(egui_wgpu::Callback::new_paint_callback(rect, SceneCallback { picture, textures, size }));
    }
}

struct SceneCallback {
    picture: Arc<Prepared>,
    textures: Arc<TextureData>,
    /// Target size in physical pixels.
    size: [u32; 2],
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
        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("w3d_vertices"), contents: &p.vertices, usage: wgpu::BufferUsages::VERTEX });
        let ibytes: Vec<u8> = p.indices.iter().flat_map(|i| i.to_le_bytes()).collect();
        let ibuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("w3d_indices"), contents: &ibytes, usage: wgpu::BufferUsages::INDEX });
        if res.pictures.len() >= KEEP
            && let Some(old) = res.pictures.iter().min_by_key(|(_, pic)| pic.used).map(|(k, _)| *k)
        {
            res.pictures.remove(&old);
        }
        res.pictures.insert(p.id, Picture { revision: p.revision, vbuf, ibuf, draws: p.draws.clone(), used: res.tick });
    }
    let tick = res.tick;
    if let Some(pic) = res.pictures.get_mut(&p.id) {
        pic.used = tick;
    }
}

/// Render a picture into the offscreen target: solid pass, then the blended pass.
fn draw_offscreen(res: &mut Resources, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, id: u64, size: [u32; 2]) {
    res.ensure_target(device, size);
    let Some(target) = &res.target else { return };
    if target.shows == Some(id) {
        return;
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
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0.0), store: wgpu::StoreOp::Discard }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        if let Some(pic) = res.pictures.get(&id) {
            rp.set_vertex_buffer(0, pic.vbuf.slice(..));
            rp.set_index_buffer(pic.ibuf.slice(..), wgpu::IndexFormat::Uint32);
            for pipeline in [&res.solid, &res.blend] {
                rp.set_pipeline(pipeline);
                for (tex, start, count) in &pic.draws {
                    let Some(bind) = res.bind_for(*tex) else { continue };
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
            draw_offscreen(res, device, encoder, self.picture.id, self.size);
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
        pass.set_pipeline(&res.full);
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
            focus_z: 0.0,
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
}
