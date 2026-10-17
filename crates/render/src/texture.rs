//! Texture handles and the GPU texture store.

/// Handle to a texture owned by the [`Renderer`](crate::Renderer).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TextureId(pub(crate) u32);

impl TextureId {
    /// A 1x1 opaque white texture (used by untextured shapes).
    pub const WHITE: TextureId = TextureId(0);
    /// The glyph atlas of the built-in text system.
    pub(crate) const GLYPHS: TextureId = TextureId(1);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Filter {
    #[default]
    Linear,
    Nearest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Wrap {
    #[default]
    Clamp,
    Repeat,
}

/// How a texture is sampled and interpreted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextureOptions {
    pub filter: Filter,
    pub wrap: Wrap,
    /// Colour data (sRGB-encoded bytes, decoded to linear on sampling).
    /// False for data textures (masks, SDF atlases, LUTs).
    pub srgb: bool,
}

impl Default for TextureOptions {
    fn default() -> Self {
        Self {
            filter: Filter::Linear,
            wrap: Wrap::Clamp,
            srgb: true,
        }
    }
}

pub(crate) struct GpuTexture {
    pub texture: wgpu::Texture,
    pub bind_group: wgpu::BindGroup,
    pub size: (u32, u32),
}

pub(crate) fn texture_format(opts: &TextureOptions) -> wgpu::TextureFormat {
    if opts.srgb {
        wgpu::TextureFormat::Rgba8UnormSrgb
    } else {
        wgpu::TextureFormat::Rgba8Unorm
    }
}

pub(crate) fn sampler_desc(opts: &TextureOptions) -> wgpu::SamplerDescriptor<'static> {
    let filter = match opts.filter {
        Filter::Linear => wgpu::FilterMode::Linear,
        Filter::Nearest => wgpu::FilterMode::Nearest,
    };
    let wrap = match opts.wrap {
        Wrap::Clamp => wgpu::AddressMode::ClampToEdge,
        Wrap::Repeat => wgpu::AddressMode::Repeat,
    };
    wgpu::SamplerDescriptor {
        label: Some("texture sampler"),
        address_mode_u: wrap,
        address_mode_v: wrap,
        address_mode_w: wrap,
        mag_filter: filter,
        min_filter: filter,
        ..Default::default()
    }
}

/// Writes rows `[rows.0, rows.1)` of a full-width RGBA8 pixel buffer into
/// `texture`.
pub(crate) fn write_rows(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    pixels: &[u8],
    width: u32,
    rows: (u32, u32),
) {
    let (y0, y1) = rows;
    if y1 <= y0 {
        return;
    }
    let stride = width * 4;
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d { x: 0, y: y0, z: 0 },
            aspect: wgpu::TextureAspect::All,
        },
        &pixels[(y0 * stride) as usize..(y1 * stride) as usize],
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(stride),
            rows_per_image: Some(y1 - y0),
        },
        wgpu::Extent3d {
            width,
            height: y1 - y0,
            depth_or_array_layers: 1,
        },
    );
}
