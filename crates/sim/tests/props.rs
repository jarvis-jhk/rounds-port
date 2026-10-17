//! Physics props and map scripts (maps.md 3-5).
mod common;
use common::*;
use rounds_sim::content::test_maps::FLOOR_Y;
use rounds_sim::{EventKind, PlayerInput, v2};

#[test]
fn props_settle_ropes_hold_platform_moves() {
    let c = content();
    let mut w = sandbox(&c, 2, 1);
    let idle = [PlayerInput::default()];
    let map = c.map(w.map_view().current.unwrap());
    let plat_idx = map
        .props
        .iter()
        .position(|p| p.move_sequence.is_some())
        .unwrap();
    let plank_idx = map.props.iter().position(|p| p.name == "Plank").unwrap();
    let start_x = w.props()[plat_idx].pos.x;
    for _ in 0..600 {
        w.step(&c, &idle);
    }
    // Crates rest on the floor.
    let crate0 = &w.props()[0];
    assert!(
        (crate0.pos.y - (FLOOR_Y + 1.0)).abs() < 0.2,
        "crate y {}",
        crate0.pos.y
    );
    assert!(crate0.vel.len() < 0.5);
    // The plank hangs from its ropes (no free fall).
    let plank = &w.props()[plank_idx];
    assert!(plank.pos.y > 4.0, "plank y {}", plank.pos.y);
    // The moving platform travelled toward its second waypoint (+10 x).
    let px = w.props()[plat_idx].pos.x;
    assert!(px > start_x + 3.0, "platform x {start_x} -> {px}");
}

#[test]
fn destructible_box_breaks_from_bullets() {
    let c = content();
    let mut w = sandbox(&c, 2, 1);
    let map = c.map(w.map_view().current.unwrap());
    let di = map
        .props
        .iter()
        .position(|p| p.destructible.is_some())
        .unwrap();
    for _ in 0..200 {
        w.step(&c, &[PlayerInput::default()]);
    }
    // Stand left of the box and shoot it twice (hp 50, 55 per bullet).
    let target = w.props()[di].pos;
    let y = w.player(0).pos.y;
    w.teleport(0, v2(target.x - 4.0, y));
    let mut destroyed = false;
    for k in 0..200 {
        let b = if k % 40 == 0 {
            PlayerInput::BTN_FIRE
        } else {
            0
        };
        w.step(&c, &[aim_input(0.0, 1.0, -0.05, b)]);
        destroyed |= w
            .events()
            .iter()
            .any(|e| matches!(e.kind, EventKind::PropDestroyed { .. }));
    }
    assert!(destroyed, "box destroyed");
    assert!(!w.props()[di].alive);
}

#[test]
fn saw_damages_and_knocks_back() {
    let c = content();
    let mut w = sandbox(&c, 2, 1);
    for _ in 0..100 {
        w.step(&c, &[PlayerInput::default()]);
    }
    w.teleport(0, v2(22.0, -0.4));
    let mut hit = false;
    for _ in 0..30 {
        w.step(&c, &[PlayerInput::default()]);
        hit |= w
            .events()
            .iter()
            .any(|e| matches!(e.kind, EventKind::SawHit { .. }));
    }
    assert!(hit);
    assert!(
        (w.player(0).health - 45.0).abs() < 1e-3 || w.player(0).health < 46.0,
        "health {}",
        w.player(0).health
    );
}

#[test]
fn player_pushes_crate() {
    let c = content();
    let mut w = sandbox(&c, 2, 1);
    for _ in 0..200 {
        w.step(&c, &[PlayerInput::default()]);
    }
    let x0 = w.props()[0].pos.x;
    let y = w.player(0).pos.y;
    w.teleport(0, v2(x0 - 2.0, y));
    for _ in 0..150 {
        w.step(&c, &[input(1.0, 0.0, 0)]);
    }
    let x1 = w.props()[0].pos.x;
    assert!(x1 > x0 + 0.5, "crate pushed {x0} -> {x1}");
}

#[test]
fn smasher_kills() {
    let c = content();
    let mut w = sandbox(&c, 3, 1);
    let y = w.player(0).pos.y;
    // Under the smasher pivot (0, 0): the kill box reaches down to ~ -7.
    w.teleport(0, v2(0.0, y));
    let mut died = false;
    for _ in 0..1000 {
        w.step(&c, &[PlayerInput::default()]);
        died |= w
            .events()
            .iter()
            .any(|e| matches!(e.kind, EventKind::Died { player: 0, .. }));
        if died {
            break;
        }
    }
    assert!(died, "smasher kill box kills the player");
}

#[test]
fn view_ropes_debris_and_feet() {
    use rounds_sim::physics::RopeAnchor;
    use rounds_sim::scene::Target;
    let c = content();
    let mut w = sandbox(&c, 2, 1);
    let map = c.map(w.map_view().current.unwrap());
    let plank = map.props.iter().position(|p| p.name == "Plank").unwrap() as u32;
    let ceiling = map
        .statics
        .iter()
        .position(|s| s.name == "Ceiling")
        .unwrap() as u32;
    // Ropes: start on the plank, end on the ceiling, world points follow the
    // plank's pose.
    let ropes = w.ropes();
    assert_eq!(ropes.len(), map.ropes.len());
    for r in &ropes {
        let def = &map.ropes[r.def as usize];
        assert_eq!(r.anchors[0], RopeAnchor::Prop(plank));
        assert_eq!(r.anchors[1], RopeAnchor::Static(Some(ceiling)));
        assert!(
            r.ends[1].dist(def.end) < 1e-4,
            "{:?} vs {:?}",
            r.ends,
            def.end
        );
        let p = &w.props()[plank as usize];
        assert!(r.ends[0].dist(p.pos) < 4.0);
        assert!(r.ends[0].y < r.ends[1].y);
    }
    // Feet: a player standing on the floor plants both feet on a static.
    let feet = w.foot_plants(&c, 0);
    for f in feet {
        let f = f.expect("foot planted");
        assert!(matches!(f.target, Target::Static(_)), "{:?}", f.target);
        assert!((f.point.y - FLOOR_Y).abs() < 0.05, "foot y {}", f.point.y);
        assert!(f.supporting);
        assert!(f.normal.y > 0.99);
    }
    let down = w.raycast(
        &c,
        w.player(0).pos,
        rounds_sim::V2::DOWN,
        5.0,
        rounds_sim::geom::layers::LEG_RAY,
        Some(0),
    );
    assert!(down.is_some());
    // Debris carries its source prop and piece index.
    let di = map
        .props
        .iter()
        .position(|p| p.destructible.is_some())
        .unwrap();
    let target = w.props()[di].pos;
    let y = w.player(0).pos.y;
    w.teleport(0, v2(target.x - 4.0, y));
    let mut seen = false;
    for k in 0..200 {
        let b = if k % 40 == 0 {
            PlayerInput::BTN_FIRE
        } else {
            0
        };
        w.step(&c, &[aim_input(0.0, 1.0, -0.05, b)]);
        if !w.debris().is_empty() {
            seen = true;
            let mut idx: Vec<u32> = w.debris().iter().map(|d| d.piece).collect();
            idx.sort();
            assert!(w.debris().iter().all(|d| d.prop as usize == di));
            assert!(idx.iter().all(|&i| (i as usize) < 4));
            break;
        }
    }
    assert!(seen, "debris spawned");
}
