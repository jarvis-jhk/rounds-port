//! Determinism and rollback (docs/sim-design.md "Rules for determinism").
mod common;
use common::*;
use rounds_sim::{Content, MapId, MatchSetup, PlayerInput, World};

/// Scripted random bot inputs (deterministic, independent of the sim).
pub struct Bots {
    s: u64,
    cur: Vec<PlayerInput>,
}

impl Bots {
    pub fn new(n: usize, seed: u64) -> Bots {
        Bots {
            s: seed | 1,
            cur: vec![PlayerInput::default(); n],
        }
    }
    fn r(&mut self) -> u32 {
        self.s ^= self.s << 13;
        self.s ^= self.s >> 7;
        self.s ^= self.s << 17;
        (self.s >> 16) as u32
    }
    pub fn next_inputs(&mut self) -> Vec<PlayerInput> {
        for i in 0..self.cur.len() {
            let mut p = self.cur[i];
            if self.r().is_multiple_of(20) {
                p.move_x = [-127i8, 0, 127][(self.r() % 3) as usize];
                p.move_y = if self.r().is_multiple_of(6) { -127 } else { 0 };
            }
            if self.r().is_multiple_of(10) {
                let a = (self.r() % 6283) as f32 / 1000.0;
                p.aim_x = (a.cos() * 32000.0) as i16;
                p.aim_y = (a.sin() * 32000.0) as i16;
            }
            let mut b = p.buttons & PlayerInput::BTN_FIRE;
            if self.r().is_multiple_of(8) {
                b ^= PlayerInput::BTN_FIRE;
            }
            if self.r().is_multiple_of(12) {
                b |= PlayerInput::BTN_JUMP;
            }
            if self.r().is_multiple_of(40) {
                b |= PlayerInput::BTN_BLOCK;
            }
            p.buttons = b;
            self.cur[i] = p;
        }
        self.cur.clone()
    }
}

fn setup(n: usize) -> MatchSetup {
    let mut s = MatchSetup::ffa(n, 1234);
    s.map_rotation = vec![MapId(0), MapId(1), MapId(2), MapId(3)];
    s
}

#[test]
fn two_worlds_10k_frames_identical_and_clone_restep() {
    let c: Content = content();
    let n = 4;
    let mut a = World::new(&c, &setup(n));
    let mut b = World::new(&c, &setup(n));
    let mut bots = Bots::new(n, 99);
    let mut inputs = Vec::new();
    let mut mid: Option<World> = None;
    let mut sums = Vec::new();
    for f in 0..10_000 {
        let inp = bots.next_inputs();
        a.step(&c, &inp);
        b.step(&c, &inp);
        if f % 50 == 0 {
            assert_eq!(a.checksum(), b.checksum(), "desync at frame {f}");
        }
        if f == 5000 {
            mid = Some(a.clone());
        }
        if f > 5000 {
            sums.push(a.checksum());
        }
        inputs.push(inp);
    }
    assert_eq!(a.checksum(), b.checksum());
    let mut m = mid.unwrap();
    for (k, f) in (5001..10_000).enumerate() {
        m.step(&c, &inputs[f]);
        assert_eq!(m.checksum(), sums[k], "clone diverged at frame {f}");
    }
    // The match actually progressed (points scored, props simulated ...).
    let progressed = a.points().iter().chain(a.rounds().iter()).any(|x| *x > 0)
        || a.rounds().iter().any(|x| *x > 0);
    assert!(progressed, "no points in 10k frames: phase {:?}", a.phase());
}

#[test]
fn rollback_8_frames() {
    let c = content();
    let n = 4;
    let mut w = World::new(&c, &setup(n));
    let mut bots = Bots::new(n, 7);
    for _ in 0..3000 {
        let inp = bots.next_inputs();
        w.step(&c, &inp);
    }
    let saved = w.clone();
    let seq: Vec<_> = (0..8).map(|_| bots.next_inputs()).collect();
    for inp in &seq {
        w.step(&c, inp);
    }
    let want = w.checksum();
    // Mispredicted branch, then restore and re-simulate.
    let mut other = Bots::new(n, 12345);
    let mut w2 = saved.clone();
    for _ in 0..8 {
        let inp = other.next_inputs();
        w2.step(&c, &inp);
    }
    let mut w2 = saved;
    for inp in &seq {
        w2.step(&c, inp);
    }
    assert_eq!(w2.checksum(), want);
}

#[test]
fn physics_map_sandbox_deterministic() {
    let c = content();
    let mk = || sandbox(&c, 2, 4);
    let mut a = mk();
    let mut b = mk();
    let mut bots = Bots::new(4, 3);
    let mut saved = None;
    let mut seq = Vec::new();
    for f in 0..3000 {
        let inp = bots.next_inputs();
        a.step(&c, &inp);
        b.step(&c, &inp);
        if f == 1500 {
            saved = Some(a.clone());
        }
        if f > 1500 {
            seq.push((inp, a.checksum()));
        }
    }
    assert_eq!(a.checksum(), b.checksum());
    let mut s = saved.unwrap();
    for (inp, sum) in seq {
        s.step(&c, &inp);
        assert_eq!(s.checksum(), sum);
    }
}
