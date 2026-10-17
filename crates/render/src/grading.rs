//! Colour grading, vignette, fog and the background stage, modelled on the
//! original's Unity post-processing stack v2 ("HDR" grading mode with the
//! ACES tonemapper) and the SF Soft Shadows fog layer.
//!
//! The grading is evaluated analytically per pixel (Unity bakes the same
//! function into a 33³ LUT in LogC space):
//!
//! 1. `colour *= 2^post_exposure`, clamped to the LUT's LogC range;
//! 2. sRGB-linear → ACES AP0; contrast in (approximate) ACEScc log space
//!    around mid grey 0.4135884;
//! 3. AP0 → ACEScg: colour filter, channel mixer, lift/gamma/gain, hue
//!    shift (HSV), saturation;
//! 4. ACEScg → AP0 → ACES tonemap (Unity's fitted RRT + ODT) → linear sRGB.

use glam::Vec2;

use crate::color::{Color, srgb_to_linear};

/// Tonemapper of a [`ColorGrading`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tonemapper {
    /// No tonemapping (values above 1 clip on output).
    None,
    /// Unity's ACES (fitted RRT + ODT for a 100-nit sRGB display).
    #[default]
    Aces,
}

/// Unity post-processing v2 colour grading in HDR mode. Field values are
/// the inspector values of the original's profiles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorGrading {
    pub tonemapper: Tonemapper,
    /// Exposure in EV (stops), applied before grading.
    pub post_exposure: f32,
    /// -100..100.
    pub contrast: f32,
    /// -100..100.
    pub saturation: f32,
    /// Degrees, -180..180.
    pub hue_shift: f32,
    /// Colour filter (sRGB, multiplied in linear).
    pub color_filter: Color,
    /// Channel mixer rows in percent: output red/green/blue from (r, g, b) in.
    pub mixer_red: [f32; 3],
    pub mixer_green: [f32; 3],
    pub mixer_blue: [f32; 3],
    /// Trackballs (rgb + offset in w), as stored in the profile.
    pub lift: [f32; 4],
    pub gamma: [f32; 4],
    pub gain: [f32; 4],
}

impl Default for ColorGrading {
    fn default() -> Self {
        Self {
            tonemapper: Tonemapper::Aces,
            post_exposure: 0.0,
            contrast: 0.0,
            saturation: 0.0,
            hue_shift: 0.0,
            color_filter: Color::WHITE,
            mixer_red: [100.0, 0.0, 0.0],
            mixer_green: [0.0, 100.0, 0.0],
            mixer_blue: [0.0, 0.0, 100.0],
            lift: [1.0, 1.0, 1.0, 0.0],
            gamma: [1.0, 1.0, 1.0, 0.0],
            gain: [1.0, 1.0, 1.0, 0.0],
        }
    }
}

fn luma(c: [f32; 3]) -> f32 {
    c[0] * 0.2126 + c[1] * 0.7152 + c[2] * 0.0722
}

impl ColorGrading {
    /// Lift as the shader uses it (Unity: trackball * 0.2, minus luma).
    pub fn lift_params(&self) -> [f32; 3] {
        let c = self.lift.map(|v| v * 0.2);
        let l = luma([c[0], c[1], c[2]]);
        [c[0] - l + c[3], c[1] - l + c[3], c[2] - l + c[3]]
    }

    /// Inverse gamma (Unity: trackball * 0.8, minus luma, plus 1, inverted).
    pub fn inv_gamma_params(&self) -> [f32; 3] {
        let c = self.gamma.map(|v| v * 0.8);
        let l = luma([c[0], c[1], c[2]]);
        let o = c[3] + 1.0;
        [0, 1, 2].map(|i| 1.0 / (c[i] - l + o).max(1e-3))
    }

    /// Gain (Unity: trackball * 0.8, minus luma, plus 1).
    pub fn gain_params(&self) -> [f32; 3] {
        let c = self.gain.map(|v| v * 0.8);
        let l = luma([c[0], c[1], c[2]]);
        let o = c[3] + 1.0;
        [0, 1, 2].map(|i| c[i] - l + o)
    }
}

/// Unity post-processing v2 "classic" vignette.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vignette {
    /// sRGB colour.
    pub color: Color,
    /// Centre in 0..1 screen coordinates.
    pub center: Vec2,
    pub intensity: f32,
    pub smoothness: f32,
    pub roundness: f32,
    /// Perfectly round (aspect-corrected) instead of following the screen.
    pub rounded: bool,
}

impl Default for Vignette {
    fn default() -> Self {
        Self {
            color: Color::BLACK,
            center: Vec2::splat(0.5),
            intensity: 0.0,
            smoothness: 0.2,
            roundness: 1.0,
            rounded: false,
        }
    }
}

/// The SF Soft Shadows fog layer, drawn over the background stage with
/// premultiplied blending: `src.rgb = (fog.rgb * fog.a + scatter.rgb *
/// mix(light_map, shadow_map, soft_hard_mix)) * exposure`, `src.a = fog.a`.
/// Needs [`LightingSettings::enabled`](crate::LightingSettings).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fog {
    /// sRGB colour; alpha = strength.
    pub color: Color,
    /// sRGB scatter colour (the fog's glow where lit).
    pub scatter: Color,
    /// 0 = light map without shadows, 1 = with shadows.
    pub soft_hard_mix: f32,
    /// Multiplier (the original uses its `SFRenderer` exposure).
    pub exposure: f32,
}

/// A separate first stage like the original's light camera: its own clear
/// colour, the [`Layer::Background`](crate::Layer::Background) items, an
/// optional fog layer and its own grading. Its tonemapped result is what the
/// world layer is drawn onto (instead of [`FrameSettings::clear`](crate::FrameSettings)).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct BackgroundSettings {
    /// Clear colour (sRGB; HDR values allowed).
    pub clear: Color,
    pub fog: Option<Fog>,
    /// Grading of the background; `None` passes it through (clamped to 0..1).
    pub grading: Option<ColorGrading>,
}

/// GPU form of grading + vignette (part of the full-screen uniform block).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct GradingParams {
    /// x: grading on, y: ACES on, z: post exposure multiplier, w: vignette on.
    pub flags: [f32; 4],
    /// Hue shift (turns), saturation (multiplier), contrast (multiplier).
    pub hsc: [f32; 4],
    pub lift: [f32; 4],
    pub inv_gamma: [f32; 4],
    pub gain: [f32; 4],
    pub filter: [f32; 4],
    pub mix_r: [f32; 4],
    pub mix_g: [f32; 4],
    pub mix_b: [f32; 4],
    /// Intensity * 3, smoothness * 5, roundness exponent, rounded.
    pub vig_settings: [f32; 4],
    pub vig_color: [f32; 4],
    /// Centre (xy), aspect (z).
    pub vig_center: [f32; 4],
}

impl GradingParams {
    pub(crate) fn new(g: Option<&ColorGrading>, v: Option<&Vignette>, aspect: f32) -> Self {
        let mut p = GradingParams::default();
        if let Some(g) = g {
            let f = g.color_filter.to_linear();
            let v3 = |a: [f32; 3]| [a[0], a[1], a[2], 0.0];
            let m = |a: [f32; 3]| [a[0] / 100.0, a[1] / 100.0, a[2] / 100.0, 0.0];
            p.flags = [
                1.0,
                if g.tonemapper == Tonemapper::Aces {
                    1.0
                } else {
                    0.0
                },
                g.post_exposure.exp2(),
                0.0,
            ];
            p.hsc = [
                g.hue_shift / 360.0,
                g.saturation / 100.0 + 1.0,
                g.contrast / 100.0 + 1.0,
                0.0,
            ];
            p.lift = v3(g.lift_params());
            p.inv_gamma = v3(g.inv_gamma_params());
            p.gain = v3(g.gain_params());
            p.filter = [f[0], f[1], f[2], 1.0];
            p.mix_r = m(g.mixer_red);
            p.mix_g = m(g.mixer_green);
            p.mix_b = m(g.mixer_blue);
        }
        if let Some(v) = v.filter(|v| v.intensity > 0.0) {
            p.flags[3] = 1.0;
            p.vig_settings = [
                v.intensity * 3.0,
                v.smoothness * 5.0,
                (1.0 - v.roundness) * 6.0 + v.roundness,
                if v.rounded { 1.0 } else { 0.0 },
            ];
            let c = v.color.to_linear();
            p.vig_color = [c[0], c[1], c[2], 1.0];
            p.vig_center = [v.center.x, v.center.y, aspect, 0.0];
        }
        p
    }
}

/// Linear-light fog colour terms: `(fog.rgb * fog.a, fog.a)` and the
/// linear scatter colour.
pub(crate) fn fog_terms(f: &Fog) -> ([f32; 4], [f32; 4]) {
    let c = f.color;
    let a = c.a;
    let lin = |v: f32| srgb_to_linear(v);
    (
        [lin(c.r) * a, lin(c.g) * a, lin(c.b) * a, a],
        [
            lin(f.scatter.r),
            lin(f.scatter.g),
            lin(f.scatter.b),
            f.soft_hard_mix,
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_trackballs_are_identity() {
        let g = ColorGrading::default();
        for v in g.lift_params() {
            assert!(v.abs() < 1e-6);
        }
        for v in g.inv_gamma_params() {
            assert!((v - 1.0).abs() < 1e-6);
        }
        for v in g.gain_params() {
            assert!((v - 1.0).abs() < 1e-6);
        }
    }

    #[test]
    fn warm_gain() {
        // The original's "Default" profile: gain (1, 0.644, 0.309).
        let g = ColorGrading {
            gain: [1.0, 0.644, 0.309, 0.0],
            ..Default::default()
        };
        let [r, gg, b] = g.gain_params();
        assert!(r > 1.2 && gg < 1.0 && b < 0.75, "{r} {gg} {b}");
    }
}
