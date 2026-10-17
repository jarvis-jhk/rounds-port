//! A tiny deterministic game for netcode tests: up to 12 bouncing dots, one
//! per slot, driven by [`PlayerInput`]. Integer math only, so every machine
//! computes bit-identical states. Also an input "bot" that behaves like a
//! human (holds an input for a while, then changes it).

use crate::input::PlayerInput;
use crate::session::{InputStatus, Request};
use std::collections::BTreeMap;

const W: i32 = 1600 * 256;
const H: i32 = 900 * 256;
const R: i32 = 24 * 256;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Dot {
    pub x: i32,
    pub y: i32,
    pub vx: i32,
    pub vy: i32,
    pub score: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toy {
    /// Frames simulated (= GGRS frame of this state).
    pub frame: i32,
    pub dots: Vec<Dot>,
    pub mix: u64,
}

impl Toy {
    pub fn new(players: usize, seed: u64) -> Toy {
        let mut s = seed | 1;
        let mut next = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        let dots = (0..players)
            .map(|_| Dot {
                x: R + (next() % (W - 2 * R) as u64) as i32,
                y: R + (next() % (H - 2 * R) as u64) as i32,
                ..Default::default()
            })
            .collect();
        Toy {
            frame: 0,
            dots,
            mix: seed,
        }
    }

    pub fn step(&mut self, inputs: &[(PlayerInput, InputStatus)]) {
        for (d, (i, _)) in self.dots.iter_mut().zip(inputs) {
            d.vx += i.move_x as i32 * 2;
            d.vy += i.move_y as i32 * 2 + 40; // gravity
            if i.buttons & PlayerInput::BTN_JUMP != 0 && d.y >= H - R - 256 {
                d.vy -= 2600;
            }
            if i.buttons & PlayerInput::BTN_FIRE != 0 {
                d.vx += ((i.aim as i32) >> 8) - 128;
            }
            d.vx -= d.vx / 64;
            d.vy -= d.vy / 128;
            d.x += d.vx;
            d.y += d.vy;
            if d.x < R {
                d.x = 2 * R - d.x;
                d.vx = -d.vx * 7 / 8;
            }
            if d.x > W - R {
                d.x = 2 * (W - R) - d.x;
                d.vx = -d.vx * 7 / 8;
            }
            if d.y < R {
                d.y = 2 * R - d.y;
                d.vy = -d.vy * 7 / 8;
            }
            if d.y > H - R {
                d.y = 2 * (H - R) - d.y;
                d.vy = -d.vy * 7 / 8;
            }
            d.x = d.x.clamp(R, W - R);
            d.y = d.y.clamp(R, H - R);
        }
        let n = self.dots.len();
        for a in 0..n {
            for b in a + 1..n {
                let (dx, dy) = (
                    (self.dots[a].x - self.dots[b].x) as i64,
                    (self.dots[a].y - self.dots[b].y) as i64,
                );
                if dx * dx + dy * dy < (2 * R as i64).pow(2) {
                    let (va, vb) = (
                        (self.dots[a].vx, self.dots[a].vy),
                        (self.dots[b].vx, self.dots[b].vy),
                    );
                    (self.dots[a].vx, self.dots[a].vy) = vb;
                    (self.dots[b].vx, self.dots[b].vy) = va;
                    self.dots[a].score += 1;
                    self.dots[b].score += 1;
                }
            }
        }
        for d in &self.dots {
            self.mix = (self.mix ^ (d.x as u32 as u64) ^ ((d.y as u32 as u64) << 32))
                .wrapping_mul(0x9E3779B97F4A7C15)
                .rotate_left(17);
        }
        self.frame += 1;
    }

    /// FNV-1a over the whole state, two lanes.
    pub fn checksum(&self) -> u128 {
        let mut a: u64 = 0xcbf29ce484222325;
        let mut b: u64 = 0x84222325cbf29ce4;
        let mut eat = |v: u64| {
            for byte in v.to_le_bytes() {
                a = (a ^ byte as u64).wrapping_mul(0x100000001b3);
                b = (b ^ (byte as u64).wrapping_add(0x5f)).wrapping_mul(0x100000001b3);
            }
        };
        eat(self.frame as u64);
        eat(self.mix);
        for d in &self.dots {
            eat(d.x as u64);
            eat(d.y as u64);
            eat(d.vx as u64);
            eat(d.vy as u64);
            eat(d.score as u64);
        }
        ((a as u128) << 64) | b as u128
    }
}

/// Plays a [`Toy`] from GGRS requests and remembers each frame's checksum.
pub struct ToyRunner {
    pub state: Toy,
    /// state.frame -> checksum (re-simulation overwrites).
    pub history: BTreeMap<i32, u128>,
}

impl ToyRunner {
    pub fn new(players: usize, seed: u64) -> ToyRunner {
        ToyRunner {
            state: Toy::new(players, seed),
            history: BTreeMap::new(),
        }
    }

    pub fn handle(&mut self, reqs: Vec<Request<PlayerInput, Toy>>) {
        for r in reqs {
            match r {
                Request::Save { cell, frame } => {
                    debug_assert_eq!(frame, self.state.frame);
                    cell.save(frame, Some(self.state.clone()), Some(self.state.checksum()));
                }
                Request::Load { cell, frame } => {
                    self.state = cell.load().expect("saved state");
                    debug_assert_eq!(frame, self.state.frame);
                }
                Request::Advance { inputs } => {
                    self.state.step(&inputs);
                    self.history.insert(self.state.frame, self.state.checksum());
                }
            }
        }
        // Keep the history small.
        while self.history.len() > 4096 {
            let k = *self.history.keys().next().unwrap();
            self.history.remove(&k);
        }
    }
}

/// A pseudo-human: holds an input for 4–40 frames, then picks another.
pub struct Bot {
    rng: u64,
    hold: u32,
    cur: PlayerInput,
}

impl Bot {
    pub fn new(seed: u64) -> Bot {
        Bot {
            rng: seed.wrapping_mul(0x9E3779B97F4A7C15) | 1,
            hold: 0,
            cur: PlayerInput::default(),
        }
    }

    fn next(&mut self) -> u64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng
    }

    pub fn input(&mut self) -> PlayerInput {
        if self.hold == 0 {
            let r = self.next();
            self.hold = 4 + (r % 37) as u32;
            let q = |v: u64| ((v % 5) as i8 - 2) * 60;
            self.cur = PlayerInput {
                move_x: q(r >> 8),
                move_y: q(r >> 16),
                aim: (r >> 24) as u16,
                buttons: ((r >> 40) & 0b11) as u16,
            };
        }
        self.hold -= 1;
        self.cur
    }
}

/// One machine playing the toy game over a [`NetSession`]: starts the sim on
/// `MatchStarted`, feeds bot inputs for the local slots, fulfils requests,
/// and reports `finish_match` once frame `target` is confirmed.
pub struct ToyMachine {
    pub net: crate::NetSession<PlayerInput, Toy>,
    pub runner: Option<ToyRunner>,
    bots: BTreeMap<usize, Bot>,
    /// Frame at which the match ends (its checksum is compared).
    pub target: i32,
    /// `MatchEnded`: reports and whether they agree.
    pub result: Option<(Vec<crate::DoneReport>, bool)>,
    pub salt: u64,
}

impl ToyMachine {
    pub fn new(net: crate::NetSession<PlayerInput, Toy>, target: i32, salt: u64) -> ToyMachine {
        ToyMachine {
            net,
            runner: None,
            bots: BTreeMap::new(),
            target,
            result: None,
            salt,
        }
    }

    /// Poll and handle events; returns them for logging. Call often.
    pub fn poll(&mut self) -> Vec<crate::NetEvent> {
        self.net.poll();
        let mut out = Vec::new();
        while let Some(e) = self.net.next_event() {
            match &e {
                crate::NetEvent::MatchStarted(ms) => {
                    self.runner = Some(ToyRunner::new(ms.players.len(), ms.seed));
                    self.bots = self
                        .net
                        .local_slots()
                        .iter()
                        .map(|&s| {
                            (
                                s,
                                Bot::new(self.salt.wrapping_mul(31).wrapping_add(s as u64 + 1)),
                            )
                        })
                        .collect();
                    self.result = None;
                }
                crate::NetEvent::MatchEnded {
                    reports, agreed, ..
                } => self.result = Some((reports.clone(), *agreed)),
                _ => {}
            }
            out.push(e);
        }
        out
    }

    /// One sim step (call at the sim rate).
    pub fn step(&mut self) {
        let Some(r) = self.runner.as_mut() else {
            return;
        };
        if self.net.current_match().is_none() {
            return;
        }
        for (&slot, bot) in self.bots.iter_mut() {
            let _ = self.net.add_local_input(slot, bot.input());
        }
        let reqs = self.net.advance();
        r.handle(reqs);
        if self.net.confirmed_frame() >= self.target
            && let Some(&c) = r.history.get(&self.target)
        {
            let _ = self.net.finish_match(self.target, c);
        }
    }

    /// Our checksum for the target frame, if simulated and confirmed.
    pub fn target_checksum(&self) -> Option<u128> {
        self.runner.as_ref()?.history.get(&self.target).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic() {
        let run = || {
            let mut t = Toy::new(12, 42);
            let mut bots: Vec<Bot> = (0..12).map(|i| Bot::new(i + 1)).collect();
            for _ in 0..2000 {
                let inputs: Vec<_> = bots
                    .iter_mut()
                    .map(|b| (b.input(), InputStatus::Confirmed))
                    .collect();
                t.step(&inputs);
            }
            t.checksum()
        };
        assert_eq!(run(), run());
        assert_ne!(Toy::new(12, 1).checksum(), Toy::new(12, 2).checksum());
    }
}
