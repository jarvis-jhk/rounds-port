//! Prints one player's state over a frame range of a smoke-test match
//! (same setup as `tests/smoke.rs`: arms race, rotation = that map).
//!
//! `cargo run --release -p rounds-content --example trace_player -- MAP PLAYERS SEED PLAYER FROM TO`
use rounds_content::bots::Bots;
use rounds_sim::{MapId, MatchSetup, World};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.len() < 6 {
        eprintln!("usage: trace_player MAP PLAYERS SEED PLAYER FROM TO");
        return;
    }
    let data = rounds_import::GameData::load(rounds_content::dev_import_dir()).expect("import");
    let mut built = rounds_content::build(&data).expect("content");
    let c = built.content();
    let map = c.maps.iter().position(|m| m.name == a[0]).expect("map") as u16;
    let n: usize = a[1].parse().unwrap();
    let seed: u64 = a[2].parse().unwrap();
    let pl: usize = a[3].parse().unwrap();
    let (from, to): (u32, u32) = (a[4].parse().unwrap(), a[5].parse().unwrap());
    let mut s = MatchSetup::ffa(n, seed);
    s.map_rotation = vec![MapId(map)];
    let mut w = World::new(&c, &s);
    let mut bots = Bots::new(seed, n);
    for f in 1..=to {
        let inp = bots.inputs(&w);
        w.step(&c, &inp);
        if f >= from {
            let p = w.player(pl);
            println!(
                "f{f} pos ({:.2},{:.2}) vel ({:.2},{:.2}) grounded {} wall {} playing {} kin {} hp {:.0} scale {:.2} {:?}",
                p.pos.x,
                p.pos.y,
                p.vel.x,
                p.vel.y,
                p.grounded,
                p.wall_grab,
                p.playing,
                p.kinematic,
                p.health,
                p.scale,
                w.phase()
            );
        }
    }
}
