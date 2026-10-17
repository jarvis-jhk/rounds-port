//! Fixed-timestep accumulator with render interpolation.

/// The original's physics step (Unity `Fixed Timestep`), ≈71.4 Hz.
pub const FIXED_DT: f64 = 0.014;

/// Turns variable frame times into a whole number of fixed simulation steps
/// plus an interpolation factor for rendering between the last two states.
#[derive(Clone, Debug)]
pub struct FixedStep {
    dt: f64,
    acc: f64,
    /// Longest frame time accepted; longer hitches are clamped so the sim
    /// does not spiral trying to catch up.
    max_frame: f64,
    ticks: u64,
}

#[allow(dead_code)]
impl FixedStep {
    pub fn new(dt: f64) -> Self {
        Self {
            dt,
            acc: 0.0,
            max_frame: 0.25,
            ticks: 0,
        }
    }

    /// Adds a frame's elapsed time; returns how many steps to run now.
    pub fn advance(&mut self, frame_dt: f64) -> u32 {
        self.acc += frame_dt.clamp(0.0, self.max_frame);
        let mut n = 0;
        while self.acc >= self.dt {
            self.acc -= self.dt;
            n += 1;
        }
        self.ticks += n as u64;
        n
    }

    /// How far (0..1) the current time is between the last step and the
    /// next: draw `lerp(previous, current, alpha)`.
    pub fn alpha(&self) -> f32 {
        ((self.acc / self.dt) as f32).clamp(0.0, 1.0)
    }

    /// Total steps run so far.
    pub fn ticks(&self) -> u64 {
        self.ticks
    }

    pub fn dt(&self) -> f64 {
        self.dt
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sixty_hz_frames() {
        let mut f = FixedStep::new(FIXED_DT);
        let mut total = 0;
        for _ in 0..600 {
            total += f.advance(1.0 / 60.0);
            assert!((0.0..=1.0).contains(&f.alpha()));
        }
        // 10 s at 0.014 s per step = 714.28 (±1 for float accumulation).
        assert!((713..=715).contains(&total), "{total}");
        assert_eq!(f.ticks(), total as u64);
    }

    #[test]
    fn hitch_is_clamped() {
        let mut f = FixedStep::new(FIXED_DT);
        assert_eq!(f.advance(5.0), (0.25 / FIXED_DT) as u32);
    }
}
