//! M5 stress: 12 players, each with many random real cards, 10k frames of
//! bot input. No panics; two worlds stay identical; a clone restored
//! mid-run reproduces the same checksums. Skipped without an import.
mod fixture;

use fixture::real;
use rounds_sim::{CardId, GameMode, MapId, MatchSetup, PlayerInput, World};

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
                p.move_x = [-127i8, 0, 0, 127][(self.r() % 4) as usize];
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
            if self.r().is_multiple_of(30) {
                b |= PlayerInput::BTN_BLOCK;
            }
            p.buttons = b;
            self.cur[i] = p;
        }
        self.cur.clone()
    }
}

fn setup(map: u16) -> MatchSetup {
    let mut s = MatchSetup::ffa(12, 4242);
    s.mode = GameMode::Sandbox;
    s.map_rotation = vec![MapId(map)];
    s
}

/// 12 sandbox players with `per` random cards each (from all 79).
fn world(c: &rounds_sim::Content, seed: u64, per: usize, map: u16) -> World {
    let mut w = World::new(c, &setup(map));
    let idle = vec![PlayerInput::default(); 12];
    for _ in 0..300 {
        w.step(c, &idle);
    }
    let mut r = Bots::new(1, seed);
    for p in 0..12 {
        for _ in 0..per {
            let k = r.r() as usize % c.cards.len();
            w.debug_give_card(c, p, CardId(k as u16));
        }
    }
    w
}

#[test]
fn twelve_players_many_cards_10k_frames() {
    let Some(rc) = real() else {
        eprintln!("no import found: skipped");
        return;
    };
    let c = &rc.content;
    for map in [0u16, 2] {
        let mut a = world(c, 7 + map as u64, 16, map);
        let mut b = world(c, 7 + map as u64, 16, map);
        assert_eq!(a.checksum(), b.checksum());
        let mut bots = Bots::new(12, 99 + map as u64);
        let mut inputs = Vec::new();
        let mut mid: Option<World> = None;
        let mut sums = Vec::new();
        let mut kinds = std::collections::BTreeSet::new();
        let mut max_proj = 0;
        let mut max_obj = 0;
        for f in 0..10_000 {
            let inp = bots.next_inputs();
            a.step(c, &inp);
            b.step(c, &inp);
            for e in a.events() {
                let d = format!("{:?}", e.kind);
                kinds.insert(d.split([' ', '{', '(']).next().unwrap_or("").to_string());
            }
            max_proj = max_proj.max(a.projectiles().len());
            max_obj = max_obj.max(a.objects().len());
            if f % 50 == 0 {
                assert_eq!(a.checksum(), b.checksum(), "map {map}: desync at frame {f}");
            }
            if f == 5000 {
                mid = Some(a.clone());
            }
            if f > 5000 {
                inputs.push(inp);
                sums.push(a.checksum());
            }
        }
        // Restore the clone and replay the second half.
        let mut m = mid.unwrap();
        for (k, inp) in inputs.iter().enumerate() {
            m.step(c, inp);
            assert_eq!(m.checksum(), sums[k], "map {map}: clone diverged at {k}");
        }
        eprintln!(
            "map {map}: max projectiles {max_proj}, max objects {max_obj}, {} event kinds",
            kinds.len()
        );
        let mut hist = std::collections::BTreeMap::new();
        for o in a.objects() {
            *hist.entry(c.object(o.def).name.clone()).or_insert(0) += 1;
        }
        eprintln!("objects at the end: {hist:?}");
        for must in [
            "Explosion",
            "ObjectSpawned",
            "BlockStarted",
            "Damaged",
            "ProjectileHit",
        ] {
            assert!(kinds.contains(must), "map {map}: no {must} events");
        }
    }
}

/// Full arms-race flow on real data: picks (bots press jump), rounds, map
/// changes with card effect objects; two worlds stay identical.
#[test]
fn arms_race_with_real_cards_is_deterministic() {
    let Some(rc) = real() else {
        eprintln!("no import found: skipped");
        return;
    };
    let c = &rc.content;
    let mut s = MatchSetup::ffa(4, 99);
    s.map_rotation = vec![MapId(0), MapId(1), MapId(2)];
    s.rounds_to_win_game = 3;
    let mut a = World::new(c, &s);
    let mut b = World::new(c, &s);
    let mut bots = Bots::new(4, 5);
    let mut max_effects = 0;
    let mut picks = 0;
    for f in 0..30_000 {
        let inp = bots.next_inputs();
        a.step(c, &inp);
        b.step(c, &inp);
        picks += a
            .events()
            .iter()
            .filter(|e| matches!(e.kind, rounds_sim::EventKind::CardPicked { .. }))
            .count();
        max_effects = max_effects.max((0..4).map(|p| a.player(p).effects.len()).sum::<usize>());
        if f % 100 == 0 {
            assert_eq!(a.checksum(), b.checksum(), "desync at frame {f}");
        }
    }
    eprintln!("picks {picks}, max effect objects {max_effects}");
    assert!(picks >= 4, "cards were picked through the flow");
}
