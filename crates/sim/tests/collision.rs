//! Player push-out from static geometry (player.md 7.3): a player whose
//! centre ends up inside one or several overlapping colliders must be pushed
//! out, not held inside by the tunnelling guard or by contradicting pushes.
//! Regression for Tunnels_M (12 players, seed 144461): a spawn above a block
//! made of two overlapping boxes left a player stuck inside both.
mod common;
use common::input;
use rounds_sim::content::test_maps::{FLOOR_Y, flat_arena};
use rounds_sim::content::{ColliderDef, ContentDefs, StaticDef, Tuning};
use rounds_sim::geom::Shape;
use rounds_sim::math::{V2, v2};
use rounds_sim::{Content, GameMode, MapId, MatchSetup, PlayerInput, World};

fn obb(c: V2, half: V2) -> StaticDef {
    StaticDef {
        name: "Block".into(),
        id: String::new(),
        collider: ColliderDef::solid(Shape::Obb {
            c,
            axis: V2::RIGHT,
            half,
        }),
        damage_box: None,
    }
}

/// The flat test arena plus a block like Tunnels_M's: two overlapping boxes
/// (a tall narrow one and a wide flat one) with their tops at different
/// heights, floating above the floor.
fn content() -> Content {
    let mut m = flat_arena();
    m.name = "Test_Overlap".into();
    m.statics.push(obb(v2(0.0, -3.0), v2(2.68, 2.48)));
    m.statics.push(obb(v2(0.0, -3.0), v2(3.38, 1.78)));
    // An L-shaped pair: a centre near the seam has its nearest face inside the
    // other box.
    m.statics.push(obb(v2(-18.0, -4.0), v2(3.0, 1.0)));
    m.statics.push(obb(v2(-20.0, -2.0), v2(1.0, 3.0)));
    Content::new(ContentDefs {
        maps: vec![m],
        tuning: Tuning::default(),
        ..Default::default()
    })
}

fn world(c: &Content) -> World {
    let mut s = MatchSetup::ffa(1, 3);
    s.mode = GameMode::Sandbox;
    s.map_rotation = vec![MapId(0)];
    let mut w = World::new(c, &s);
    let idle = [PlayerInput::default()];
    for _ in 0..300 {
        w.step(c, &idle);
    }
    assert!(w.player(0).playing);
    w
}

fn inside_any(c: &Content, p: V2) -> bool {
    c.prepared(MapId(0))
        .statics
        .iter()
        .any(|s| !s.trigger && s.shape.contains(p))
}

/// Teleports the player into `at` with velocity `vel`, steps with `inp`, and
/// returns the number of ticks until the centre is outside every collider
/// (None if it never leaves within `ticks`), plus the final position.
fn escape(c: &Content, at: V2, vel: V2, inp: PlayerInput, ticks: u32) -> (Option<u32>, V2) {
    let mut w = world(c);
    w.teleport(0, at);
    w.player_mut(0).vel = vel;
    assert!(inside_any(c, at), "start {at:?} should be inside geometry");
    let mut out = None;
    for t in 0..ticks {
        w.step(c, &[inp]);
        let p = w.player(0).pos;
        if out.is_none() && !inside_any(c, p) {
            out = Some(t + 1);
        }
    }
    (out, w.player(0).pos)
}

#[test]
fn pushed_out_of_two_overlapping_boxes() {
    let c = content();
    // Inside both boxes, just under the lower top (the Tunnels_M state),
    // moving up fast, idle and walking.
    for (vel, inp) in [
        (v2(0.0, 50.0), PlayerInput::default()),
        (v2(3.0, 50.0), input(-1.0, 0.0, 0)),
        (V2::ZERO, PlayerInput::default()),
        (v2(0.0, -20.0), input(1.0, 0.0, 0)),
    ] {
        let (out, end) = escape(&c, v2(0.3, -1.75), vel, inp, 120);
        let t = out.unwrap_or_else(|| panic!("vel {vel:?}: still inside at {end:?}"));
        assert!(t <= 10, "vel {vel:?}: took {t} ticks");
        assert!(!inside_any(&c, end), "vel {vel:?}: ended inside at {end:?}");
    }
    // Deep inside both (centre of the block).
    let (out, end) = escape(&c, v2(0.0, -3.0), V2::ZERO, PlayerInput::default(), 120);
    assert!(out.is_some_and(|t| t <= 15), "deep: {out:?} at {end:?}");
    assert!(!inside_any(&c, end), "deep: ended inside at {end:?}");
}

#[test]
fn pushed_out_at_the_seam_of_an_l_shape() {
    let c = content();
    // Inside the horizontal bar near the vertical bar: the bar's nearest
    // face (its top) is inside the vertical bar.
    for at in [v2(-19.5, -3.4), v2(-20.5, -3.6), v2(-19.2, -4.5)] {
        let (out, end) = escape(&c, at, V2::ZERO, PlayerInput::default(), 120);
        assert!(out.is_some_and(|t| t <= 15), "{at:?}: {out:?} at {end:?}");
        assert!(!inside_any(&c, end), "{at:?}: ended inside at {end:?}");
    }
}

#[test]
fn standing_player_is_not_disturbed() {
    // The fix must not change normal ground contact.
    let c = content();
    let w = world(&c);
    let p = w.player(0);
    assert!(p.grounded && !inside_any(&c, p.pos));
    assert!(
        (p.pos.y - FLOOR_Y - 0.74 * p.scale).abs() < 0.1,
        "{:?}",
        p.pos
    );
}
