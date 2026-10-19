//! Built-in mods (docs/MODS.md): every toggle off is vanilla, every toggle
//! on changes behaviour.
mod common;
use common::*;
use rounds_sim::{Content, MapId, MatchSetup, PickMode, PlayerInput, World};

/// Scripted random inputs (same generator as the determinism test).
struct Bots {
    s: u64,
    cur: Vec<PlayerInput>,
}

impl Bots {
    fn new(n: usize, seed: u64) -> Bots {
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
    fn next_inputs(&mut self) -> Vec<PlayerInput> {
        for i in 0..self.cur.len() {
            let mut p = self.cur[i];
            if self.r().is_multiple_of(20) {
                p.move_x = [-127i8, 0, 127][(self.r() % 3) as usize];
                p.move_y = if self.r().is_multiple_of(6) { -127 } else { 0 };
            }
            if self.r().is_multiple_of(10) {
                let a = (self.r() % 6283) as f32 / 1000.0;
                p.aim_x = (libm::cosf(a) * 32000.0) as i16;
                p.aim_y = (libm::sinf(a) * 32000.0) as i16;
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

/// Checksums at frames 2000, 5000 and 10000 of a bot match.
fn run(c: &Content, s: &MatchSetup, mode: PickMode) -> [u64; 3] {
    let mut w = World::new(c, s);
    w.set_pick_mode(mode);
    let mut bots = Bots::new(s.slots.len(), 99);
    let mut out = [0; 3];
    for f in 1..=10_000 {
        w.step(c, &bots.next_inputs());
        match f {
            2000 => out[0] = w.checksum(),
            5000 => out[1] = w.checksum(),
            10_000 => out[2] = w.checksum(),
            _ => {}
        }
    }
    out
}

/// Recorded before the mods existed (same runs as `run`): with every mod
/// off the simulation must not change by a single bit.
#[test]
fn all_off_is_vanilla() {
    let c = content();
    let s = setup(3);
    assert!(s.mods.is_vanilla());
    assert_eq!(
        run(&c, &s, PickMode::Sequential),
        [0x23a73f80eade9ba0, 0xf764c78a1a3f3e77, 0xe6988266326c6a3b]
    );
    assert_eq!(
        run(&c, &setup(4), PickMode::Simultaneous),
        [0x746dfea5d690002e, 0x2d2a9ac0feb3219e, 0xbb64c50947f0fb9d]
    );
}

/// Steps through the opening pick phase confirming every card as soon as
/// possible. Returns the largest hand seen and every player's card count
/// when the first battle starts.
fn opening_picks(c: &Content, s: &MatchSetup, mode: PickMode) -> (usize, Vec<usize>) {
    let mut w = World::new(c, s);
    w.set_pick_mode(mode);
    let n = s.slots.len();
    let mut max_hand = 0;
    for f in 0..20_000 {
        let b = if f % 2 == 0 {
            PlayerInput::BTN_MENU_OK
        } else {
            0
        };
        let inp = vec![
            PlayerInput {
                buttons: b,
                ..PlayerInput::default()
            };
            n
        ];
        w.step(c, &inp);
        if let Some(p) = w.pick() {
            max_hand = max_hand.max(p.hand.len());
        }
        for h in w.pick_hands() {
            max_hand = max_hand.max(h.state.hand.len());
        }
        if w.battle_ongoing() {
            return (
                max_hand,
                w.players().iter().map(|p| p.cards.len()).collect(),
            );
        }
    }
    panic!("no battle after the opening picks: {:?}", w.phase());
}

#[test]
fn pick_n_cards() {
    use rounds_sim::mods::PickNCards;
    let c = content();
    for mode in [PickMode::Sequential, PickMode::Simultaneous] {
        let vanilla = setup(3);
        assert_eq!(
            opening_picks(&c, &vanilla, mode),
            (5, vec![1, 1, 1]),
            "{mode:?}"
        );
        let mut s = setup(3);
        s.mods.pick_n_cards = Some(PickNCards {
            picks: 2,
            hand_size: 3,
        });
        assert_eq!(opening_picks(&c, &s, mode), (3, vec![2, 2, 2]), "{mode:?}");
        // Same setup, same match: the mod is deterministic.
        assert_eq!(run(&c, &s, mode), run(&c, &s, mode));
        assert_ne!(run(&c, &s, mode), run(&c, &vanilla, mode));
    }
}
