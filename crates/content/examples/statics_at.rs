//! Lists the static colliders of a map near a point (collision debugging).
//!
//! `cargo run --release -p rounds-content --example statics_at -- MAP X Y [R]`
use rounds_sim::math::v2;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.len() < 3 {
        eprintln!("usage: statics_at MAP X Y [R]");
        return;
    }
    let data = rounds_import::GameData::load(rounds_content::dev_import_dir()).expect("import");
    let mut built = rounds_content::build(&data).expect("content");
    let c = built.content();
    let map = c.maps.iter().position(|m| m.name == a[0]).expect("map") as u16;
    let (x, y): (f32, f32) = (a[1].parse().unwrap(), a[2].parse().unwrap());
    let r: f32 = a.get(3).map(|s| s.parse().unwrap()).unwrap_or(1.5);
    let p = v2(x, y);
    let pm = c.prepared(rounds_sim::MapId(map));
    for (i, s) in pm.statics.iter().enumerate() {
        let (q, inside) = s.shape.closest_surface_point(p);
        let d = (q - p).len();
        if inside || d < r {
            println!(
                "static {i}: layer {} trigger {} inside {inside} closest ({:.3},{:.3}) d {d:.3} {:?}",
                s.layer, s.trigger, q.x, q.y, s.shape
            );
        }
    }
}
