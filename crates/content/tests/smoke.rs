//! Headless smoke test on the imported content (skipped when `cache/import`
//! is missing; the repo holds no game data).
//!
//! For every map: a 4-player and a 12-player arms-race match (rotation
//! restricted to that map, initial card picks on) driven by the bots for
//! `ROUNDS_SMOKE_FRAMES` ticks (default 5000). Checks: no panic, finite
//! positions, living players stay near the arena (|x| < 60, |y| < 60), the
//! match reaches battles and card picks, and the same seed gives the same
//! checksum sequence (each match runs twice). Living players whose centre is
//! inside static geometry for > 5% of ticks are reported as `WARN` lines.
//!
//! Run: `cargo test --release -p rounds-content --test smoke -- --nocapture`

use std::sync::OnceLock;

use rayon::prelude::*;
use rounds_content::bots::Bots;
use rounds_sim::flow::Phase;
use rounds_sim::{Content, EventKind, MapId, MatchSetup, World};

fn content() -> Option<&'static Content> {
    static C: OnceLock<Option<Content>> = OnceLock::new();
    C.get_or_init(|| {
        let dir = rounds_content::dev_import_dir();
        if !dir.join("index.json").is_file() {
            eprintln!("smoke: no import at {}; skipped", dir.display());
            return None;
        }
        let data = rounds_import::GameData::load(&dir).expect("load import");
        let mut built = rounds_content::build(&data).expect("build content");
        Some(built.content())
    })
    .as_ref()
}

#[derive(Debug, Default, Clone, PartialEq)]
struct Stats {
    checksums: Vec<u64>,
    battles: u32,
    picks: u32,
    deaths: u32,
    max_abs: f32,
    /// Living-player ticks with the centre inside static geometry.
    inside_static: u32,
    player_ticks: u32,
    outside: Vec<String>,
    inside_examples: Vec<String>,
    /// Longest run of consecutive ticks a living player spent inside.
    max_inside_streak: u32,
}

fn run(c: &Content, map: u16, players: usize, seed: u64, frames: u32) -> Stats {
    let mut s = MatchSetup::ffa(players, seed);
    s.map_rotation = vec![MapId(map)];
    let mut w = World::new(c, &s);
    let mut bots = Bots::new(seed, players);
    let mut st = Stats::default();
    let mut was_battle = false;
    let mut streak = vec![0u32; players];
    for f in 0..frames {
        let inputs = bots.inputs(&w);
        w.step(c, &inputs);
        for e in w.events() {
            match e.kind {
                EventKind::CardPicked { .. } => st.picks += 1,
                EventKind::Died { .. } => st.deaths += 1,
                _ => {}
            }
        }
        let battle = w.phase() == Phase::Battle;
        if battle && !was_battle {
            st.battles += 1;
        }
        was_battle = battle;
        let statics = w.map_view().current.map(|m| &c.prepared(m).statics);
        for (i, p) in w.players().iter().enumerate() {
            assert!(
                p.pos.x.is_finite() && p.pos.y.is_finite(),
                "map {map} frame {f}: player {i} position {:?}",
                p.pos
            );
            if !(p.alive() && p.playing && p.simulated) {
                streak[i] = 0;
                continue;
            }
            st.player_ticks += 1;
            let a = p.pos.x.abs().max(p.pos.y.abs());
            st.max_abs = st.max_abs.max(a);
            if (p.pos.x.abs() > 60.0 || p.pos.y.abs() > 60.0) && st.outside.len() < 5 {
                st.outside
                    .push(format!("frame {f} player {i} at {:?}", p.pos));
            }
            if let Some(sts) = statics
                && sts.iter().any(|s| !s.trigger && s.shape.contains(p.pos))
            {
                streak[i] += 1;
                st.max_inside_streak = st.max_inside_streak.max(streak[i]);
                st.inside_static += 1;
                if st.inside_examples.len() < 6 && (st.inside_static % 40 == 1) {
                    st.inside_examples.push(format!(
                        "frame {f} player {i} at ({:.2}, {:.2})",
                        p.pos.x, p.pos.y
                    ));
                }
            } else {
                streak[i] = 0;
            }
        }
        if f % 250 == 249 {
            st.checksums.push(w.checksum());
        }
    }
    st
}

fn frames() -> u32 {
    std::env::var("ROUNDS_SMOKE_FRAMES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5000)
}

fn smoke(players: usize) {
    let Some(c) = content() else { return };
    let frames = frames();
    let results: Vec<(usize, Stats, Stats)> = (0..c.maps.len())
        .into_par_iter()
        .map(|m| {
            let seed = 0x5eed ^ (m as u64 * 7919) ^ players as u64;
            let a = run(c, m as u16, players, seed, frames);
            let b = run(c, m as u16, players, seed, frames);
            (m, a, b)
        })
        .collect();
    let mut failures = vec![];
    let (mut picks, mut deaths, mut battles) = (0, 0, 0);
    for (m, a, b) in &results {
        let name = &c.maps[*m].name;
        if a.checksums != b.checksums {
            failures.push(format!("{name}: non-deterministic checksums"));
        }
        if !a.outside.is_empty() {
            failures.push(format!(
                "{name}: players far outside the arena: {:?}",
                a.outside
            ));
        }
        if a.battles == 0 {
            failures.push(format!("{name}: never reached a battle"));
        }
        let inside = a.inside_static as f32 / a.player_ticks.max(1) as f32;
        if inside > 0.05 {
            // Reported, not fatal: deep penetration that the sim's player
            // push-out does not resolve is a sim collision issue, not content.
            println!(
                "WARN {name}: living players inside static geometry {:.1}% of ticks, e.g. {:?}",
                inside * 100.0,
                a.inside_examples
            );
        }
        println!(
            "{players:2}p {m:2} {name:24} battles {:2} picks {:3} deaths {:3} max|pos| {:5.1} inside {:5.2}% (max run {:3}) checksum {:016x}",
            a.battles,
            a.picks,
            a.deaths,
            a.max_abs,
            inside * 100.0,
            a.max_inside_streak,
            a.checksums.last().copied().unwrap_or(0)
        );
        picks += a.picks;
        deaths += a.deaths;
        battles += a.battles;
    }
    println!(
        "{players}p: {} maps x {frames} frames (x2 for determinism): {battles} battles, {picks} picks, {deaths} deaths, {} failures",
        results.len(),
        failures.len()
    );
    assert!(picks > 0, "no card was picked in any match");
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn smoke_4_players_every_map() {
    smoke(4);
}

#[test]
fn smoke_12_players_every_map() {
    smoke(12);
}
