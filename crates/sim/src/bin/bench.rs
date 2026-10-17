//! Step / clone benchmark: 12 players, rapid-fire spread guns, busy arena.
//! `cargo run --release -p rounds-sim --bin sim-bench [frames] [--real]`
//! `--real`: the player's imported cards (`cache/import`), 16 random cards
//! per player, bot inputs with blocks (M5 card effects).

#[path = "../../tests/fixture.rs"]
#[allow(unused_imports)]
mod fixture;

use rounds_sim::content::test_maps::{physics_arena, test_cards, test_objects};
use rounds_sim::content::{CardDef, Content, ContentDefs, Rarity, Tuning};
use rounds_sim::stats::GunStats;
use rounds_sim::{GameMode, MapId, MatchSetup, PlayerInput, World, v2};

/// Test content plus a crate-heavy arena and a bullet-hose card.
fn bench_content() -> Content {
    let flat = rounds_sim::content::test_maps::flat_arena();
    let mut busy = physics_arena();
    busy.name = "Bench_Crates".into();
    let template = busy.props[0].clone();
    for k in 0..56 {
        let mut p = template.clone();
        p.pos = v2(-26.0 + (k % 14) as f32 * 3.8, -10.5 + (k / 14) as f32 * 2.2);
        busy.props.push(p);
    }
    let mut cards = test_cards();
    cards.push(CardDef {
        name: "Hose".into(),
        rarity: Rarity::Common,
        gun: Some(GunStats {
            number_of_projectiles: 3,
            attack_speed: 0.5,
            ammo: 60,
            reflects: 3,
            spread: 0.3,
            damage: 0.2,
            ..Default::default()
        }),
        ..Default::default()
    });
    let tuning = Tuning {
        bullet_collision: std::env::var("NO_BULLET_COLLISION").is_err(),
        ..Tuning::default()
    };
    Content::new(ContentDefs {
        maps: vec![flat, busy],
        cards,
        pool: vec![],
        objects: test_objects(),
        projectiles: vec![],
        tuning,
        sound_events: Default::default(),
    })
}
use std::time::Instant;

struct Bots(u64);
impl Bots {
    fn r(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 16) as u32
    }
}

fn main() {
    let frames: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(3000);
    if std::env::args().any(|a| a == "--real") {
        return real_bench(frames);
    }
    let c = bench_content();
    for (map, label) in [
        (0u16, "flat arena (no props)"),
        (1u16, "crate arena (62 props, ropes, saw, platform)"),
    ] {
        let mut s = MatchSetup::ffa(12, 42);
        s.mode = GameMode::Sandbox;
        s.map_rotation = vec![MapId(map)];
        let mut w = World::new(&c, &s);
        let hose = c.card_by_name("Hose").unwrap();
        for i in 0..12 {
            w.debug_give_card(&c, i, hose);
            w.debug_give_card(&c, i, hose);
        }
        let mut bots = Bots(0x9e37_79b9_7f4a_7c15);
        let mut inputs = vec![PlayerInput::default(); 12];
        let mut step_ns = Vec::with_capacity(frames);
        let mut clone_ns = Vec::with_capacity(frames);
        let mut max_bullets = 0;
        let mut bullets_sum = 0usize;
        for f in 0..frames {
            for (i, p) in inputs.iter_mut().enumerate() {
                if bots.r().is_multiple_of(25) {
                    p.move_x = [-127i8, 0, 127][(bots.r() % 3) as usize];
                }
                let a = (f as f32 * 0.05 + i as f32).sin();
                *p = PlayerInput {
                    aim_x: (a * 32000.0) as i16,
                    aim_y: ((1.0 - a * a).sqrt() * 32000.0) as i16,
                    ..*p
                };
                p.buttons = PlayerInput::BTN_FIRE
                    | if bots.r().is_multiple_of(30) {
                        PlayerInput::BTN_JUMP
                    } else {
                        0
                    };
            }
            let t0 = Instant::now();
            w.step(&c, &inputs);
            let t1 = Instant::now();
            let snap = std::hint::black_box(w.clone());
            let t2 = Instant::now();
            drop(snap);
            if f >= 300 {
                step_ns.push((t1 - t0).as_nanos() as u64);
                clone_ns.push((t2 - t1).as_nanos() as u64);
                bullets_sum += w.projectiles().len();
            }
            max_bullets = max_bullets.max(w.projectiles().len());
            // Keep everyone alive and on the map for a busy, stable scene.
            for i in 0..12 {
                if w.player(i).dead || w.player(i).health < 50.0 {
                    let content = &c;
                    w.revive(content, i, true);
                    let x = -24.0 + i as f32 * 4.3;
                    w.teleport(i, v2(x, -10.0));
                }
            }
        }
        let stat = |v: &mut Vec<u64>| {
            v.sort_unstable();
            let avg = v.iter().sum::<u64>() as f64 / v.len() as f64;
            (
                avg / 1000.0,
                v[v.len() / 2] as f64 / 1000.0,
                v[v.len() * 99 / 100] as f64 / 1000.0,
                *v.last().unwrap() as f64 / 1000.0,
            )
        };
        let (sa, sm, s99, smax) = stat(&mut step_ns);
        let (ca, cm, c99, cmax) = stat(&mut clone_ns);
        println!(
            "{label}: {} frames, 12 players, bullets avg {:.0} max {}, props {}",
            step_ns.len(),
            bullets_sum as f64 / step_ns.len() as f64,
            max_bullets,
            w.props().len()
        );
        println!("  step  us: avg {sa:.1}  median {sm:.1}  p99 {s99:.1}  max {smax:.1}");
        println!("  clone us: avg {ca:.1}  median {cm:.1}  p99 {c99:.1}  max {cmax:.1}");
    }
}

/// 12 players with 16 random real cards each, random bot inputs (fire,
/// jump, block, move) on the flat and the crate arena.
fn real_bench(frames: usize) {
    let Some(rc) = fixture::real() else {
        eprintln!("no import found (cache/import or ROUNDS_IMPORT_DIR)");
        return;
    };
    let c = &rc.content;
    for (map, label) in [(0u16, "flat arena"), (2u16, "physics arena")] {
        let mut s = MatchSetup::ffa(12, 4242);
        s.mode = GameMode::Sandbox;
        s.map_rotation = vec![MapId(map)];
        let mut w = World::new(c, &s);
        let idle = vec![PlayerInput::default(); 12];
        for _ in 0..300 {
            w.step(c, &idle);
        }
        let mut bots = Bots(0x2545_f491_4f6c_dd1d);
        for p in 0..12 {
            for _ in 0..16 {
                let k = bots.r() as usize % c.cards.len();
                w.debug_give_card(c, p, rounds_sim::CardId(k as u16));
            }
        }
        let mut inputs = vec![PlayerInput::default(); 12];
        let mut step_ns = Vec::with_capacity(frames);
        let mut clone_ns = Vec::with_capacity(frames);
        let (mut maxp, mut maxo, mut sump, mut sumo) = (0, 0, 0usize, 0usize);
        for f in 0..frames {
            for p in inputs.iter_mut() {
                if bots.r().is_multiple_of(20) {
                    p.move_x = [-127i8, 0, 0, 127][(bots.r() % 4) as usize];
                }
                if bots.r().is_multiple_of(10) {
                    let a = (bots.r() % 6283) as f32 / 1000.0;
                    p.aim_x = (a.cos() * 32000.0) as i16;
                    p.aim_y = (a.sin() * 32000.0) as i16;
                }
                let mut b = p.buttons & PlayerInput::BTN_FIRE;
                if bots.r().is_multiple_of(8) {
                    b ^= PlayerInput::BTN_FIRE;
                }
                if bots.r().is_multiple_of(12) {
                    b |= PlayerInput::BTN_JUMP;
                }
                if bots.r().is_multiple_of(30) {
                    b |= PlayerInput::BTN_BLOCK;
                }
                p.buttons = b;
            }
            let t0 = Instant::now();
            w.step(c, &inputs);
            let t1 = Instant::now();
            let snap = std::hint::black_box(w.clone());
            let t2 = Instant::now();
            drop(snap);
            if f >= 300 {
                step_ns.push((t1 - t0).as_nanos() as u64);
                clone_ns.push((t2 - t1).as_nanos() as u64);
                sump += w.projectiles().len();
                sumo += w.objects().len();
            }
            maxp = maxp.max(w.projectiles().len());
            maxo = maxo.max(w.objects().len());
        }
        let stat = |v: &mut Vec<u64>| {
            v.sort_unstable();
            let avg = v.iter().sum::<u64>() as f64 / v.len() as f64;
            (
                avg / 1000.0,
                v[v.len() / 2] as f64 / 1000.0,
                v[v.len() * 99 / 100] as f64 / 1000.0,
                *v.last().unwrap() as f64 / 1000.0,
            )
        };
        let n = step_ns.len().max(1) as f64;
        let (sa, sm, s99, smax) = stat(&mut step_ns);
        let (ca, cm, c99, cmax) = stat(&mut clone_ns);
        println!(
            "real cards, {label}: {} frames, 12 players x 16 cards, bullets avg {:.0} max {maxp}, objects avg {:.0} max {maxo}",
            n as usize,
            sump as f64 / n,
            sumo as f64 / n
        );
        println!("  step  us: avg {sa:.1}  median {sm:.1}  p99 {s99:.1}  max {smax:.1}");
        println!("  clone us: avg {ca:.1}  median {cm:.1}  p99 {c99:.1}  max {cmax:.1}");
    }
}
