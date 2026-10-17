//! Lists the ticks where a living player's centre is inside static map
//! geometry, for a smoke-test match (same setup as `tests/smoke.rs`).
//!
//! `cargo run --release -p rounds-content --example inside_scan -- MAP PLAYERS [SEED] [FRAMES]`
//! (default seed: the smoke test's for that map).
use rounds_content::bots::Bots;
use rounds_sim::{MapId, MatchSetup, World};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.len() < 2 {
        eprintln!("usage: inside_scan MAP PLAYERS [SEED] [FRAMES]");
        return;
    }
    let data = rounds_import::GameData::load(rounds_content::dev_import_dir()).expect("import");
    let mut built = rounds_content::build(&data).expect("content");
    let c = built.content();
    let map = c.maps.iter().position(|m| m.name == a[0]).expect("map") as u16;
    let n: usize = a[1].parse().unwrap();
    let seed: u64 = a
        .get(2)
        .map(|s| s.parse().unwrap())
        .unwrap_or(0x5eed ^ (map as u64 * 7919) ^ n as u64);
    let frames: u32 = a.get(3).map(|s| s.parse().unwrap()).unwrap_or(5000);
    let mut s = MatchSetup::ffa(n, seed);
    s.map_rotation = vec![MapId(map)];
    let mut w = World::new(&c, &s);
    let mut bots = Bots::new(seed, n);
    for f in 0..frames {
        let inp = bots.inputs(&w);
        w.step(&c, &inp);
        let Some(m) = w.map_view().current else {
            continue;
        };
        let statics = &c.prepared(m).statics;
        for (i, p) in w.players().iter().enumerate() {
            if !(p.alive() && p.playing && p.simulated) {
                continue;
            }
            let hit: Vec<usize> = statics
                .iter()
                .enumerate()
                .filter(|(_, s)| !s.trigger && s.shape.contains(p.pos))
                .map(|(k, _)| k)
                .collect();
            if !hit.is_empty() {
                println!(
                    "f{f} p{i} pos ({:.2},{:.2}) vel ({:.2},{:.2}) kin {} grounded {} scale {:.2} statics {hit:?} {:?}",
                    p.pos.x,
                    p.pos.y,
                    p.vel.x,
                    p.vel.y,
                    p.kinematic,
                    p.grounded,
                    p.scale,
                    w.phase()
                );
            }
        }
    }
}
