//! Regression tests for the findings of docs/spec/review/sim-vs-spec.md that need
//! access to sim internals (one test per fix; the card-level ones live in
//! `tests/cards.rs`).

use crate::content::test_maps::{FLOOR_Y, test_content};
use crate::content::{Content, ExplosionDef, MapId};
use crate::events::EventKind;
use crate::input::PlayerInput;
use crate::math::{V2, v2};
use crate::objects::ExplodeHooks;
use crate::player::Pull;
use crate::script::{ChargeState, CompState, Kind};
use crate::world::{DT, GameMode, MatchSetup, World, ticks};

fn sandbox(c: &Content, map: u16, n: usize) -> World {
    let mut s = MatchSetup::ffa(n, 7);
    s.mode = GameMode::Sandbox;
    s.map_rotation = vec![MapId(map)];
    let mut w = World::new(c, &s);
    steps(&mut w, c, 300);
    assert!(w.players.iter().all(|p| p.playing));
    w
}

fn steps(w: &mut World, c: &Content, n: usize) -> Vec<EventKind> {
    let idle = vec![PlayerInput::default(); w.players.len()];
    let mut ev = Vec::new();
    for _ in 0..n {
        w.step(c, &idle);
        ev.extend(w.events().iter().map(|e| e.kind.clone()));
    }
    ev
}

/// M1: with vision required, a nearer enemy behind a wall does not hide a
/// visible one (cards.md 5.5).
#[test]
fn m1_targeting_picks_nearest_visible() {
    let c = test_content();
    let mut w = sandbox(&c, 1, 3);
    // PlatTop spans y 3.5..4.5 around x 0.
    w.teleport(0, v2(-25.0, FLOOR_Y + 1.0));
    w.teleport(1, v2(0.0, 1.5));
    w.teleport(2, v2(7.5, 8.0));
    let from = v2(0.0, 8.0);
    assert_eq!(w.closest_enemy(&c, from, 0, false), Some(1));
    assert_eq!(w.closest_enemy(&c, from, 0, true), Some(2));
    assert_eq!(w.closest_player(&c, from, true), Some(2));
    // Nobody visible: None.
    w.teleport(2, v2(7.5, 1.5));
    assert_eq!(w.closest_enemy(&c, from, 0, true), None);
}

/// M2: explosions push props through `BulletPush` (x bulletPushMultiplier)
/// plus the plain rigid-body impulse (weapons.md 19.1).
#[test]
fn m2_explosion_prop_impulse_uses_push_multiplier() {
    let c = test_content();
    let mut w = sandbox(&c, 2, 1);
    w.teleport(0, v2(25.0, FLOOR_Y + 1.0));
    steps(&mut w, &c, 200);
    let crate_pos = w.physics.props[0].pos;
    let v0 = w.physics.props[0].vel;
    assert!(v0.len() < 0.1);
    let e = ExplosionDef {
        damage: 0.0,
        force: 2000.0,
        range: 4.0,
        object_force_multiplier: 1.0,
        ignore_walls: true,
        ..Default::default()
    };
    // 1.5 from the crate's left face: falloff 0.625. (Friction and drag
    // take a bit of the first step; without the multiplier vx is ~0.1.)
    let centre = crate_pos - v2(2.5, 0.0);
    w.explode_with(&c, centre, &e, 1.0, None, None, 0, ExplodeHooks::default());
    steps(&mut w, &c, 1);
    let mass = 20000.0;
    let expect = (20.0 + 1.0) * 0.625 * 2000.0 / mass;
    let vx = w.physics.props[0].vel.x;
    assert!(
        vx > expect * 0.5 && vx < expect * 1.1,
        "crate vx {vx}, expected about {expect}"
    );
}

/// L2: Shield Charge recoil: `knockBack` respects mass, only `stopForce`
/// ignores it; collisions count while `blockTime` lasts (cards.md 6.4).
#[test]
fn l2_shield_charge_recoil_and_window() {
    let Some(rc) = crate::test_fixture::real() else {
        eprintln!("no import found: skipped");
        return;
    };
    let c = &rc.content;
    let mut w = sandbox(c, 0, 2);
    w.teleport(0, v2(-10.0, FLOOR_Y + 1.0));
    w.teleport(1, v2(-8.5, FLOOR_Y + 1.0));
    steps(&mut w, c, 40);
    w.debug_give_card(c, 0, rc.card("Shield Charge"));
    let (oi, ci) = w.players[0]
        .effects
        .clone()
        .into_iter()
        .find_map(|id| {
            let oi = w.obj_index(id)?;
            let pf = c.prefab(w.objects[oi].def);
            let ci = pf
                .comps
                .iter()
                .position(|k| matches!(k.kind, Kind::ShieldCharge(_)))?;
            Some((oi, ci))
        })
        .expect("shield charge component");
    let pf = c.prefab(w.objects[oi].def);
    let Kind::ShieldCharge(d) = &pf.comps[ci].kind else {
        unreachable!()
    };
    let slot = pf.comps[ci].slot;
    let state = |active: bool, block_time: f32| ChargeState {
        active,
        c: 0.5,
        used_time: 0.2,
        block_time,
        dir: v2(1.0, 0.0),
        force: 0.0,
        damage: 0.0,
        knockback: 0.0,
        hits: 0,
        cancelled: false,
        wait: 0,
        from: V2::ZERO,
    };
    w.set_slot(oi, slot, CompState::Charge(state(true, 0.1)));
    let v0 = w.players[0].vel;
    let m = w.players[0].mass;
    w.charge_collide(c, oi, ci, 0, 1, d);
    let dv = w.players[0].vel.x - v0.x;
    let expect = -(d.knockback / m + d.stop_force / 100.0);
    assert!(
        (dv - expect).abs() < 1e-3 * expect.abs(),
        "recoil {dv}, expected {expect} (mass {m})"
    );
    // After the force loop (inactive) but while blockTime >= 0: still hits.
    w.events.begin(w.frame);
    w.set_slot(oi, slot, CompState::Charge(state(false, 0.05)));
    w.charge_collide(c, oi, ci, 0, 1, d);
    assert!(
        w.events()
            .iter()
            .any(|e| matches!(e.kind, EventKind::ShieldChargeHit { .. }))
    );
    // blockTime run out: no hit.
    w.events.begin(w.frame);
    w.set_slot(oi, slot, CompState::Charge(state(false, -0.01)));
    w.charge_collide(c, oi, ci, 0, 1, d);
    assert!(
        !w.events()
            .iter()
            .any(|e| matches!(e.kind, EventKind::ShieldChargeHit { .. }))
    );
}

/// L3: the Implosion drag term uses `fixed_dt = 0.014 * ts`.
#[test]
fn l3_implosion_drag_uses_scaled_fixed_dt() {
    let c = test_content();
    let mut w = sandbox(&c, 0, 1);
    let pos = w.players[0].pos;
    w.players[0].vel = v2(10.0, 0.0);
    w.players[0].pulls.push(Pull {
        point: pos,
        force: 0.0,
        clamp: 1.0,
        drag: 10.0,
        left: 1.0,
    });
    let ts = w.time_scale();
    let m = w.players[0].mass;
    w.pulls_tick(DT * ts);
    let f = -10.0 * 10.0 * (DT * ts) * 10.0 * ts;
    let expect = 10.0 + f * 0.02 / m;
    assert!(
        (w.players[0].vel.x - expect).abs() < 1e-5,
        "vx {} expected {expect}",
        w.players[0].vel.x
    );
}

/// L4: taking damage ends running pulls (`StopAllCoroutines`, player.md 11.3).
#[test]
fn l4_damage_cancels_pulls() {
    let c = test_content();
    let mut w = sandbox(&c, 0, 1);
    let pos = w.players[0].pos;
    w.players[0].pulls.push(Pull {
        point: pos + v2(5.0, 0.0),
        force: 1000.0,
        clamp: 1.0,
        drag: 10.0,
        left: 0.2,
    });
    w.do_damage(&c, 0, v2(0.0, -5.0), pos, None, true, false);
    assert!(w.players[0].pulls.is_empty());
}

/// L5: a bullet's `sinceReflect` runs on the game clock.
#[test]
fn l5_since_reflect_is_game_time() {
    let c = test_content();
    let mut w = sandbox(&c, 0, 1);
    let fire = PlayerInput::from_axes(v2(0.0, 0.0), v2(1.0, 0.2), PlayerInput::BTN_FIRE);
    w.step(&c, &[fire]);
    steps(&mut w, &c, 2);
    assert!(!w.projectiles.is_empty());
    let s1 = w.projectiles[0].since_reflect;
    steps(&mut w, &c, 1);
    let s2 = w.projectiles[0].since_reflect;
    let ts = w.time_scale();
    assert!((s2 - s1 - DT * ts).abs() < 1e-5, "{s1} -> {s2}");
}

/// L6: the gun cooldown advances while the player is not simulated.
#[test]
fn l6_gun_cooldown_runs_while_unsimulated() {
    let c = test_content();
    let mut w = sandbox(&c, 0, 1);
    w.players[0].simulated = false;
    w.players[0].gun.since_attack = 0.0;
    steps(&mut w, &c, 1);
    let expect = DT * w.time_scale() * w.players[0].gun.attack_speed_multiplier;
    assert!(
        (w.players[0].gun.since_attack - expect).abs() < 1e-6,
        "{}",
        w.players[0].gun.since_attack
    );
}

/// L7: legs keep stepping in the air (IkLeg.DoStep is not gated on ground).
#[test]
fn l7_legs_step_while_airborne() {
    let c = test_content();
    let mut w = sandbox(&c, 0, 1);
    w.teleport(0, v2(0.0, 5.0));
    steps(&mut w, &c, 3);
    assert!(!w.players[0].grounded);
    assert!(w.players[0].legs.iter().all(|l| !l.stepping));
    w.players[0].vel.x = 6.0;
    steps(&mut w, &c, 1);
    assert!(w.players[0].legs.iter().any(|l| l.stepping));
}

/// L7: the leg rays run during the spawn flight: hovering on the spawn
/// point sets the ground flag and refills the jumps.
#[test]
fn l7_ground_contact_during_spawn_hold() {
    let c = test_content();
    let mut s = MatchSetup::ffa(2, 3);
    s.pick_phase = false;
    s.map_rotation = vec![MapId(0)];
    let mut w = World::new(&c, &s);
    let mut seen = false;
    for _ in 0..400 {
        w.players[0].current_jumps = 0;
        steps(&mut w, &c, 1);
        let p = &w.players[0];
        if let Some(sm) = p.spawn_move
            && sm.c >= 0.9
            && sm.hold < 8
        {
            assert!(p.grounded, "grounded while hovering on the spawn point");
            assert_eq!(p.current_jumps, p.stats.jumps);
            seen = true;
        }
    }
    assert!(seen, "spawn hold observed");
}

/// L8: a DoT chunk's wait is fixed in real ticks when the chunk is dealt; a
/// slow motion starting during the wait does not stretch it.
#[test]
fn l8_dot_wait_is_real_time() {
    let c = test_content();
    let mut w = sandbox(&c, 0, 1);
    let ts = w.time_scale();
    w.take_damage_over_time(&c, 0, v2(0.0, -10.0), 1.0, 0.25, None);
    assert_eq!(w.players[0].dots[0].wait_ticks, ticks(0.25 / ts));
    w.time.slow_down();
    let mut at = None;
    for k in 1..=60u32 {
        let ev = steps(&mut w, &c, 1);
        if ev
            .iter()
            .any(|e| matches!(e, EventKind::Damaged { player: 0, .. }))
        {
            at = Some(k);
            break;
        }
    }
    assert_eq!(at, Some(ticks(0.25 / ts)));
}

/// L9: one damage cooldown per saw, shared by all targets (maps.md 5.1).
#[test]
fn l9_saw_cooldown_is_per_saw() {
    let c = test_content();
    let mut w = sandbox(&c, 2, 2);
    // The saw: circle (22, -2) r 1.15.
    w.teleport(0, v2(20.5, -2.0));
    w.teleport(1, v2(23.5, -2.0));
    let ev = steps(&mut w, &c, 1);
    let hits = ev
        .iter()
        .filter(|e| matches!(e, EventKind::SawHit { .. }))
        .count();
    assert_eq!(hits, 1);
}

/// L10: the 0.15 prop-collision circle follows size changes.
#[test]
fn l10_prop_circle_radius_follows_scale() {
    let c = test_content();
    let mut w = sandbox(&c, 2, 1);
    let r0 = w.physics.player_circle_radius(0).expect("player circle");
    assert!((r0 - 0.15 * w.players[0].scale).abs() < 1e-5);
    w.players[0].scale = 2.0;
    steps(&mut w, &c, 1);
    let r1 = w.physics.player_circle_radius(0).unwrap();
    assert!((r1 - 0.3).abs() < 1e-5, "radius {r1}");
}

/// L11: explosion candidates are tested against the player's circle, not
/// its bounding box.
#[test]
fn l11_explosion_uses_the_real_shape() {
    let c = test_content();
    let mut w = sandbox(&c, 0, 1);
    let p = w.players[0].pos;
    let rad = w.players[0].radius();
    let r = 1.0f32;
    let s2 = core::f32::consts::SQRT_2;
    // Diagonal offset: AABB corner within range, circle outside.
    let k = ((r + rad) / s2 + rad + r / s2) * 0.5;
    let e = ExplosionDef {
        damage: 10.0,
        range: r,
        ignore_walls: true,
        ..Default::default()
    };
    w.events.begin(w.frame);
    w.explode_with(
        &c,
        p + v2(k, k),
        &e,
        1.0,
        None,
        None,
        0,
        ExplodeHooks::default(),
    );
    assert!(
        !w.events()
            .iter()
            .any(|e| matches!(e.kind, EventKind::Damaged { .. })),
        "outside the circle: no damage"
    );
    // Straight to the side within r + rad: hit.
    w.explode_with(
        &c,
        p + v2(r + rad - 0.05, 0.0),
        &e,
        1.0,
        None,
        None,
        0,
        ExplodeHooks::default(),
    );
    assert!(
        w.events()
            .iter()
            .any(|e| matches!(e.kind, EventKind::Damaged { .. }))
    );
}

/// L12: a blocked bullet shakes the screen with magnitude 1.
#[test]
fn l12_blocked_bullet_shake_magnitude() {
    let c = test_content();
    let mut w = sandbox(&c, 0, 2);
    let y = FLOOR_Y + 1.0;
    w.teleport(0, v2(-10.0, y));
    w.teleport(1, v2(-5.0, y));
    steps(&mut w, &c, 30);
    let fire = PlayerInput::from_axes(v2(0.0, 0.0), v2(1.0, 0.0), PlayerInput::BTN_FIRE);
    let block = PlayerInput::from_axes(v2(0.0, 0.0), v2(0.0, 0.0), PlayerInput::BTN_BLOCK);
    w.step(&c, &[fire, block]);
    let mut found = false;
    for _ in 0..20 {
        let ev = steps(&mut w, &c, 1);
        if ev
            .iter()
            .any(|e| matches!(e, EventKind::ProjectileBlocked { player: 1, .. }))
        {
            found = true;
            assert!(
                ev.iter().any(
                    |e| matches!(e, EventKind::Shake { vec } if (vec.len() - 1.0).abs() < 1e-4)
                )
            );
            break;
        }
    }
    assert!(found, "bullet blocked");
}
