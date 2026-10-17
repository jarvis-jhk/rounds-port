//! Deterministic 2D math: a small `V2` type, angle helpers and Unity-style
//! animation curves. Transcendental functions go through `libm` only
//! (docs/sim-design.md, "Rules for determinism").

use serde::{Deserialize, Serialize};
use std::ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Neg, Sub, SubAssign};

pub const DEG2RAD: f32 = core::f32::consts::PI / 180.0;
pub const RAD2DEG: f32 = 180.0 / core::f32::consts::PI;

#[inline]
pub fn sin(x: f32) -> f32 {
    libm::sinf(x)
}
#[inline]
pub fn cos(x: f32) -> f32 {
    libm::cosf(x)
}
#[inline]
pub fn atan2(y: f32, x: f32) -> f32 {
    libm::atan2f(y, x)
}
#[inline]
pub fn pow(x: f32, y: f32) -> f32 {
    libm::powf(x, y)
}
#[inline]
pub fn exp(x: f32) -> f32 {
    libm::expf(x)
}
#[inline]
pub fn sqrt(x: f32) -> f32 {
    x.sqrt() // IEEE-exact
}
#[inline]
pub fn clamp(x: f32, lo: f32, hi: f32) -> f32 {
    if x < lo {
        lo
    } else if x > hi {
        hi
    } else {
        x
    }
}
#[inline]
pub fn clamp01(x: f32) -> f32 {
    clamp(x, 0.0, 1.0)
}
#[inline]
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * clamp01(t)
}
/// Unity `Mathf.Lerp` without clamping the parameter.
#[inline]
pub fn lerp_unclamped(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
/// Banker's rounding (Unity `Mathf.RoundToInt`).
pub fn round_half_even(x: f32) -> i32 {
    let f = libm::floorf(x);
    let d = x - f;
    let fi = f as i32;
    if d > 0.5 || (d == 0.5 && fi % 2 != 0) {
        fi + 1
    } else {
        fi
    }
}

/// 2D vector (f32). Gameplay uses x right, y up (Unity units).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct V2 {
    pub x: f32,
    pub y: f32,
}

#[inline]
pub const fn v2(x: f32, y: f32) -> V2 {
    V2 { x, y }
}

impl V2 {
    pub const ZERO: V2 = v2(0.0, 0.0);
    pub const UP: V2 = v2(0.0, 1.0);
    pub const DOWN: V2 = v2(0.0, -1.0);
    pub const RIGHT: V2 = v2(1.0, 0.0);
    pub const LEFT: V2 = v2(-1.0, 0.0);

    #[inline]
    pub fn new(x: f32, y: f32) -> V2 {
        v2(x, y)
    }
    #[inline]
    pub fn dot(self, o: V2) -> f32 {
        self.x * o.x + self.y * o.y
    }
    /// z of the 3D cross product.
    #[inline]
    pub fn cross(self, o: V2) -> f32 {
        self.x * o.y - self.y * o.x
    }
    #[inline]
    pub fn len_sq(self) -> f32 {
        self.dot(self)
    }
    #[inline]
    pub fn len(self) -> f32 {
        sqrt(self.len_sq())
    }
    /// Unit vector, or zero when the length is (almost) zero (Unity semantics:
    /// `normalized` returns zero below 1e-5).
    #[inline]
    pub fn normalized(self) -> V2 {
        let l = self.len();
        if l > 1e-5 { self / l } else { V2::ZERO }
    }
    #[inline]
    pub fn is_zero(self) -> bool {
        self.x == 0.0 && self.y == 0.0
    }
    /// `(y, -x)`: rotated by -90 degrees (the right-hand side of travel).
    #[inline]
    pub fn perp_right(self) -> V2 {
        v2(self.y, -self.x)
    }
    /// `(-y, x)`: rotated by +90 degrees.
    #[inline]
    pub fn perp_left(self) -> V2 {
        v2(-self.y, self.x)
    }
    /// Reflect about a unit normal (Unity `Vector2.Reflect`).
    #[inline]
    pub fn reflect(self, n: V2) -> V2 {
        self - n * (2.0 * self.dot(n))
    }
    #[inline]
    pub fn rotate(self, c: f32, s: f32) -> V2 {
        v2(self.x * c - self.y * s, self.x * s + self.y * c)
    }
    #[inline]
    pub fn rotate_angle(self, radians: f32) -> V2 {
        self.rotate(cos(radians), sin(radians))
    }
    #[inline]
    pub fn lerp(self, o: V2, t: f32) -> V2 {
        let t = clamp01(t);
        self + (o - self) * t
    }
    #[inline]
    pub fn clamp_len(self, max: f32) -> V2 {
        let l2 = self.len_sq();
        if l2 > max * max {
            self * (max / sqrt(l2))
        } else {
            self
        }
    }
    #[inline]
    pub fn dist(self, o: V2) -> f32 {
        (self - o).len()
    }
    #[inline]
    pub fn min(self, o: V2) -> V2 {
        v2(self.x.min(o.x), self.y.min(o.y))
    }
    #[inline]
    pub fn max(self, o: V2) -> V2 {
        v2(self.x.max(o.x), self.y.max(o.y))
    }
    #[inline]
    pub fn abs(self) -> V2 {
        v2(self.x.abs(), self.y.abs())
    }
    /// Unsigned angle in degrees between two vectors (Unity `Vector2.Angle`;
    /// 0 if either is zero).
    pub fn angle_deg(self, o: V2) -> f32 {
        if self.len_sq() < 1e-15 || o.len_sq() < 1e-15 {
            return 0.0;
        }
        let c = self.cross(o).abs();
        let d = self.dot(o);
        atan2(c, d) * RAD2DEG
    }
    /// Angle of the vector in radians, counter-clockwise from +x.
    #[inline]
    pub fn angle(self) -> f32 {
        atan2(self.y, self.x)
    }
    #[inline]
    pub fn from_angle(radians: f32) -> V2 {
        v2(cos(radians), sin(radians))
    }
    #[inline]
    pub fn bits(self) -> [u32; 2] {
        [self.x.to_bits(), self.y.to_bits()]
    }
}

impl Add for V2 {
    type Output = V2;
    #[inline]
    fn add(self, o: V2) -> V2 {
        v2(self.x + o.x, self.y + o.y)
    }
}
impl Sub for V2 {
    type Output = V2;
    #[inline]
    fn sub(self, o: V2) -> V2 {
        v2(self.x - o.x, self.y - o.y)
    }
}
impl Mul<f32> for V2 {
    type Output = V2;
    #[inline]
    fn mul(self, s: f32) -> V2 {
        v2(self.x * s, self.y * s)
    }
}
impl Mul<V2> for f32 {
    type Output = V2;
    #[inline]
    fn mul(self, v: V2) -> V2 {
        v2(v.x * self, v.y * self)
    }
}
impl Div<f32> for V2 {
    type Output = V2;
    #[inline]
    fn div(self, s: f32) -> V2 {
        v2(self.x / s, self.y / s)
    }
}
impl Neg for V2 {
    type Output = V2;
    #[inline]
    fn neg(self) -> V2 {
        v2(-self.x, -self.y)
    }
}
impl AddAssign for V2 {
    #[inline]
    fn add_assign(&mut self, o: V2) {
        self.x += o.x;
        self.y += o.y;
    }
}
impl SubAssign for V2 {
    #[inline]
    fn sub_assign(&mut self, o: V2) {
        self.x -= o.x;
        self.y -= o.y;
    }
}
impl MulAssign<f32> for V2 {
    #[inline]
    fn mul_assign(&mut self, s: f32) {
        self.x *= s;
        self.y *= s;
    }
}
impl DivAssign<f32> for V2 {
    #[inline]
    fn div_assign(&mut self, s: f32) {
        self.x /= s;
        self.y /= s;
    }
}

/// The 16 compass directions (22.5 degrees apart), starting straight up and
/// going counter-clockwise (player.md 3.5, 17).
pub fn dir16(i: usize) -> V2 {
    let a = (90.0 + 22.5 * i as f32) * DEG2RAD;
    let v = V2::from_angle(a);
    // Snap tiny components to exact zero so axis directions are exact.
    v2(
        if v.x.abs() < 1e-6 { 0.0 } else { v.x },
        if v.y.abs() < 1e-6 { 0.0 } else { v.y },
    )
}

/// Snap to the nearest of the 16 directions (largest dot product; ties keep
/// the lowest index). Zero stays zero. Result is a unit vector.
pub fn snap16(v: V2) -> V2 {
    if v.is_zero() {
        return V2::ZERO;
    }
    let mut best = 0usize;
    let mut best_dot = f32::NEG_INFINITY;
    for i in 0..16 {
        let d = dir16(i).dot(v);
        if d > best_dot {
            best_dot = d;
            best = i;
        }
    }
    dir16(best)
}

/// Rotate `cur` toward `target` by at most `max_rad` radians and change its
/// length by at most `max_len_delta` (Unity `Vector3.RotateTowards`, 2D).
pub fn rotate_towards(cur: V2, target: V2, max_rad: f32, max_len_delta: f32) -> V2 {
    let cl = cur.len();
    let tl = target.len();
    if cl < 1e-6 || tl < 1e-6 {
        return move_towards(cur, target, max_len_delta);
    }
    let a0 = cur.angle();
    let a1 = target.angle();
    let mut d = a1 - a0;
    let pi = core::f32::consts::PI;
    while d > pi {
        d -= 2.0 * pi;
    }
    while d < -pi {
        d += 2.0 * pi;
    }
    let step = clamp(d, -max_rad, max_rad);
    let l = cl + clamp(tl - cl, -max_len_delta, max_len_delta);
    V2::from_angle(a0 + step) * l
}

pub fn move_towards(cur: V2, target: V2, max_delta: f32) -> V2 {
    let d = target - cur;
    let l = d.len();
    if l <= max_delta || l < 1e-9 {
        target
    } else {
        cur + d / l * max_delta
    }
}

/// One key of a Unity `AnimationCurve` (unweighted tangents).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Key {
    pub time: f32,
    pub value: f32,
    #[serde(default)]
    pub in_slope: f32,
    #[serde(default)]
    pub out_slope: f32,
}

pub const fn key(time: f32, value: f32, in_slope: f32, out_slope: f32) -> Key {
    Key {
        time,
        value,
        in_slope,
        out_slope,
    }
}

/// Cubic Hermite evaluation per segment, clamped outside the key range
/// (cards.md "AnimationCurve evaluation").
pub fn eval_curve(keys: &[Key], x: f32) -> f32 {
    match keys.len() {
        0 => return 0.0,
        1 => return keys[0].value,
        _ => {}
    }
    if x <= keys[0].time {
        return keys[0].value;
    }
    let last = keys[keys.len() - 1];
    if x >= last.time {
        return last.value;
    }
    for w in keys.windows(2) {
        let (a, b) = (w[0], w[1]);
        if x <= b.time {
            let dt = b.time - a.time;
            if dt <= 0.0 {
                return b.value;
            }
            let t = (x - a.time) / dt;
            let t2 = t * t;
            let t3 = t2 * t;
            let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
            let h10 = t3 - 2.0 * t2 + t;
            let h01 = -2.0 * t3 + 3.0 * t2;
            let h11 = t3 - t2;
            return h00 * a.value + h10 * dt * a.out_slope + h01 * b.value + h11 * dt * b.in_slope;
        }
    }
    last.value
}

/// Owned curve for content data.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Curve {
    pub keys: Vec<Key>,
}

impl Curve {
    pub fn eval(&self, x: f32) -> f32 {
        eval_curve(&self.keys, x)
    }
    pub fn end_time(&self) -> f32 {
        self.keys.last().map(|k| k.time).unwrap_or(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snap_directions() {
        assert_eq!(snap16(v2(0.0, 1.0)), v2(0.0, 1.0));
        assert_eq!(snap16(v2(1.0, 0.0)), v2(1.0, 0.0));
        let d = snap16(v2(0.9, 0.9));
        assert!((d.x - core::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        assert_eq!(snap16(V2::ZERO), V2::ZERO);
        // A tiny deflection is already full speed.
        assert_eq!(snap16(v2(0.01, 0.0)), v2(1.0, 0.0));
    }

    #[test]
    fn curve_hermite() {
        let k = [key(0.0, 0.0, 0.0, 0.0), key(1.0, 1.0, 0.0, 0.0)];
        assert_eq!(eval_curve(&k, -1.0), 0.0);
        assert_eq!(eval_curve(&k, 2.0), 1.0);
        assert!((eval_curve(&k, 0.5) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn rounding() {
        assert_eq!(round_half_even(0.5), 0);
        assert_eq!(round_half_even(1.5), 2);
        assert_eq!(round_half_even(2.5), 2);
        assert_eq!(round_half_even(2.6), 3);
    }

    #[test]
    fn angles() {
        assert!((v2(1.0, 0.0).angle_deg(v2(0.0, 1.0)) - 90.0).abs() < 1e-4);
        assert!((v2(1.0, 0.0).angle_deg(v2(-1.0, 0.0)) - 180.0).abs() < 1e-4);
    }
}
