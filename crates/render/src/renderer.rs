//! The renderer: pipelines, texture store, per-frame batching and passes.
//!
//! Frame structure (all colour math in linear light, premultiplied alpha):
//!
//! 0. Lighting (optional): light map with shadows at 1/`shadow_map_scale`
//!    resolution, cleared to ambient; per light a shadow mask pass then an
//!    additive light pass. With a fog layer also a light map without
//!    shadows at 1/`light_map_scale`.
//! 1. Background stage (optional, [`FrameSettings::background`]):
//!    `Background` layer into its own HDR texture, fog layer, grading ->
//!    the scene texture.
//! 2. `World` layer -> HDR scene texture (cleared to the clear colour, or on
//!    top of the background stage), with a stencil buffer for sprite masks.
//! 3. Composite: scene x light map x exposure -> HDR "lit" texture.
//! 4. `WorldUnlit` layer -> lit texture.
//! 5. Post: vignette + colour grading -> output target (sRGB).
//! 6. `Screen` layer -> output target.

use std::f32::consts::TAU;

use bytemuck::{Pod, Zeroable};
use glam::{Affine2, Mat2, Mat4, Vec2};

use crate::camera::{Camera2D, screen_proj};
use crate::color::Color;
use crate::draw::{Blend, DrawList, LAYER_COUNT, Layer, Mask, Primitive, Sprite, Text};
use crate::gpu::Gpu;
use crate::grading::{BackgroundSettings, ColorGrading, GradingParams, Vignette, fog_terms};
use crate::image::Image;
use crate::lighting::{LightInstance, LightingSettings, ShadowEdge, shadow_edges};
use crate::target::{OffscreenTarget, TargetView};
use crate::text::{
    ATLAS_SIZE, FontId, MAX_GLYPH_PX, PlacedGlyph, SDF_BASE, SdfFont, SdfFontDesc, TextSystem,
};
use crate::texture::{
    GpuTexture, TextureId, TextureOptions, sampler_desc, texture_format, write_rows,
};
use crate::{RenderError, tess};

/// Format of the intermediate scene textures.
pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const MASK_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;
const STENCIL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24PlusStencil8;

const KIND_TEXTURED: u32 = 0;
const KIND_ELLIPSE: u32 = 1;
const KIND_RING: u32 = 2;
const KIND_ROUNDED_RECT: u32 = 3;
/// Signed-distance-field glyphs (TextMeshPro atlases).
const KIND_SDF: u32 = 4;

/// Post-processing parameters. The default is the identity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PostSettings {
    /// 0 = off, 1 = full black at the corners.
    pub vignette: f32,
    /// Distance from the centre (0 centre, 1 corner) where darkening starts.
    pub vignette_inner: f32,
    /// Distance where darkening is complete.
    pub vignette_outer: f32,
    /// 1 = unchanged, 0 = greyscale.
    pub saturation: f32,
    /// 1 = unchanged (pivot at linear 0.18 grey).
    pub contrast: f32,
    /// Linear multiplier.
    pub brightness: f32,
    /// Multiplied with the image (sRGB colour).
    pub tint: Color,
    /// Unity-style HDR colour grading + tonemapping, applied before the
    /// simple hooks above.
    pub grading: Option<ColorGrading>,
    /// Unity-style classic vignette (before grading).
    pub unity_vignette: Option<Vignette>,
    /// Unity post-processing v2 bloom (before vignette and grading).
    pub bloom: Option<Bloom>,
}

/// Post-processing v2 bloom parameters, as authored in a profile.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bloom {
    /// Authored intensity (linear strength = 2^(intensity / 10) - 1).
    pub intensity: f32,
    /// Brightness threshold in gamma space.
    pub threshold: f32,
    /// 0..1 soft knee of the threshold.
    pub soft_knee: f32,
    /// 1..10: how far it spreads (more mip levels).
    pub diffusion: f32,
    /// Bloom tint.
    pub color: Color,
    /// Prefilter clamp (linear).
    pub clamp: f32,
}

impl Bloom {
    /// Linear strength (PPv2: `2^(intensity / 10) - 1`).
    pub fn strength(&self) -> f32 {
        2f32.powf(self.intensity / 10.0) - 1.0
    }

    /// Threshold in linear light (sRGB curve, extended above 1).
    pub fn threshold_linear(&self) -> f32 {
        let x = self.threshold.max(0.0);
        if x <= 0.04045 {
            x / 12.92
        } else {
            ((x + 0.055) / 1.055).powf(2.4)
        }
    }

    /// (iterations, sample scale) for a half-resolution start of `size`.
    pub fn chain(&self, size: (u32, u32)) -> (usize, f32) {
        let half = (size.0.max(size.1) / 2).max(1) as f32;
        let logs = half.log2() + self.diffusion.clamp(1.0, 10.0) - 10.0;
        let logh = logs.floor();
        (
            (logh as i32).clamp(2, BLOOM_LEVELS as i32) as usize,
            0.5 + logs - logh,
        )
    }
}

/// Maximum bloom mip levels.
const BLOOM_LEVELS: usize = 8;

impl Default for PostSettings {
    fn default() -> Self {
        Self {
            vignette: 0.0,
            vignette_inner: 0.4,
            vignette_outer: 1.2,
            saturation: 1.0,
            contrast: 1.0,
            brightness: 1.0,
            tint: Color::WHITE,
            grading: None,
            unity_vignette: None,
            bloom: None,
        }
    }
}

/// Per-frame settings that are not draw items.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameSettings {
    pub camera: Camera2D,
    /// Background colour of the world (sRGB).
    pub clear: Color,
    pub lighting: LightingSettings,
    pub post: PostSettings,
    /// Optional first stage (see [`BackgroundSettings`]).
    pub background: Option<BackgroundSettings>,
}

/// Statistics of the last rendered frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameStats {
    pub quads: u32,
    pub mesh_triangles: u32,
    pub draw_calls: u32,
    pub lights: u32,
    pub shadow_edges: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct QuadInstance {
    axes: [f32; 4],
    origin_size: [f32; 4],
    uv: [f32; 4],
    color: [f32; 4],
    params: [f32; 4],
    kind: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct MeshVertex {
    pos: [f32; 2],
    uv: [f32; 2],
    color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct FullscreenParams {
    a: [f32; 4],
    b: [f32; 4],
    tint: [f32; 4],
    g: GradingParams,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BatchKind {
    Quads,
    Mesh,
}

#[derive(Clone, Copy, Debug)]
struct Batch {
    kind: BatchKind,
    texture: TextureId,
    mask: Mask,
    /// Instance range (quads) or index range (mesh).
    start: u32,
    end: u32,
}

/// A vertex/instance/index buffer that grows on demand.
struct GrowBuffer {
    buffer: wgpu::Buffer,
    usage: wgpu::BufferUsages,
    label: &'static str,
}

impl GrowBuffer {
    fn new(device: &wgpu::Device, label: &'static str, usage: wgpu::BufferUsages) -> Self {
        let usage = usage | wgpu::BufferUsages::COPY_DST;
        Self {
            buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: 4096,
                usage,
                mapped_at_creation: false,
            }),
            usage,
            label,
        }
    }

    fn write(&mut self, gpu: &Gpu, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        let len = data.len() as u64;
        if len > self.buffer.size() {
            self.buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(self.label),
                size: len.next_power_of_two(),
                usage: self.usage,
                mapped_at_creation: false,
            });
        }
        gpu.queue.write_buffer(&self.buffer, 0, data);
    }
}

/// Size-dependent intermediate textures.
struct Intermediates {
    size: (u32, u32),
    /// Light map without shadows.
    light_size: (u32, u32),
    /// Light map with shadows (and the shadow mask).
    shadow_size: (u32, u32),
    scene_view: wgpu::TextureView,
    lit_view: wgpu::TextureView,
    bg_view: wgpu::TextureView,
    stencil_view: wgpu::TextureView,
    light_map_view: wgpu::TextureView,
    shadow_map_view: wgpu::TextureView,
    mask_view: wgpu::TextureView,
    composite_bg: wgpu::BindGroup,
    post_bg: wgpu::BindGroup,
    fog_bg: wgpu::BindGroup,
    grade_bg: wgpu::BindGroup,
    mask_bg: wgpu::BindGroup,
    /// Bloom chain: per level (size, down view, up view, prefilter/down
    /// bind group, up bind group).
    bloom: Vec<BloomLevel>,
}

struct BloomLevel {
    down_view: wgpu::TextureView,
    up_view: wgpu::TextureView,
    /// Reads the previous (bigger) level's down texture (level 0: the lit
    /// scene).
    down_bg: wgpu::BindGroup,
    /// Reads the next (smaller) level's up texture plus this level's down
    /// texture.
    up_bg: wgpu::BindGroup,
    /// The same with the next level's *down* texture (when that level is
    /// the smallest one used this frame).
    up_from_down_bg: wgpu::BindGroup,
}

struct Pipelines {
    quad_hdr: wgpu::RenderPipeline,
    mesh_hdr: wgpu::RenderPipeline,
    /// World pass (with stencil): by [`Mask`] mode.
    quad_world: [wgpu::RenderPipeline; 6],
    mesh_world: wgpu::RenderPipeline,
    fog: wgpu::RenderPipeline,
    grade: wgpu::RenderPipeline,
    quad_out: wgpu::RenderPipeline,
    mesh_out: wgpu::RenderPipeline,
    shadow: wgpu::RenderPipeline,
    light: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    post: wgpu::RenderPipeline,
    bloom_prefilter: wgpu::RenderPipeline,
    bloom_down: wgpu::RenderPipeline,
    bloom_up: wgpu::RenderPipeline,
}

/// The 2D renderer. One per output format (window or offscreen).
pub struct Renderer {
    output_format: wgpu::TextureFormat,
    pipelines: Pipelines,
    texture_bgl: wgpu::BindGroupLayout,
    mask_bgl: wgpu::BindGroupLayout,
    fullscreen_bgl: wgpu::BindGroupLayout,
    globals_buf: wgpu::Buffer,
    globals_bg: wgpu::BindGroup,
    globals_stride: u32,
    composite_params: wgpu::Buffer,
    post_params: wgpu::Buffer,
    bloom_params: wgpu::Buffer,
    fog_params: wgpu::Buffer,
    grade_params: wgpu::Buffer,
    linear_sampler: wgpu::Sampler,
    textures: Vec<Option<GpuTexture>>,
    text: TextSystem,
    sdf_fonts: Vec<SdfFont>,
    /// Minimum shadow-map divisor (`ROUNDS_SHADOW_SCALE`, default 1):
    /// a cheaper shadow mask for slow software rasterisers.
    min_shadow_scale: u32,
    /// `ROUNDS_NO_BLOOM`: skip bloom (comparisons, slow software GPUs).
    no_bloom: bool,
    instance_buf: GrowBuffer,
    vertex_buf: GrowBuffer,
    index_buf: GrowBuffer,
    shadow_buf: GrowBuffer,
    light_buf: GrowBuffer,
    targets: Option<Intermediates>,
    // Per-frame scratch, kept to reuse allocations.
    instances: Vec<QuadInstance>,
    vertices: Vec<MeshVertex>,
    indices: Vec<u32>,
    batches: [Vec<Batch>; LAYER_COUNT],
    order: Vec<usize>,
    glyphs: Vec<PlacedGlyph>,
    edges: Vec<ShadowEdge>,
    light_instances: Vec<LightInstance>,
    /// (edge range, has shadows, cookie texture) per light, for the light
    /// map with shadows; the instances of the light map without shadows
    /// follow at `light_jobs.len()..`.
    light_jobs: Vec<(u32, u32, bool, TextureId)>,
    /// Whether the light map without shadows is built this frame.
    plain_lights: bool,
    stats: FrameStats,
}

fn shader(device: &wgpu::Device, label: &str, src: &str) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(src.into()),
    })
}

const PREMUL_BLEND: wgpu::BlendState = wgpu::BlendState {
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

const ADD_BLEND: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    },
};

struct PipelineSpec<'a> {
    label: &'a str,
    layout: &'a wgpu::PipelineLayout,
    module: &'a wgpu::ShaderModule,
    fs_entry: &'a str,
    buffers: &'a [Option<wgpu::VertexBufferLayout<'a>>],
    topology: wgpu::PrimitiveTopology,
    format: wgpu::TextureFormat,
    blend: Option<wgpu::BlendState>,
    depth_stencil: Option<wgpu::DepthStencilState>,
    write_mask: wgpu::ColorWrites,
}

/// Stencil state for the world pass by mask mode (reference value 1).
fn stencil_state(mask: Mask) -> wgpu::DepthStencilState {
    let (compare, pass_op) = match mask {
        Mask::None => (wgpu::CompareFunction::Always, wgpu::StencilOperation::Keep),
        Mask::Write => (
            wgpu::CompareFunction::Always,
            wgpu::StencilOperation::Replace,
        ),
        // Unity: visible inside = `1 <= stencil`, outside = `1 > stencil`.
        Mask::VisibleInside => (
            wgpu::CompareFunction::LessEqual,
            wgpu::StencilOperation::Keep,
        ),
        Mask::VisibleOutside => (wgpu::CompareFunction::Greater, wgpu::StencilOperation::Keep),
        Mask::InsideValue(_) => (wgpu::CompareFunction::Equal, wgpu::StencilOperation::Keep),
        Mask::WriteValue(_) => (
            wgpu::CompareFunction::Always,
            wgpu::StencilOperation::Replace,
        ),
    };
    let face = wgpu::StencilFaceState {
        compare,
        fail_op: wgpu::StencilOperation::Keep,
        depth_fail_op: wgpu::StencilOperation::Keep,
        pass_op,
    };
    wgpu::DepthStencilState {
        format: STENCIL_FORMAT,
        depth_write_enabled: Some(false),
        depth_compare: Some(wgpu::CompareFunction::Always),
        stencil: wgpu::StencilState {
            front: face,
            back: face,
            read_mask: 0xff,
            write_mask: 0xff,
        },
        bias: Default::default(),
    }
}

fn pipeline(device: &wgpu::Device, s: PipelineSpec<'_>) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(s.label),
        layout: Some(s.layout),
        vertex: wgpu::VertexState {
            module: s.module,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: s.buffers,
        },
        primitive: wgpu::PrimitiveState {
            topology: s.topology,
            ..Default::default()
        },
        depth_stencil: s.depth_stencil,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: s.module,
            entry_point: Some(s.fs_entry),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: s.format,
                blend: s.blend,
                write_mask: s.write_mask,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn tex_entry(binding: u32, filterable: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

fn uniform_entry(
    binding: u32,
    dynamic: bool,
    visibility: wgpu::ShaderStages,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: dynamic,
            min_binding_size: None,
        },
        count: None,
    }
}

impl Renderer {
    /// Creates a renderer drawing into targets of `output_format` (the
    /// surface format, or [`OFFSCREEN_FORMAT`](crate::OFFSCREEN_FORMAT)).
    pub fn new(gpu: &Gpu, output_format: wgpu::TextureFormat) -> Self {
        let device = &gpu.device;
        let globals_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals"),
            entries: &[uniform_entry(0, true, wgpu::ShaderStages::VERTEX)],
        });
        let texture_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("texture"),
            entries: &[tex_entry(0, true), sampler_entry(1)],
        });
        let mask_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow mask"),
            entries: &[tex_entry(0, false)],
        });
        let fullscreen_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fullscreen"),
            entries: &[
                tex_entry(0, true),
                tex_entry(1, true),
                sampler_entry(2),
                uniform_entry(3, false, wgpu::ShaderStages::FRAGMENT),
                tex_entry(4, true),
            ],
        });

        let layout = |label, bgls: &[Option<&wgpu::BindGroupLayout>]| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: bgls,
                immediate_size: 0,
            })
        };
        let draw_layout = layout("draw", &[Some(&globals_bgl), Some(&texture_bgl)]);
        let shadow_layout = layout("shadow", &[Some(&globals_bgl)]);
        let light_layout = layout(
            "light",
            &[Some(&globals_bgl), Some(&mask_bgl), Some(&texture_bgl)],
        );
        let fullscreen_layout = layout("fullscreen", &[Some(&fullscreen_bgl)]);

        let quad_sh = shader(device, "quad.wgsl", include_str!("shaders/quad.wgsl"));
        let mesh_sh = shader(device, "mesh.wgsl", include_str!("shaders/mesh.wgsl"));
        let shadow_sh = shader(device, "shadow.wgsl", include_str!("shaders/shadow.wgsl"));
        let light_sh = shader(device, "light.wgsl", include_str!("shaders/light.wgsl"));
        let full_sh = shader(
            device,
            "fullscreen.wgsl",
            include_str!("shaders/fullscreen.wgsl"),
        );

        let quad_attrs = wgpu::vertex_attr_array![
            0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4, 5 => Uint32
        ];
        let quad_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<QuadInstance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &quad_attrs,
        })];
        let mesh_attrs = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4];
        let mesh_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<MeshVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &mesh_attrs,
        })];
        let pair_attrs = wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4];
        let shadow_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<ShadowEdge>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &pair_attrs,
        })];
        let light_attrs = wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4];
        let light_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<LightInstance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &light_attrs,
        })];

        let quad_ds = |format, label, ds: Option<wgpu::DepthStencilState>, colour: bool| {
            pipeline(
                device,
                PipelineSpec {
                    label,
                    layout: &draw_layout,
                    module: &quad_sh,
                    fs_entry: "fs_main",
                    buffers: &quad_buffers,
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    format,
                    blend: Some(PREMUL_BLEND),
                    depth_stencil: ds,
                    write_mask: if colour {
                        wgpu::ColorWrites::ALL
                    } else {
                        wgpu::ColorWrites::empty()
                    },
                },
            )
        };
        let quad = |format, label| quad_ds(format, label, None, true);
        let mesh_ds = |format, label, ds: Option<wgpu::DepthStencilState>| {
            pipeline(
                device,
                PipelineSpec {
                    label,
                    layout: &draw_layout,
                    module: &mesh_sh,
                    fs_entry: "fs_main",
                    buffers: &mesh_buffers,
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    format,
                    blend: Some(PREMUL_BLEND),
                    depth_stencil: ds,
                    write_mask: wgpu::ColorWrites::ALL,
                },
            )
        };
        let mesh = |format, label| mesh_ds(format, label, None);
        let fullscreen = |label, fs_entry, format, blend| {
            pipeline(
                device,
                PipelineSpec {
                    label,
                    layout: &fullscreen_layout,
                    module: &full_sh,
                    fs_entry,
                    buffers: &[],
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    format,
                    blend,
                    depth_stencil: None,
                    write_mask: wgpu::ColorWrites::ALL,
                },
            )
        };
        let pipelines = Pipelines {
            quad_hdr: quad(HDR_FORMAT, "quads (hdr)"),
            mesh_hdr: mesh(HDR_FORMAT, "mesh (hdr)"),
            quad_world: [
                Mask::None,
                Mask::Write,
                Mask::VisibleInside,
                Mask::VisibleOutside,
                Mask::InsideValue(2),
                Mask::WriteValue(2),
            ]
            .map(|m| {
                quad_ds(
                    HDR_FORMAT,
                    "quads (world)",
                    Some(stencil_state(m)),
                    m != Mask::Write,
                )
            }),
            mesh_world: mesh_ds(HDR_FORMAT, "mesh (world)", Some(stencil_state(Mask::None))),
            fog: fullscreen("fog", "fs_fog", HDR_FORMAT, Some(PREMUL_BLEND)),
            grade: fullscreen("background grade", "fs_grade", HDR_FORMAT, None),
            quad_out: quad(output_format, "quads (out)"),
            mesh_out: mesh(output_format, "mesh (out)"),
            shadow: pipeline(
                device,
                PipelineSpec {
                    label: "shadow mask",
                    layout: &shadow_layout,
                    module: &shadow_sh,
                    fs_entry: "fs_main",
                    buffers: &shadow_buffers,
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    format: MASK_FORMAT,
                    blend: Some(ADD_BLEND),
                    depth_stencil: None,
                    write_mask: wgpu::ColorWrites::ALL,
                },
            ),
            light: pipeline(
                device,
                PipelineSpec {
                    label: "light",
                    layout: &light_layout,
                    module: &light_sh,
                    fs_entry: "fs_main",
                    buffers: &light_buffers,
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    format: HDR_FORMAT,
                    blend: Some(ADD_BLEND),
                    depth_stencil: None,
                    write_mask: wgpu::ColorWrites::ALL,
                },
            ),
            composite: fullscreen("composite", "fs_composite", HDR_FORMAT, None),
            post: fullscreen("post", "fs_post", output_format, None),
            bloom_prefilter: fullscreen("bloom prefilter", "fs_bloom_prefilter", HDR_FORMAT, None),
            bloom_down: fullscreen("bloom down", "fs_bloom_down", HDR_FORMAT, None),
            bloom_up: fullscreen("bloom up", "fs_bloom_up", HDR_FORMAT, None),
        };

        let align = device.limits().min_uniform_buffer_offset_alignment.max(64);
        let globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: (align * 2) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals"),
            layout: &globals_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &globals_buf,
                    offset: 0,
                    size: wgpu::BufferSize::new(64),
                }),
            }],
        });
        let params = |label| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: std::mem::size_of::<FullscreenParams>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let composite_params = params("composite params");
        let post_params = params("post params");
        let bloom_params = params("bloom params");
        let fog_params = params("fog params");
        let grade_params = params("grade params");
        let linear_sampler = device.create_sampler(&sampler_desc(&TextureOptions::default()));

        let mut r = Self {
            output_format,
            pipelines,
            texture_bgl,
            mask_bgl,
            fullscreen_bgl,
            globals_buf,
            globals_bg,
            globals_stride: align,
            composite_params,
            post_params,
            bloom_params,
            fog_params,
            grade_params,
            linear_sampler,
            textures: Vec::new(),
            text: TextSystem::new(),
            sdf_fonts: Vec::new(),
            no_bloom: std::env::var_os("ROUNDS_NO_BLOOM").is_some(),
            min_shadow_scale: std::env::var("ROUNDS_SHADOW_SCALE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1u32)
                .max(1),
            instance_buf: GrowBuffer::new(device, "quad instances", wgpu::BufferUsages::VERTEX),
            vertex_buf: GrowBuffer::new(device, "mesh vertices", wgpu::BufferUsages::VERTEX),
            index_buf: GrowBuffer::new(device, "mesh indices", wgpu::BufferUsages::INDEX),
            shadow_buf: GrowBuffer::new(device, "shadow edges", wgpu::BufferUsages::VERTEX),
            light_buf: GrowBuffer::new(device, "lights", wgpu::BufferUsages::VERTEX),
            targets: None,
            instances: Vec::new(),
            vertices: Vec::new(),
            indices: Vec::new(),
            batches: Default::default(),
            order: Vec::new(),
            glyphs: Vec::new(),
            edges: Vec::new(),
            light_instances: Vec::new(),
            light_jobs: Vec::new(),
            plain_lights: false,
            stats: FrameStats::default(),
        };
        let white = r.create_texture(
            gpu,
            &Image::solid(1, 1, [255; 4]),
            TextureOptions::default(),
        );
        debug_assert_eq!(white, TextureId::WHITE);
        let atlas = Image::from_pixels(ATLAS_SIZE, ATLAS_SIZE, r.text.atlas.clone());
        let glyphs = r.create_texture(gpu, &atlas, TextureOptions::default());
        debug_assert_eq!(glyphs, TextureId::GLYPHS);
        r.text.dirty_rows = None;
        r
    }

    pub fn output_format(&self) -> wgpu::TextureFormat {
        self.output_format
    }

    /// Statistics of the last [`render`](Self::render).
    pub fn stats(&self) -> FrameStats {
        self.stats
    }

    // ---- textures -------------------------------------------------------

    fn texture_bind_group(
        &self,
        gpu: &Gpu,
        view: &wgpu::TextureView,
        opts: &TextureOptions,
    ) -> wgpu::BindGroup {
        let sampler = gpu.device.create_sampler(&sampler_desc(opts));
        gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("texture"),
            layout: &self.texture_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        })
    }

    fn insert_texture(&mut self, tex: GpuTexture) -> TextureId {
        if let Some(i) = self.textures.iter().skip(2).position(Option::is_none) {
            self.textures[i + 2] = Some(tex);
            TextureId(i as u32 + 2)
        } else {
            self.textures.push(Some(tex));
            TextureId(self.textures.len() as u32 - 1)
        }
    }

    /// Uploads an RGBA8 image as a texture.
    pub fn create_texture(&mut self, gpu: &Gpu, img: &Image, opts: TextureOptions) -> TextureId {
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("texture"),
            size: wgpu::Extent3d {
                width: img.width,
                height: img.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: texture_format(&opts),
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        write_rows(
            &gpu.queue,
            &texture,
            &img.pixels,
            img.width,
            (0, img.height),
        );
        let view = texture.create_view(&Default::default());
        let bind_group = self.texture_bind_group(gpu, &view, &opts);
        self.insert_texture(GpuTexture {
            texture,
            bind_group,
            size: (img.width, img.height),
        })
    }

    /// Replaces the contents of a texture created by
    /// [`create_texture`](Self::create_texture) (same size).
    pub fn update_texture(
        &mut self,
        gpu: &Gpu,
        id: TextureId,
        img: &Image,
    ) -> Result<(), RenderError> {
        let t = self
            .textures
            .get(id.0 as usize)
            .and_then(Option::as_ref)
            .ok_or(RenderError::BadTexture)?;
        if t.size != (img.width, img.height) {
            return Err(RenderError::Unsupported(
                "update_texture with a different size".into(),
            ));
        }
        write_rows(
            &gpu.queue,
            &t.texture,
            &img.pixels,
            img.width,
            (0, img.height),
        );
        Ok(())
    }

    /// Makes an offscreen target usable as a sprite texture (render
    /// textures). The target must outlive its use in draw lists.
    pub fn texture_from_target(
        &mut self,
        gpu: &Gpu,
        target: &OffscreenTarget,
        opts: TextureOptions,
    ) -> TextureId {
        let bind_group = self.texture_bind_group(gpu, &target.view, &opts);
        self.insert_texture(GpuTexture {
            texture: target.texture.clone(),
            bind_group,
            size: target.size,
        })
    }

    /// Frees a texture; the id may be reused.
    pub fn destroy_texture(&mut self, id: TextureId) {
        if id.0 >= 2
            && let Some(slot) = self.textures.get_mut(id.0 as usize)
        {
            *slot = None;
        }
    }

    pub fn texture_size(&self, id: TextureId) -> Option<(u32, u32)> {
        self.textures
            .get(id.0 as usize)
            .and_then(Option::as_ref)
            .map(|t| t.size)
    }

    // ---- fonts ----------------------------------------------------------

    /// Loads a TTF/OTF font.
    pub fn load_font(&mut self, bytes: &[u8]) -> Result<FontId, RenderError> {
        self.text.add_font(bytes)
    }

    /// Adds face `index` of a font file that may be a collection (`.ttc`).
    pub fn load_font_indexed(&mut self, bytes: &[u8], index: u32) -> Result<FontId, RenderError> {
        self.text.add_font_indexed(bytes, index)
    }

    /// Whether raster font `font` has a glyph for `ch`.
    pub fn font_has_char(&self, font: FontId, ch: char) -> bool {
        self.text.has_char(font, ch)
    }

    /// Fonts used, in order, for characters a requested raster font lacks
    /// (e.g. a system CJK font behind the embedded Latin one).
    pub fn set_fallback_fonts(&mut self, fonts: &[FontId]) {
        self.text.fallbacks = fonts.to_vec();
    }

    /// Adds a signed-distance-field font (e.g. a TextMeshPro font asset).
    pub fn add_sdf_font(&mut self, desc: SdfFontDesc) -> FontId {
        self.sdf_fonts.push(SdfFont::new(desc));
        FontId(SDF_BASE + self.sdf_fonts.len() as u32 - 1)
    }

    fn sdf_font(&self, font: FontId) -> Option<&SdfFont> {
        if font.is_sdf() {
            self.sdf_fonts.get((font.0 - SDF_BASE) as usize)
        } else {
            None
        }
    }

    /// Size of a text block at `size` (same units as [`Text::size`]).
    pub fn measure_text(&self, font: FontId, text: &str, size: f32) -> Vec2 {
        if let Some(f) = self.sdf_font(font) {
            return f.layout(text, size, 0.0).1;
        }
        // Measure at a fixed resolution and scale, so the answer does not
        // depend on the camera.
        let px = 64.0;
        self.text.measure(font, text, px) * (size / px)
    }

    // ---- frame ----------------------------------------------------------

    fn ensure_targets(&mut self, gpu: &Gpu, size: (u32, u32), light_scale: u32, shadow_scale: u32) {
        let scaled = |s: u32| {
            let s = s.max(1);
            (size.0.div_ceil(s).max(1), size.1.div_ceil(s).max(1))
        };
        let light_size = scaled(light_scale);
        let shadow_size = scaled(shadow_scale);
        if let Some(t) = &self.targets
            && t.size == size
            && t.light_size == light_size
            && t.shadow_size == shadow_size
        {
            return;
        }
        let device = &gpu.device;
        let make = |label, size: (u32, u32), format| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: size.0,
                        height: size.1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let scene_view = make("scene", size, HDR_FORMAT);
        let lit_view = make("lit", size, HDR_FORMAT);
        let bg_view = make("background", size, HDR_FORMAT);
        let stencil_view = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("stencil"),
                size: wgpu::Extent3d {
                    width: size.0,
                    height: size.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: STENCIL_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let light_map_view = make("light map", light_size, HDR_FORMAT);
        let shadow_map_view = make("light map (shadows)", shadow_size, HDR_FORMAT);
        let mask_view = make("shadow mask", shadow_size, MASK_FORMAT);
        let fullscreen = |src: &wgpu::TextureView,
                          t1: &wgpu::TextureView,
                          t4: &wgpu::TextureView,
                          params: &wgpu::Buffer| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("fullscreen"),
                layout: &self.fullscreen_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(src),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(t1),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.linear_sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: params.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::TextureView(t4),
                    },
                ],
            })
        };
        // Bloom chain from half resolution down.
        let mut bloom_tex: Vec<((u32, u32), wgpu::TextureView, wgpu::TextureView)> = Vec::new();
        let mut bs = ((size.0 / 2).max(1), (size.1 / 2).max(1));
        for _ in 0..BLOOM_LEVELS {
            bloom_tex.push((
                bs,
                make("bloom down", bs, HDR_FORMAT),
                make("bloom up", bs, HDR_FORMAT),
            ));
            bs = ((bs.0 / 2).max(1), (bs.1 / 2).max(1));
        }
        let mut bloom = Vec::new();
        for i in 0..bloom_tex.len() {
            let src = if i == 0 {
                &lit_view
            } else {
                &bloom_tex[i - 1].1
            };
            let down_bg = fullscreen(src, src, src, &self.bloom_params);
            let j = (i + 1).min(bloom_tex.len() - 1);
            let up_bg = fullscreen(&bloom_tex[j].2, &bloom_tex[i].1, src, &self.bloom_params);
            let up_from_down_bg =
                fullscreen(&bloom_tex[j].1, &bloom_tex[i].1, src, &self.bloom_params);
            bloom.push((down_bg, up_bg, up_from_down_bg));
        }
        let bloom: Vec<BloomLevel> = bloom_tex
            .into_iter()
            .zip(bloom)
            .map(
                |((_, down_view, up_view), (down_bg, up_bg, up_from_down_bg))| BloomLevel {
                    down_view,
                    up_view,
                    down_bg,
                    up_bg,
                    up_from_down_bg,
                },
            )
            .collect();
        let composite_bg = fullscreen(
            &scene_view,
            &shadow_map_view,
            &shadow_map_view,
            &self.composite_params,
        );
        let post_bg = fullscreen(
            &lit_view,
            &shadow_map_view,
            &bloom[0].up_view,
            &self.post_params,
        );
        let fog_bg = fullscreen(
            &lit_view,
            &light_map_view,
            &shadow_map_view,
            &self.fog_params,
        );
        let grade_bg = fullscreen(
            &bg_view,
            &shadow_map_view,
            &shadow_map_view,
            &self.grade_params,
        );
        let mask_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mask"),
            layout: &self.mask_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&mask_view),
            }],
        });
        self.targets = Some(Intermediates {
            size,
            light_size,
            shadow_size,
            scene_view,
            lit_view,
            bg_view,
            stencil_view,
            light_map_view,
            shadow_map_view,
            mask_view,
            composite_bg,
            post_bg,
            fog_bg,
            grade_bg,
            mask_bg,
            bloom,
        });
    }

    /// Renders `list` into `target`. `target.format` must equal the
    /// renderer's output format.
    pub fn render(
        &mut self,
        gpu: &Gpu,
        target: TargetView<'_>,
        frame: &FrameSettings,
        list: &DrawList,
    ) {
        assert_eq!(
            target.format, self.output_format,
            "target format differs from renderer output format"
        );
        let size = (target.size.0.max(1), target.size.1.max(1));
        let ls = &frame.lighting;
        let shadow_scale = if ls.shadow_map_scale == 0 {
            ls.light_map_scale
        } else {
            ls.shadow_map_scale
        }
        .max(self.min_shadow_scale);
        self.ensure_targets(gpu, size, ls.light_map_scale, shadow_scale);
        self.text.begin_frame();
        self.stats = FrameStats::default();
        let aspect = size.0 as f32 / size.1 as f32;

        // Build geometry for every layer.
        self.instances.clear();
        self.vertices.clear();
        self.indices.clear();
        let ppu = frame.camera.pixels_per_unit(size.1);
        for layer in [
            Layer::Background,
            Layer::World,
            Layer::WorldUnlit,
            Layer::Screen,
        ] {
            self.build_layer(list, layer, ppu);
        }
        if let Some(rows) = self.text.dirty_rows.take() {
            let atlas = &self.textures[TextureId::GLYPHS.0 as usize]
                .as_ref()
                .unwrap()
                .texture;
            write_rows(&gpu.queue, atlas, &self.text.atlas, ATLAS_SIZE, rows);
        }
        let lighting = ls.enabled;
        let fog = frame.background.as_ref().and_then(|b| b.fog);
        self.edges.clear();
        self.light_instances.clear();
        self.light_jobs.clear();
        self.plain_lights = lighting && fog.is_some();
        if lighting {
            self.build_lights(list);
        }

        // Uploads.
        self.instance_buf
            .write(gpu, bytemuck::cast_slice(&self.instances));
        self.vertex_buf
            .write(gpu, bytemuck::cast_slice(&self.vertices));
        self.index_buf
            .write(gpu, bytemuck::cast_slice(&self.indices));
        self.shadow_buf
            .write(gpu, bytemuck::cast_slice(&self.edges));
        self.light_buf
            .write(gpu, bytemuck::cast_slice(&self.light_instances));
        let world_vp = frame.camera.view_proj(aspect);
        let screen_vp = screen_proj(size.0, size.1);
        let write_mat = |slot: u32, m: Mat4| {
            gpu.queue.write_buffer(
                &self.globals_buf,
                (slot * self.globals_stride) as u64,
                bytemuck::cast_slice(&m.to_cols_array()),
            );
        };
        write_mat(0, world_vp);
        write_mat(1, screen_vp);
        let write_params = |buf: &wgpu::Buffer, p: FullscreenParams| {
            gpu.queue.write_buffer(buf, 0, bytemuck::bytes_of(&p));
        };
        let multiply = lighting && ls.multiply_world;
        write_params(
            &self.composite_params,
            FullscreenParams {
                a: [ls.exposure, if multiply { 1.0 } else { 0.0 }, 0.0, 0.0],
                b: [0.0; 4],
                tint: [1.0; 4],
                g: GradingParams::default(),
            },
        );
        let p = &frame.post;
        let bloom = p.bloom.filter(|b| b.intensity > 0.0 && !self.no_bloom);
        write_params(
            &self.post_params,
            FullscreenParams {
                a: [p.vignette, p.vignette_inner, p.vignette_outer, p.saturation],
                b: [
                    p.contrast,
                    p.brightness,
                    bloom.map_or(0.0, |b| b.strength()),
                    0.0,
                ],
                tint: p.tint.to_linear(),
                g: GradingParams::new(p.grading.as_ref(), p.unity_vignette.as_ref(), aspect),
            },
        );
        if let Some(bg) = &frame.background {
            if let Some(f) = &bg.fog {
                let (fc, sc) = fog_terms(f);
                write_params(
                    &self.fog_params,
                    FullscreenParams {
                        a: fc,
                        b: sc,
                        tint: [f.exposure, 0.0, 0.0, 0.0],
                        g: GradingParams::default(),
                    },
                );
            }
            write_params(
                &self.grade_params,
                FullscreenParams {
                    a: [0.0; 4],
                    b: [0.0; 4],
                    tint: [1.0; 4],
                    g: GradingParams::new(bg.grading.as_ref(), None, aspect),
                },
            );
        }

        let bloom_chain = bloom.map(|b| b.chain(size));
        if let (Some(b), Some((_, scale))) = (bloom, bloom_chain) {
            let t = b.threshold_linear();
            let knee = t * b.soft_knee.clamp(0.0, 1.0) + 1e-5;
            let c = b.color.to_linear();
            write_params(
                &self.bloom_params,
                FullscreenParams {
                    a: [t - knee, knee * 2.0, 0.25 / knee, t],
                    b: [scale, b.clamp.max(1.0), 0.0, 0.0],
                    tint: [c[0], c[1], c[2], 1.0],
                    g: GradingParams::default(),
                },
            );
        }
        let mut enc = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
        let t = self.targets.as_ref().unwrap();
        let world_off = 0;
        let screen_off = self.globals_stride;

        // 0. Lighting.
        if lighting {
            let a = ls.ambient.to_linear();
            let ambient = Some([a[0], a[1], a[2], 1.0]);
            drop(begin_pass(
                &mut enc,
                "shadow light map clear",
                &t.shadow_map_view,
                ambient,
            ));
            for (li, &(e0, e1, shadows, cookie)) in self.light_jobs.iter().enumerate() {
                if shadows {
                    let mut pass =
                        begin_pass(&mut enc, "shadow mask", &t.mask_view, Some([0.0; 4]));
                    if e1 > e0 {
                        pass.set_pipeline(&self.pipelines.shadow);
                        pass.set_bind_group(0, &self.globals_bg, &[world_off]);
                        pass.set_vertex_buffer(0, self.shadow_buf.buffer.slice(..));
                        pass.draw(0..6, e0..e1);
                    }
                }
                let mut pass = begin_pass(&mut enc, "light", &t.shadow_map_view, None);
                self.draw_light(&mut pass, t, li as u32, cookie, world_off);
            }
            if self.plain_lights {
                let mut pass = begin_pass(&mut enc, "light map", &t.light_map_view, ambient);
                let n = self.light_jobs.len() as u32;
                for li in 0..n {
                    let cookie = self.light_jobs[li as usize].3;
                    self.draw_light(&mut pass, t, n + li, cookie, world_off);
                }
            }
        }
        // 1. Background stage.
        let world_clear = if let Some(bg) = &frame.background {
            {
                let clear = bg.clear.to_linear();
                let mut pass = begin_pass(&mut enc, "background", &t.bg_view, Some(clear));
                self.draw_batches(&mut pass, Layer::Background, world_off, true);
            }
            if bg.fog.is_some() && lighting {
                let mut pass = begin_pass(&mut enc, "fog", &t.bg_view, None);
                pass.set_pipeline(&self.pipelines.fog);
                pass.set_bind_group(0, &t.fog_bg, &[]);
                pass.draw(0..3, 0..1);
            }
            {
                let mut pass = begin_pass(&mut enc, "background grade", &t.scene_view, None);
                pass.set_pipeline(&self.pipelines.grade);
                pass.set_bind_group(0, &t.grade_bg, &[]);
                pass.draw(0..3, 0..1);
            }
            None
        } else {
            Some(frame.clear.to_linear_premul())
        };
        // 2. World layer (with the stencil buffer for sprite masks).
        {
            let load = match world_clear {
                Some(c) => wgpu::LoadOp::Clear(wgpu::Color {
                    r: c[0] as f64,
                    g: c[1] as f64,
                    b: c[2] as f64,
                    a: c[3] as f64,
                }),
                None => wgpu::LoadOp::Load,
            };
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("world"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &t.scene_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &t.stencil_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0),
                        store: wgpu::StoreOp::Discard,
                    }),
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_stencil_reference(1);
            self.draw_world_batches(&mut pass, world_off);
        }
        // 3. Composite.
        {
            let mut pass = begin_pass(&mut enc, "composite", &t.lit_view, Some([0.0; 4]));
            pass.set_pipeline(&self.pipelines.composite);
            pass.set_bind_group(0, &t.composite_bg, &[]);
            pass.draw(0..3, 0..1);
        }
        // 4. Unlit world layer.
        if !self.batches[Layer::WorldUnlit as usize].is_empty() {
            let mut pass = begin_pass(&mut enc, "world unlit", &t.lit_view, None);
            self.draw_batches(&mut pass, Layer::WorldUnlit, world_off, true);
        }
        // 5a. Bloom (post-processing v2): prefilter + box down chain, then
        // box upsampling back up, each level added to the next bigger one.
        if let Some((n, _)) = bloom_chain {
            for i in 0..n {
                let l = &t.bloom[i];
                let mut pass = begin_pass(&mut enc, "bloom down", &l.down_view, None);
                pass.set_pipeline(if i == 0 {
                    &self.pipelines.bloom_prefilter
                } else {
                    &self.pipelines.bloom_down
                });
                pass.set_bind_group(0, &l.down_bg, &[]);
                pass.draw(0..3, 0..1);
            }
            for i in (0..n - 1).rev() {
                let l = &t.bloom[i];
                let mut pass = begin_pass(&mut enc, "bloom up", &l.up_view, None);
                pass.set_pipeline(&self.pipelines.bloom_up);
                // The smallest level used feeds its down texture directly.
                pass.set_bind_group(
                    0,
                    if i + 2 == n {
                        &l.up_from_down_bg
                    } else {
                        &l.up_bg
                    },
                    &[],
                );
                pass.draw(0..3, 0..1);
            }
        }
        // 5. Post into the output, 6. screen layer on top.
        {
            let mut pass = begin_pass(
                &mut enc,
                "post + screen",
                target.view,
                Some([0.0, 0.0, 0.0, 1.0]),
            );
            pass.set_pipeline(&self.pipelines.post);
            pass.set_bind_group(0, &t.post_bg, &[]);
            pass.draw(0..3, 0..1);
            self.draw_batches(&mut pass, Layer::Screen, screen_off, false);
        }
        gpu.queue.submit([enc.finish()]);

        self.stats.quads = self.instances.len() as u32;
        self.stats.mesh_triangles = self.indices.len() as u32 / 3;
        self.stats.draw_calls = self.batches.iter().map(|b| b.len() as u32).sum::<u32>() + 3;
        self.stats.lights = self.light_jobs.len() as u32;
        self.stats.shadow_edges = self.edges.len() as u32;
    }

    fn draw_light(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        t: &Intermediates,
        instance: u32,
        cookie: TextureId,
        world_off: u32,
    ) {
        let tex = self
            .textures
            .get(cookie.0 as usize)
            .and_then(Option::as_ref)
            .unwrap_or_else(|| self.textures[0].as_ref().unwrap());
        pass.set_pipeline(&self.pipelines.light);
        pass.set_bind_group(0, &self.globals_bg, &[world_off]);
        pass.set_bind_group(1, &t.mask_bg, &[]);
        pass.set_bind_group(2, &tex.bind_group, &[]);
        pass.set_vertex_buffer(0, self.light_buf.buffer.slice(..));
        pass.draw(0..4, instance..instance + 1);
    }

    fn texture_or_white(&self, id: TextureId) -> &GpuTexture {
        self.textures
            .get(id.0 as usize)
            .and_then(Option::as_ref)
            .unwrap_or_else(|| self.textures[0].as_ref().unwrap())
    }

    /// The world layer, with stencil pipelines chosen per batch mask mode.
    fn draw_world_batches(&self, pass: &mut wgpu::RenderPass<'_>, globals_off: u32) {
        let batches = &self.batches[Layer::World as usize];
        if batches.is_empty() {
            return;
        }
        pass.set_bind_group(0, &self.globals_bg, &[globals_off]);
        for b in batches {
            match b.kind {
                BatchKind::Quads => {
                    pass.set_pipeline(&self.pipelines.quad_world[b.mask.pipeline_index()]);
                    pass.set_stencil_reference(b.mask.reference());
                    pass.set_vertex_buffer(0, self.instance_buf.buffer.slice(..));
                }
                BatchKind::Mesh => {
                    pass.set_pipeline(&self.pipelines.mesh_world);
                    pass.set_vertex_buffer(0, self.vertex_buf.buffer.slice(..));
                    pass.set_index_buffer(
                        self.index_buf.buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                }
            }
            pass.set_bind_group(1, &self.texture_or_white(b.texture).bind_group, &[]);
            match b.kind {
                BatchKind::Quads => pass.draw(0..4, b.start..b.end),
                BatchKind::Mesh => pass.draw_indexed(b.start..b.end, 0, 0..1),
            }
        }
    }

    fn draw_batches(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        layer: Layer,
        globals_off: u32,
        hdr: bool,
    ) {
        let batches = &self.batches[layer as usize];
        if batches.is_empty() {
            return;
        }
        let (quad, mesh) = if hdr {
            (&self.pipelines.quad_hdr, &self.pipelines.mesh_hdr)
        } else {
            (&self.pipelines.quad_out, &self.pipelines.mesh_out)
        };
        pass.set_bind_group(0, &self.globals_bg, &[globals_off]);
        let mut current: Option<BatchKind> = None;
        for b in batches {
            if current != Some(b.kind) {
                match b.kind {
                    BatchKind::Quads => {
                        pass.set_pipeline(quad);
                        pass.set_vertex_buffer(0, self.instance_buf.buffer.slice(..));
                    }
                    BatchKind::Mesh => {
                        pass.set_pipeline(mesh);
                        pass.set_vertex_buffer(0, self.vertex_buf.buffer.slice(..));
                        pass.set_index_buffer(
                            self.index_buf.buffer.slice(..),
                            wgpu::IndexFormat::Uint32,
                        );
                    }
                }
                current = Some(b.kind);
            }
            pass.set_bind_group(1, &self.texture_or_white(b.texture).bind_group, &[]);
            match b.kind {
                BatchKind::Quads => pass.draw(0..4, b.start..b.end),
                BatchKind::Mesh => pass.draw_indexed(b.start..b.end, 0, 0..1),
            }
        }
    }

    fn push_batch(
        &mut self,
        layer: Layer,
        kind: BatchKind,
        texture: TextureId,
        mask: Mask,
        start: u32,
        end: u32,
    ) {
        if end <= start {
            return;
        }
        let batches = &mut self.batches[layer as usize];
        if let Some(last) = batches.last_mut()
            && last.kind == kind
            && last.texture == texture
            && last.mask == mask
            && last.end == start
        {
            last.end = end;
            return;
        }
        batches.push(Batch {
            kind,
            texture,
            mask,
            start,
            end,
        });
    }

    fn quad(&mut self, layer: Layer, texture: TextureId, inst: QuadInstance) {
        self.quad_masked(layer, texture, Mask::None, inst);
    }

    fn quad_masked(&mut self, layer: Layer, texture: TextureId, mask: Mask, inst: QuadInstance) {
        let i = self.instances.len() as u32;
        self.instances.push(inst);
        self.push_batch(layer, BatchKind::Quads, texture, mask, i, i + 1);
    }

    fn build_layer(&mut self, list: &DrawList, layer: Layer, ppu: f32) {
        self.batches[layer as usize].clear();
        let items = &list.layers[layer as usize];
        let mut order = std::mem::take(&mut self.order);
        order.clear();
        order.extend(0..items.len());
        order.sort_by_key(|&i| items[i].sort);
        let screen = layer == Layer::Screen;
        for &i in &order {
            match &items[i].prim {
                Primitive::Sprite(s) => {
                    let mask = if layer == Layer::World {
                        s.mask
                    } else {
                        Mask::None
                    };
                    let mut inst = sprite_instance(s, screen, mask == Mask::Write);
                    if matches!(mask, Mask::WriteValue(_)) {
                        // Drop texels by texture alpha only (an invisible
                        // tint still writes the mask).
                        inst.params[1] = 1.0;
                    }
                    self.quad_masked(layer, s.texture, mask, inst);
                }
                Primitive::Rect {
                    transform,
                    size,
                    pivot,
                    corner_radius,
                    color,
                } => {
                    let inst = shape_instance(
                        *transform,
                        *size,
                        *pivot,
                        *color,
                        KIND_ROUNDED_RECT,
                        [*corner_radius, 0.0, 0.0, 0.0],
                    );
                    self.quad(layer, TextureId::WHITE, inst);
                }
                Primitive::Ellipse {
                    center,
                    radii,
                    rotation,
                    color,
                } => {
                    let t = Affine2::from_angle_translation(*rotation, *center);
                    let inst = shape_instance(
                        t,
                        *radii * 2.0,
                        Vec2::splat(0.5),
                        *color,
                        KIND_ELLIPSE,
                        [0.0; 4],
                    );
                    self.quad(layer, TextureId::WHITE, inst);
                }
                Primitive::Ring {
                    center,
                    radius,
                    thickness,
                    start,
                    sweep,
                    color,
                } => {
                    let (mut start, mut sweep) = (*start, sweep.clamp(-TAU, TAU));
                    if sweep < 0.0 {
                        start += sweep;
                        sweep = -sweep;
                    }
                    let t = Affine2::from_translation(*center);
                    let params = [thickness.min(*radius), start, sweep, 0.0];
                    let inst = shape_instance(
                        t,
                        Vec2::splat(radius * 2.0),
                        Vec2::splat(0.5),
                        *color,
                        KIND_RING,
                        params,
                    );
                    self.quad(layer, TextureId::WHITE, inst);
                }
                Primitive::Line {
                    a,
                    b,
                    width,
                    round,
                    color,
                } => {
                    let d = *b - *a;
                    let len = d.length();
                    let angle = d.y.atan2(d.x);
                    let caps = if *round { *width } else { 0.0 };
                    let t = Affine2::from_angle_translation(angle, (*a + *b) * 0.5);
                    let radius = if *round { width * 0.5 } else { 0.0 };
                    let inst = shape_instance(
                        t,
                        Vec2::new(len + caps, *width),
                        Vec2::splat(0.5),
                        *color,
                        KIND_ROUNDED_RECT,
                        [radius, 0.0, 0.0, 0.0],
                    );
                    self.quad(layer, TextureId::WHITE, inst);
                }
                Primitive::Polygon { points, color } => {
                    let base = self.vertices.len() as u32;
                    let c = color.to_linear_premul();
                    self.vertices.extend(points.iter().map(|p| MeshVertex {
                        pos: p.to_array(),
                        uv: [0.5, 0.5],
                        color: c,
                    }));
                    let i0 = self.indices.len() as u32;
                    let mut idx = Vec::new();
                    tess::triangulate(points, &mut idx);
                    self.indices.extend(idx.into_iter().map(|i| i + base));
                    let i1 = self.indices.len() as u32;
                    self.push_batch(layer, BatchKind::Mesh, TextureId::WHITE, Mask::None, i0, i1);
                }
                Primitive::Text(t) => self.build_text(layer, t, ppu),
            }
        }
        self.order = order;
    }

    fn build_sdf_text(&mut self, layer: Layer, t: &Text) {
        let screen = layer == Layer::Screen;
        let Some(f) = self.sdf_font(t.font) else {
            return;
        };
        if t.text.is_empty() || t.size.is_nan() || t.size <= 0.0 {
            return;
        }
        let (quads, block) = f.layout(&t.text, t.size, t.anchor.x);
        let tex = f.desc.atlas;
        let (aw, ah) = (f.desc.atlas_size.0 as f32, f.desc.atlas_size.1 as f32);
        let pad = f.desc.padding;
        let dilate = f.desc.dilate;
        let per_texel = 1.0 / (2.0 * (pad + 1.0));
        let offset = -t.anchor * block;
        let rot = Mat2::from_angle(t.rotation);
        let color = t.color.to_linear_premul();
        for (tl, size, g) in quads {
            let [gx, gy, gw, gh] = g.rect;
            let (u0, v0, u1, v1) = (
                (gx - pad) / aw,
                (gy - pad) / ah,
                (gx + gw + pad) / aw,
                (gy + gh + pad) / ah,
            );
            let top_left = tl + offset;
            let (corner, uv) = if screen {
                (top_left, [u0, v0, u1, v1])
            } else {
                (
                    Vec2::new(top_left.x, -top_left.y - size.y),
                    [u0, v1, u1, v0],
                )
            };
            let origin = t.position + rot * corner;
            let xa = rot * Vec2::new(size.x, 0.0);
            let ya = rot * Vec2::new(0.0, size.y);
            self.quad(
                layer,
                tex,
                QuadInstance {
                    axes: [xa.x, xa.y, ya.x, ya.y],
                    origin_size: [origin.x, origin.y, size.x, size.y],
                    uv,
                    color,
                    params: [dilate, per_texel, 0.0, 0.0],
                    kind: KIND_SDF,
                },
            );
        }
    }

    fn build_text(&mut self, layer: Layer, t: &Text, ppu: f32) {
        if t.font.is_sdf() {
            return self.build_sdf_text(layer, t);
        }
        let screen = layer == Layer::Screen;
        let px_f = if screen { t.size } else { t.size * ppu };
        if px_f.is_nan() || px_f < 1.0 || t.text.is_empty() {
            return;
        }
        let px = (px_f.round() as u32).clamp(1, MAX_GLYPH_PX);
        let scale = t.size / px as f32;
        let mut glyphs = std::mem::take(&mut self.glyphs);
        glyphs.clear();
        let block = self
            .text
            .layout(t.font, &t.text, px, t.anchor.x, &mut glyphs);
        let offset = -t.anchor * block;
        let rot = Mat2::from_angle(t.rotation);
        let mut origin_base = t.position;
        if screen && t.rotation == 0.0 {
            origin_base = (origin_base + offset * scale).round() - offset * scale;
        }
        let color = t.color.to_linear_premul();
        let inv = 1.0 / ATLAS_SIZE as f32;
        for g in &glyphs {
            let [ax, ay, w, h] = g.atlas.rect;
            let (wf, hf) = (w as f32 * scale, h as f32 * scale);
            let top_left = (g.pos + offset) * scale;
            let (corner, uv) = if screen {
                (
                    top_left,
                    [
                        ax as f32 * inv,
                        ay as f32 * inv,
                        (ax + w) as f32 * inv,
                        (ay + h) as f32 * inv,
                    ],
                )
            } else {
                (
                    Vec2::new(top_left.x, -top_left.y - hf),
                    [
                        ax as f32 * inv,
                        (ay + h) as f32 * inv,
                        (ax + w) as f32 * inv,
                        ay as f32 * inv,
                    ],
                )
            };
            let origin = origin_base + rot * corner;
            let xa = rot * Vec2::new(wf, 0.0);
            let ya = rot * Vec2::new(0.0, hf);
            self.quad(
                layer,
                TextureId::GLYPHS,
                QuadInstance {
                    axes: [xa.x, xa.y, ya.x, ya.y],
                    origin_size: [origin.x, origin.y, wf, hf],
                    uv,
                    color,
                    params: [0.0; 4],
                    kind: KIND_TEXTURED,
                },
            );
        }
        self.glyphs = glyphs;
    }

    fn build_lights(&mut self, list: &DrawList) {
        let mut plain = Vec::new();
        for light in &list.lights {
            if light.reach() <= 0.0 || light.intensity <= 0.0 {
                continue;
            }
            let e0 = self.edges.len() as u32;
            if light.casts_shadows {
                shadow_edges(light, &list.casters, &mut self.edges);
            }
            let e1 = self.edges.len() as u32;
            let c = light.color.to_linear();
            let k = light.intensity;
            let shadows = light.casts_shadows && e1 > e0;
            let half = light.cookie_size * 0.5;
            let inst = |shadows: bool| LightInstance {
                pos_range: [
                    light.position.x,
                    light.position.y,
                    light.range,
                    if shadows { 1.0 } else { 0.0 },
                ],
                color: [c[0] * k, c[1] * k, c[2] * k, 1.0],
                cookie: [
                    half.x,
                    half.y,
                    if light.cookie.is_some() { 1.0 } else { 0.0 },
                    0.0,
                ],
            };
            self.light_instances.push(inst(shadows));
            plain.push(inst(false));
            let cookie = light.cookie.unwrap_or(TextureId::WHITE);
            self.light_jobs.push((e0, e1, shadows, cookie));
        }
        if self.plain_lights {
            self.light_instances.extend(plain);
        }
    }
}

fn begin_pass<'e>(
    enc: &'e mut wgpu::CommandEncoder,
    label: &str,
    view: &'e wgpu::TextureView,
    clear: Option<[f32; 4]>,
) -> wgpu::RenderPass<'e> {
    let load = match clear {
        Some(c) => wgpu::LoadOp::Clear(wgpu::Color {
            r: c[0] as f64,
            g: c[1] as f64,
            b: c[2] as f64,
            a: c[3] as f64,
        }),
        None => wgpu::LoadOp::Load,
    };
    enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load,
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    })
}

/// Instance for a quad of `size` at `transform` with `pivot`.
fn place(transform: Affine2, size: Vec2, pivot: Vec2) -> ([f32; 4], [f32; 2]) {
    let m = transform.matrix2;
    let xa = m.x_axis * size.x;
    let ya = m.y_axis * size.y;
    let origin = transform.transform_point2(-pivot * size);
    ([xa.x, xa.y, ya.x, ya.y], origin.to_array())
}

fn shape_instance(
    transform: Affine2,
    size: Vec2,
    pivot: Vec2,
    color: Color,
    kind: u32,
    params: [f32; 4],
) -> QuadInstance {
    let (axes, o) = place(transform, size, pivot);
    QuadInstance {
        axes,
        origin_size: [o[0], o[1], size.x, size.y],
        uv: [0.0, 0.0, 1.0, 1.0],
        color: color.to_linear_premul(),
        params,
        kind,
    }
}

fn sprite_instance(s: &Sprite, screen: bool, mask_write: bool) -> QuadInstance {
    let (axes, o) = place(s.transform, s.size, s.pivot);
    // World: local y up, so the quad's bottom edge samples the bottom of the
    // image (max v). Screen: y down, so local y=0 is the top (min v).
    let (mut u0, mut u1) = (s.uv.min.x, s.uv.max.x);
    let (mut v0, mut v1) = if screen {
        (s.uv.min.y, s.uv.max.y)
    } else {
        (s.uv.max.y, s.uv.min.y)
    };
    if s.flip_x {
        std::mem::swap(&mut u0, &mut u1);
    }
    if s.flip_y {
        std::mem::swap(&mut v0, &mut v1);
    }
    QuadInstance {
        axes,
        origin_size: [o[0], o[1], s.size.x, s.size.y],
        uv: [u0, v0, u1, v1],
        color: s.tint.to_linear_premul(),
        params: [
            0.0,
            0.0,
            if mask_write { 1.0 } else { 0.0 },
            if s.blend == Blend::Additive { 1.0 } else { 0.0 },
        ],
        kind: KIND_TEXTURED,
    }
}
