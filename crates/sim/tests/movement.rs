//! Movement constants (player.md 5, 6).
mod common;
use common::*;
use rounds_sim::content::test_maps::FLOOR_Y;
use rounds_sim::{PlayerInput, World};

fn run(w: &mut World, c: &rounds_sim::Content, inp: PlayerInput, n: usize) {
    for _ in 0..n {
        w.step(c, &[inp]);
    }
}

#[test]
fn stands_at_equilibrium_height() {
    let c = content();
    let w = sandbox(&c, 0, 1);
    let p = w.player(0);
    let h = p.pos.y - FLOOR_Y;
    // player.md 5.6: centre rests ~0.74 * scale (~0.92) above the surface.
    assert!(
        (h - 0.74 * p.scale).abs() < 0.08,
        "height {h} scale {}",
        p.scale
    );
    assert!(p.grounded);
    assert!(p.vel.len() < 0.05, "{:?}", p.vel);
}

#[test]
fn top_speed_on_ground() {
    let c = content();
    let mut w = sandbox(&c, 0, 1);
    run(&mut w, &c, input(1.0, 0.0, 0), 60);
    let mut sum = 0.0;
    let mut max = 0.0f32;
    for _ in 0..60 {
        w.step(&c, &[input(1.0, 0.0, 0)]);
        sum += w.player(0).vel.x;
        max = max.max(w.player(0).vel.x);
    }
    let avg = sum / 60.0;
    // player.md 5.2: ~14.1 without the leg spring; the walk bob (5.6) dips
    // the body into the push-out radius and costs a little on average.
    assert!((max - 14.1).abs() < 0.2, "max vx {max}");
    assert!(avg > 13.0 && avg < 14.2, "avg vx {avg}");
}

#[test]
fn jump_apex_tap_and_hold() {
    let c = content();
    for (hold, expect, tol) in [(false, 3.2, 0.8), (true, 5.5, 1.0)] {
        let mut w = sandbox(&c, 0, 1);
        let y0 = w.player(0).pos.y;
        w.step(&c, &[input(0.0, 0.0, PlayerInput::BTN_JUMP)]);
        let held = if hold { PlayerInput::BTN_JUMP } else { 0 };
        let mut apex = y0;
        for _ in 0..150 {
            w.step(&c, &[input(0.0, 0.0, held)]);
            apex = apex.max(w.player(0).pos.y);
        }
        let h = apex - y0;
        assert!(
            (h - expect).abs() < tol,
            "hold={hold}: apex {h}, expected ~{expect}"
        );
    }
}
