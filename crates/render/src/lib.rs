//! `rounds-render`: the 2D wgpu renderer of rounds-port.
//!
//! The game fills a [`DrawList`] each frame (sprites, shapes, text, lights,
//! shadow casters) and calls [`Renderer::render`] with a [`FrameSettings`]
//! (camera, clear colour, lighting, post) and a target: a window surface
//! ([`SurfaceTarget`]) or an [`OffscreenTarget`] that can be captured to an
//! [`Image`] and saved as PNG. The same code path runs headless (no window,
//! no display server) for screenshots and tests.
//!
//! Conventions: world space is y-up with Unity-style orthographic cameras;
//! the [`Layer::Screen`] layer uses pixels with a top-left origin. Colours
//! are sRGB like Unity's `Color` in a linear-space project and are blended in
//! linear light.
//!
//! ```no_run
//! use rounds_render::*;
//! use glam::Vec2;
//! let gpu = Gpu::headless().unwrap();
//! let mut renderer = Renderer::new(&gpu, OFFSCREEN_FORMAT);
//! let target = OffscreenTarget::new(&gpu, (640, 360), OFFSCREEN_FORMAT);
//! let mut list = DrawList::new();
//! list.circle(Layer::World, 0, Vec2::ZERO, 1.0, Color::hex(0xff8800));
//! let frame = FrameSettings { clear: Color::hex(0x202030), ..Default::default() };
//! renderer.render(&gpu, target.target(), &frame, &list);
//! target.capture(&gpu).unwrap().save_png("scratch/example.png").unwrap();
//! ```

mod camera;
mod color;
mod draw;
mod gpu;
mod grading;
mod image;
mod lighting;
mod renderer;
mod target;
pub mod tess;
mod text;
mod texture;

pub use camera::{Camera2D, screen_proj};
pub use color::{Color, linear_to_srgb, srgb_to_linear};
pub use draw::{Blend, DrawList, Layer, Mask, Primitive, Sprite, Text, UvRect};
pub use gpu::{Gpu, new_instance};
pub use grading::{BackgroundSettings, ColorGrading, Fog, Tonemapper, Vignette};
pub use image::Image;
pub use lighting::{Light, LightingSettings, ShadowCaster};
pub use renderer::{Bloom, FrameSettings, FrameStats, HDR_FORMAT, PostSettings, Renderer};
pub use target::{OFFSCREEN_FORMAT, OffscreenTarget, SurfaceFrame, SurfaceTarget, TargetView};
pub use text::{DEFAULT_FONT_BYTES, FontId, SdfFontDesc, SdfGlyph};
pub use texture::{Filter, TextureId, TextureOptions, Wrap};

/// Re-export so dependents use the same wgpu version.
pub use wgpu;

/// Renderer errors.
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("no suitable GPU adapter: {0}")]
    NoAdapter(String),
    #[error("GPU device error: {0}")]
    Device(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("unknown texture id")]
    BadTexture,
    #[error("font error: {0}")]
    Font(String),
    #[error("PNG error: {0}")]
    Png(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
