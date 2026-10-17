//! 2D lights and soft shadows, modelled on the original's SF Soft Shadows
//! setup (`SFRenderer` / `SFLight` / `SFPolygon`).
//!
//! The model: a low-resolution *light map* covering the view is cleared to
//! the ambient colour; every light adds its radial falloff, attenuated by a
//! per-light *shadow mask*. Shadow casters are closed polygons; each edge
//! that faces a light is extruded away from it into a shadow quad whose
//! fragment shader computes, analytically, what fraction of the light's disk
//! (`source_radius`) the edge hides from that pixel, which gives soft
//! penumbrae for free. Lit layers are multiplied by `light map * exposure`.
//!
//! What the original has that this does not (yet): light penetration into
//! casters (`minLightPenetration`), fog/scatter, cookie textures, the
//! non-linear "soft" light blend mode, per-material lighting opt-out (use
//! [`Layer::WorldUnlit`](crate::Layer::WorldUnlit) instead).

use bytemuck::{Pod, Zeroable};
use glam::Vec2;

use crate::color::Color;
use crate::texture::TextureId;

/// A point light.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Light {
    pub position: Vec2,
    /// sRGB colour; multiplied by `intensity`.
    pub color: Color,
    pub intensity: f32,
    /// Distance at which the light falls off to zero (world units).
    pub range: f32,
    /// Radius of the emitting disk (world units): larger = softer shadows.
    /// Zero gives hard shadows.
    pub source_radius: f32,
    pub casts_shadows: bool,
    /// Cookie texture (like `SFLight`): when set, the light covers the
    /// rectangle `position ± cookie_size / 2` and its colour is the cookie
    /// sample times `color * intensity` (no radial falloff; `range` is
    /// ignored and derived from the rectangle).
    pub cookie: Option<TextureId>,
    /// World size of the cookie rectangle.
    pub cookie_size: Vec2,
}

impl Light {
    /// Distance beyond which the light has no effect.
    pub fn reach(&self) -> f32 {
        if self.cookie.is_some() {
            self.cookie_size.length() * 0.5
        } else {
            self.range
        }
    }
}

impl Default for Light {
    fn default() -> Self {
        Self {
            position: Vec2::ZERO,
            color: Color::WHITE,
            intensity: 1.0,
            range: 10.0,
            source_radius: 0.3,
            casts_shadows: true,
            cookie: None,
            cookie_size: Vec2::ZERO,
        }
    }
}

/// A closed polygon (world space, either winding) that blocks light.
#[derive(Clone, Debug, PartialEq)]
pub struct ShadowCaster {
    pub points: Vec<Vec2>,
}

/// Scene-wide lighting parameters (cf. `SFRenderer`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightingSettings {
    /// When false, lit layers are drawn at full brightness and lights are
    /// ignored.
    pub enabled: bool,
    /// Light everywhere, before lights are added (sRGB).
    pub ambient: Color,
    /// Multiplier applied to the light map when it is applied to the scene.
    pub exposure: f32,
    /// Light map resolution divisor relative to the target (the original
    /// uses 8).
    pub light_map_scale: u32,
    /// Resolution divisor of the light map *with shadows* (the original
    /// uses 1; 0 = same as `light_map_scale`).
    pub shadow_map_scale: u32,
    /// Multiply the [`Layer::World`](crate::Layer::World) layer by the
    /// shadowed light map (`light map * exposure`). The original instead
    /// shows its lights only through the background fog layer
    /// ([`Fog`](crate::Fog)) and draws the world unlit: set this to false.
    pub multiply_world: bool,
}

impl Default for LightingSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            ambient: Color::WHITE,
            exposure: 1.0,
            light_map_scale: 4,
            shadow_map_scale: 0,
            multiply_world: true,
        }
    }
}

/// GPU instance: one shadow-casting edge for one light.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(crate) struct ShadowEdge {
    /// Edge endpoints (a.xy, b.xy), oriented so the light is on the right.
    pub edge: [f32; 4],
    /// Light position, source radius, extrusion length.
    pub light: [f32; 4],
}

/// GPU instance: one light quad.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(crate) struct LightInstance {
    /// Position, range, unused.
    pub pos_range: [f32; 4],
    /// Linear colour * intensity.
    pub color: [f32; 4],
    /// Cookie half size (x, y), cookie flag (z), unused.
    pub cookie: [f32; 4],
}

fn segment_distance(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_squared().max(1e-12)).clamp(0.0, 1.0);
    (a + ab * t).distance(p)
}

/// Appends the edges of `casters` that face `light` and lie within its
/// range. Edges are oriented so that the light lies on their outer side
/// (`cross(b - a, light - a) < 0`, i.e. to the right walking from a to b).
pub(crate) fn shadow_edges(light: &Light, casters: &[ShadowCaster], out: &mut Vec<ShadowEdge>) {
    let lp = light.position;
    let r = light.source_radius.max(1e-4);
    for c in casters {
        let pts = &c.points;
        let n = pts.len();
        if n < 2 {
            continue;
        }
        let ccw = crate::tess::signed_area2(pts) >= 0.0;
        for i in 0..n {
            let (mut a, mut b) = (pts[i], pts[(i + 1) % n]);
            if !ccw {
                std::mem::swap(&mut a, &mut b);
            }
            // CCW polygon: outward normal of a->b is to the right.
            let e = b - a;
            if e.length_squared() < 1e-12 {
                continue;
            }
            let faces_light = e.perp_dot(lp - a) < 0.0;
            let reach = light.reach();
            if !faces_light || segment_distance(lp, a, b) > reach {
                continue;
            }
            let extrude = 2.0 * reach + a.distance(lp).max(b.distance(lp));
            out.push(ShadowEdge {
                edge: [a.x, a.y, b.x, b.y],
                light: [lp.x, lp.y, r, extrude],
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(c: Vec2, h: f32) -> ShadowCaster {
        ShadowCaster {
            points: vec![
                c + Vec2::new(-h, -h),
                c + Vec2::new(h, -h),
                c + Vec2::new(h, h),
                c + Vec2::new(-h, h),
            ],
        }
    }

    #[test]
    fn only_light_facing_edges() {
        let light = Light {
            position: Vec2::new(-5.0, 0.0),
            ..Default::default()
        };
        let mut out = Vec::new();
        shadow_edges(&light, &[square(Vec2::ZERO, 1.0)], &mut out);
        // Only the left edge (x = -1) faces a light straight to the left.
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].edge[0], -1.0);
        assert_eq!(out[0].edge[2], -1.0);
        // Clockwise input gives the same edge set.
        let mut cw = square(Vec2::ZERO, 1.0);
        cw.points.reverse();
        let mut out2 = Vec::new();
        shadow_edges(&light, &[cw], &mut out2);
        assert_eq!(out2.len(), 1);
        // Light at a corner diagonal sees two edges.
        let diag = Light {
            position: Vec2::new(-5.0, -5.0),
            ..Default::default()
        };
        out.clear();
        shadow_edges(&diag, &[square(Vec2::ZERO, 1.0)], &mut out);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn out_of_range_culled() {
        let light = Light {
            position: Vec2::new(-50.0, 0.0),
            range: 10.0,
            ..Default::default()
        };
        let mut out = Vec::new();
        shadow_edges(&light, &[square(Vec2::ZERO, 1.0)], &mut out);
        assert!(out.is_empty());
    }
}
