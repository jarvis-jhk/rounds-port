//! Deterministic bot inputs for headless smoke runs and debug renders.
//!
//! A bot walks toward the closest other living player, jumps at random and
//! when blocked by a wall, aims at its target with some jitter, fires most
//! of the time and blocks now and then. During card picks it moves the
//! selection and confirms with a jump press. Everything comes from a
//! xorshift generator seeded per run and the world state, so the inputs are
//! a pure function of `(seed, world)`.

use rounds_sim::{PlayerInput, World, v2};

pub struct Bots {
    rng: u64,
    /// Per player: ticks left on the current random move, its direction.
    hold: Vec<(u32, f32)>,
    /// Behaviour: 0 = chase and shoot, 1 = random wanderer.
    style: Vec<u8>,
}

impl Bots {
    pub fn new(seed: u64, players: usize) -> Bots {
        let mut b = Bots {
            rng: seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1,
            hold: vec![(0, 0.0); players],
            style: vec![0; players],
        };
        for i in 0..players {
            b.style[i] = b.next().is_multiple_of(3) as u8;
        }
        b
    }

    fn next(&mut self) -> u32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        (x >> 32) as u32
    }

    fn chance(&mut self, one_in: u32) -> bool {
        self.next().is_multiple_of(one_in.max(1))
    }

    fn unit(&mut self) -> f32 {
        (self.next() & 0xffff) as f32 / 65535.0 * 2.0 - 1.0
    }

    /// Inputs for every player of `w` for the next tick.
    pub fn inputs(&mut self, w: &World) -> Vec<PlayerInput> {
        let players = w.players();
        let frame = w.frame();
        let mut out = Vec::with_capacity(players.len());
        for (i, p) in players.iter().enumerate() {
            if i >= self.hold.len() {
                self.hold.push((0, 0.0));
                self.style.push(0);
            }
            // Card pick / menus: wiggle and confirm (edge: pressed on even
            // 16-frame windows only).
            let mut buttons = 0u16;
            if w.pick().is_some() {
                if self.chance(20) {
                    buttons |= if self.chance(2) {
                        PlayerInput::BTN_MENU_LEFT
                    } else {
                        PlayerInput::BTN_MENU_RIGHT
                    };
                }
                if (frame / 8) % 4 == 1 && self.chance(3) {
                    buttons |= PlayerInput::BTN_JUMP | PlayerInput::BTN_MENU_OK;
                }
                out.push(PlayerInput {
                    buttons,
                    ..Default::default()
                });
                continue;
            }
            // Target: closest other living player.
            let target = players
                .iter()
                .enumerate()
                .filter(|(j, q)| *j != i && q.alive() && q.playing)
                .min_by(|a, b| {
                    let da = (a.1.pos - p.pos).len_sq();
                    let db = (b.1.pos - p.pos).len_sq();
                    da.total_cmp(&db).then(a.0.cmp(&b.0))
                })
                .map(|(_, q)| q.pos);
            let (h, dir) = self.hold[i];
            let mx = if h > 0 {
                self.hold[i].0 -= 1;
                dir
            } else if self.style[i] == 1 || target.is_none() || self.chance(6) {
                let d = [-1.0, -1.0, 0.0, 1.0, 1.0][(self.next() % 5) as usize];
                self.hold[i] = (20 + self.next() % 60, d);
                d
            } else {
                let t = target.unwrap();
                if (t.x - p.pos.x).abs() < 2.0 {
                    0.0
                } else {
                    (t.x - p.pos.x).signum()
                }
            };
            let wants_up = target.is_some_and(|t| t.y > p.pos.y + 2.0);
            if self.chance(if wants_up { 12 } else { 40 }) || (p.wall_grab && self.chance(6)) {
                buttons |= PlayerInput::BTN_JUMP;
            }
            // Hold jump for a few frames after pressing (higher jumps).
            if p.since_jump < 0.15 && self.chance(2) {
                buttons |= PlayerInput::BTN_JUMP;
            }
            let my = if self.chance(30) { -1.0 } else { 0.0 };
            let aim = match target {
                Some(t) => {
                    let d = (t - p.pos).normalized();
                    v2(d.x + self.unit() * 0.15, d.y + self.unit() * 0.15 + 0.05)
                }
                None => v2(self.unit(), self.unit().abs()),
            };
            if self.chance(4) || (frame + i as u32).is_multiple_of(3) {
                buttons |= PlayerInput::BTN_FIRE;
            }
            if self.chance(90) {
                buttons |= PlayerInput::BTN_BLOCK;
            }
            out.push(PlayerInput::from_axes(
                v2(mx, my),
                aim.normalized(),
                buttons,
            ));
        }
        out
    }
}
