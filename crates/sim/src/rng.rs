//! Deterministic PRNG (PCG32, XSH-RR) with separate streams so that, e.g.,
//! card draws never perturb bullet spread (docs/sim-design.md; weapons.md 24).

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Pcg32 {
    state: u64,
    inc: u64,
}

const MUL: u64 = 6364136223846793005;

impl Pcg32 {
    pub fn new(seed: u64, stream: u64) -> Pcg32 {
        let mut r = Pcg32 {
            state: 0,
            inc: (stream << 1) | 1,
        };
        r.next_u32();
        r.state = r.state.wrapping_add(seed);
        r.next_u32();
        r
    }

    pub fn next_u32(&mut self) -> u32 {
        let old = self.state;
        self.state = old.wrapping_mul(MUL).wrapping_add(self.inc);
        let xorshifted = (((old >> 18) ^ old) >> 27) as u32;
        let rot = (old >> 59) as u32;
        xorshifted.rotate_right(rot)
    }

    /// Uniform in [0, 1) with 24 bits of precision.
    pub fn next_f32(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 * (1.0 / 16_777_216.0)
    }

    /// Uniform float in [lo, hi) (`Random.Range(float, float)`).
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.next_f32()
    }

    /// Uniform integer in [0, n) without modulo bias. `n == 0` returns 0.
    pub fn below(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        let threshold = n.wrapping_neg() % n;
        loop {
            let r = self.next_u32();
            if r >= threshold {
                return r % n;
            }
        }
    }

    pub fn words(&self) -> [u64; 2] {
        [self.state, self.inc]
    }
}

/// Independent streams of the simulation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rngs {
    /// Projectile spawn draws (spread, speed seed) - weapons.md 24.1.
    pub gun: Pcg32,
    /// Hit-time rolls (bash) and explosion-side randomness - weapons.md 24.2.
    pub hit: Pcg32,
    /// Card hands - cards.md 3.
    pub cards: Pcg32,
    /// Map rotation and sandbox spawn choice - match-flow.md 6.
    pub maps: Pcg32,
    /// Everything else gameplay-relevant (debris impulses, ...).
    pub misc: Pcg32,
    /// Seeds handed to the presentation (per-child slide delays); never
    /// consumed by gameplay decisions.
    pub cosmetic: Pcg32,
}

impl Rngs {
    pub fn new(seed: u64) -> Rngs {
        Rngs {
            gun: Pcg32::new(seed, 1),
            hit: Pcg32::new(seed, 2),
            cards: Pcg32::new(seed, 3),
            maps: Pcg32::new(seed, 4),
            misc: Pcg32::new(seed, 5),
            cosmetic: Pcg32::new(seed, 6),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streams_differ_and_repeat() {
        let mut a = Pcg32::new(42, 1);
        let mut b = Pcg32::new(42, 2);
        let mut c = Pcg32::new(42, 1);
        let xa: Vec<u32> = (0..8).map(|_| a.next_u32()).collect();
        let xb: Vec<u32> = (0..8).map(|_| b.next_u32()).collect();
        let xc: Vec<u32> = (0..8).map(|_| c.next_u32()).collect();
        assert_ne!(xa, xb);
        assert_eq!(xa, xc);
    }

    #[test]
    fn ranges() {
        let mut r = Pcg32::new(7, 9);
        for _ in 0..10_000 {
            let f = r.next_f32();
            assert!((0.0..1.0).contains(&f));
            assert!(r.below(5) < 5);
        }
    }
}
