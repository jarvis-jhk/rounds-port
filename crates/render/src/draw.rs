//! The per-frame draw list: what the game hands the renderer every frame.
//!
//! Build a [`DrawList`] (or clear and refill one), then pass it to
//! [`Renderer::render`](crate::Renderer::render). Items are drawn per
//! [`Layer`] in ascending `sort` key; equal keys keep submission order.

use glam::{Affine2, Vec2};

use crate::color::Color;
use crate::lighting::{Light, ShadowCaster};
use crate::text::FontId;
use crate::texture::TextureId;

/// Where an item is drawn and with which camera.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Layer {
    /// World space (y up, world camera), multiplied by the light map.
    World,
    /// World space, drawn after lighting (glows, in-world UI, debug).
    WorldUnlit,
    /// Screen space in target pixels (origin top-left, y down), after the
    /// post pass: HUD and menus.
    Screen,
    /// World space, drawn by the background stage (only used when
    /// [`FrameSettings::background`](crate::FrameSettings::background) is
    /// set): before the fog layer and the background grading, like the
    /// original's light camera.
    Background,
}

pub(crate) const LAYER_COUNT: usize = 4;

/// How a sprite blends with what is below it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Blend {
    /// Normal alpha blending.
    #[default]
    Alpha,
    /// Additive: adds `rgb * alpha`.
    Additive,
}

/// Stencil masking of world sprites (like Unity's sprite masks; only
/// honoured on [`Layer::World`]). Mask writers mark their opaque pixels
/// (alpha > 0.01) and draw no colour; masked sprites draw only inside or
/// outside the union of all marks drawn before them in the frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Mask {
    /// No interaction.
    #[default]
    None,
    /// Writes the mask, no colour.
    Write,
    /// Visible only where the mask is set.
    VisibleInside,
    /// Visible only where the mask is not set.
    VisibleOutside,
    /// Draws colour **and** sets the stencil to the value (replace), for
    /// per-object masks such as a player's body (value 2..=255; 1 is the
    /// map's mask).
    WriteValue(u8),
    /// Visible only where the stencil equals the value (see `WriteValue`).
    InsideValue(u8),
}

impl Mask {
    /// Index of the world pipeline for this mode.
    pub(crate) fn pipeline_index(self) -> usize {
        match self {
            Mask::None => 0,
            Mask::Write => 1,
            Mask::VisibleInside => 2,
            Mask::VisibleOutside => 3,
            Mask::InsideValue(_) => 4,
            Mask::WriteValue(_) => 5,
        }
    }

    /// Stencil reference value.
    pub(crate) fn reference(self) -> u32 {
        match self {
            Mask::WriteValue(v) | Mask::InsideValue(v) => v as u32,
            _ => 1,
        }
    }
}

/// A rectangle in normalised texture coordinates, `v = 0` at the top row of
/// the image (PNG order).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UvRect {
    pub min: Vec2,
    pub max: Vec2,
}

impl UvRect {
    pub const FULL: UvRect = UvRect {
        min: Vec2::ZERO,
        max: Vec2::ONE,
    };

    /// From a pixel rect with a top-left origin.
    pub fn from_pixels(x: f32, y: f32, w: f32, h: f32, tex_w: u32, tex_h: u32) -> Self {
        let s = Vec2::new(tex_w as f32, tex_h as f32);
        Self {
            min: Vec2::new(x, y) / s,
            max: Vec2::new(x + w, y + h) / s,
        }
    }

    /// From a Unity sprite rect (pixels, bottom-left origin) on a texture
    /// stored top row first.
    pub fn from_unity_pixels(x: f32, y: f32, w: f32, h: f32, tex_w: u32, tex_h: u32) -> Self {
        Self::from_pixels(x, tex_h as f32 - y - h, w, h, tex_w, tex_h)
    }
}

impl Default for UvRect {
    fn default() -> Self {
        Self::FULL
    }
}

/// A textured quad.
#[derive(Clone, Copy, Debug)]
pub struct Sprite {
    pub texture: TextureId,
    pub uv: UvRect,
    /// Position, rotation and scale (world units, or pixels on
    /// [`Layer::Screen`]).
    pub transform: Affine2,
    /// Quad size before `transform` (Unity: sprite rect / pixels per unit).
    pub size: Vec2,
    /// Pivot in 0..1 of the quad. World: (0,0) = bottom-left like Unity;
    /// screen: (0,0) = top-left. (0.5, 0.5) is the centre in both.
    pub pivot: Vec2,
    /// Multiplied with the texture (sRGB colour, straight alpha).
    pub tint: Color,
    pub flip_x: bool,
    pub flip_y: bool,
    pub blend: Blend,
    pub mask: Mask,
}

impl Default for Sprite {
    fn default() -> Self {
        Self {
            texture: TextureId::WHITE,
            uv: UvRect::FULL,
            transform: Affine2::IDENTITY,
            size: Vec2::ONE,
            pivot: Vec2::splat(0.5),
            tint: Color::WHITE,
            flip_x: false,
            flip_y: false,
            blend: Blend::Alpha,
            mask: Mask::None,
        }
    }
}

/// Text anchored at a point.
#[derive(Clone, Debug)]
pub struct Text {
    pub text: String,
    pub font: FontId,
    /// Font size (em) in world units, or pixels on [`Layer::Screen`].
    pub size: f32,
    pub position: Vec2,
    /// Rotation in radians around `position`.
    pub rotation: f32,
    /// Which point of the text block sits at `position`, in 0..1:
    /// (0,0) top-left, (0.5,0.5) centre, (1,1) bottom-right (on every
    /// layer). `anchor.x` also aligns the lines of multi-line text.
    pub anchor: Vec2,
    pub color: Color,
}

impl Default for Text {
    fn default() -> Self {
        Self {
            text: String::new(),
            font: FontId::DEFAULT,
            size: 1.0,
            position: Vec2::ZERO,
            rotation: 0.0,
            anchor: Vec2::new(0.0, 0.0),
            color: Color::WHITE,
        }
    }
}

/// Everything the renderer can draw.
#[derive(Clone, Debug)]
pub enum Primitive {
    Sprite(Sprite),
    /// Filled rectangle, optionally with rounded corners (anti-aliased).
    Rect {
        transform: Affine2,
        size: Vec2,
        pivot: Vec2,
        corner_radius: f32,
        color: Color,
    },
    /// Filled ellipse/circle (anti-aliased).
    Ellipse {
        center: Vec2,
        radii: Vec2,
        rotation: f32,
        color: Color,
    },
    /// Ring or arc (UI timers): outer `radius`, band `thickness`, starting at
    /// `start` radians (0 = +x, counter-clockwise in world space, clockwise
    /// on screen), covering `sweep` radians (`TAU` = full ring).
    Ring {
        center: Vec2,
        radius: f32,
        thickness: f32,
        start: f32,
        sweep: f32,
        color: Color,
    },
    /// A line segment of the given width; `round` gives round caps.
    Line {
        a: Vec2,
        b: Vec2,
        width: f32,
        round: bool,
        color: Color,
    },
    /// Filled simple polygon (convex or concave, either winding).
    Polygon {
        points: Vec<Vec2>,
        color: Color,
    },
    Text(Text),
}

impl From<Sprite> for Primitive {
    fn from(s: Sprite) -> Self {
        Primitive::Sprite(s)
    }
}

impl From<Text> for Primitive {
    fn from(t: Text) -> Self {
        Primitive::Text(t)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Item {
    pub sort: i32,
    pub prim: Primitive,
}

/// A frame's worth of drawing commands plus lights and shadow casters.
#[derive(Clone, Debug, Default)]
pub struct DrawList {
    pub(crate) layers: [Vec<Item>; LAYER_COUNT],
    pub lights: Vec<Light>,
    pub casters: Vec<ShadowCaster>,
}

impl DrawList {
    pub fn new() -> Self {
        Self::default()
    }

    /// Empties the list, keeping allocations.
    pub fn clear(&mut self) {
        for l in &mut self.layers {
            l.clear();
        }
        self.lights.clear();
        self.casters.clear();
    }

    /// Number of queued primitives over all layers.
    pub fn len(&self) -> usize {
        self.layers.iter().map(Vec::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn push(&mut self, layer: Layer, sort: i32, prim: impl Into<Primitive>) {
        self.layers[layer as usize].push(Item {
            sort,
            prim: prim.into(),
        });
    }

    pub fn sprite(&mut self, layer: Layer, sort: i32, sprite: Sprite) {
        self.push(layer, sort, sprite);
    }

    /// Axis-aligned (then rotated) rectangle centred at `center`.
    pub fn rect(
        &mut self,
        layer: Layer,
        sort: i32,
        center: Vec2,
        size: Vec2,
        rotation: f32,
        color: Color,
    ) {
        self.push(
            layer,
            sort,
            Primitive::Rect {
                transform: Affine2::from_angle_translation(rotation, center),
                size,
                pivot: Vec2::splat(0.5),
                corner_radius: 0.0,
                color,
            },
        );
    }

    pub fn rounded_rect(
        &mut self,
        layer: Layer,
        sort: i32,
        center: Vec2,
        size: Vec2,
        radius: f32,
        color: Color,
    ) {
        self.push(
            layer,
            sort,
            Primitive::Rect {
                transform: Affine2::from_translation(center),
                size,
                pivot: Vec2::splat(0.5),
                corner_radius: radius,
                color,
            },
        );
    }

    pub fn circle(&mut self, layer: Layer, sort: i32, center: Vec2, radius: f32, color: Color) {
        self.push(
            layer,
            sort,
            Primitive::Ellipse {
                center,
                radii: Vec2::splat(radius),
                rotation: 0.0,
                color,
            },
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn ring(
        &mut self,
        layer: Layer,
        sort: i32,
        center: Vec2,
        radius: f32,
        thickness: f32,
        start: f32,
        sweep: f32,
        color: Color,
    ) {
        self.push(
            layer,
            sort,
            Primitive::Ring {
                center,
                radius,
                thickness,
                start,
                sweep,
                color,
            },
        );
    }

    pub fn line(&mut self, layer: Layer, sort: i32, a: Vec2, b: Vec2, width: f32, color: Color) {
        self.push(
            layer,
            sort,
            Primitive::Line {
                a,
                b,
                width,
                round: false,
                color,
            },
        );
    }

    pub fn polygon(&mut self, layer: Layer, sort: i32, points: &[Vec2], color: Color) {
        self.push(
            layer,
            sort,
            Primitive::Polygon {
                points: points.to_vec(),
                color,
            },
        );
    }

    /// Text with the default font.
    #[allow(clippy::too_many_arguments)]
    pub fn text(
        &mut self,
        layer: Layer,
        sort: i32,
        text: impl Into<String>,
        position: Vec2,
        size: f32,
        anchor: Vec2,
        color: Color,
    ) {
        self.push(
            layer,
            sort,
            Text {
                text: text.into(),
                position,
                size,
                anchor,
                color,
                ..Default::default()
            },
        );
    }

    pub fn light(&mut self, light: Light) {
        self.lights.push(light);
    }

    /// A closed polygon (world space) that casts shadows.
    pub fn shadow_caster(&mut self, points: &[Vec2]) {
        self.casters.push(ShadowCaster {
            points: points.to_vec(),
        });
    }
}
