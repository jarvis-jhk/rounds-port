//! Weapons (weapons.md): damage, knockback, block reflection, ammo, reload,
//! hit-spawned explosions, bounces.
mod common;
use common::*;
use rounds_sim::content::test_maps::FLOOR_Y;
use rounds_sim::{Content, EventKind, PlayerInput, World, v2};

/// Two players facing each other 6 units apart on the flat arena.
fn duel(c: &Content) -> World {
    let mut w = sandbox(c, 0, 2);
    let y = w.player(0).pos.y;
    w.teleport(1, v2(-14.0, y));
    w.teleport(0, v2(-20.0, y));
    for _ in 0..30 {
        w.step(c, &[PlayerInput::default(); 2]);
    }
    w
}

fn fire(aim_x: f32) -> PlayerInput {
    aim_input(0.0, aim_x, 0.0, PlayerInput::BTN_FIRE)
}

fn step_n(w: &mut World, c: &Content, a: PlayerInput, b: PlayerInput, n: usize) -> Vec<EventKind> {
    let mut ev = vec![];
    for _ in 0..n {
        w.step(c, &[a, b]);
        ev.extend(w.events().iter().map(|e| e.kind.clone()));
    }
    ev
}

#[test]
fn bullet_hits_for_55_and_knocks_back() {
    let c = content();
    let mut w = duel(&c);
    let idle = PlayerInput::default();
    step_n(&mut w, &c, fire(1.0), idle, 1);
    assert_eq!(w.projectiles().len(), 1);
    assert_eq!(w.player(0).gun.ammo, 2);
    let ev = step_n(&mut w, &c, idle, idle, 40);
    let hit = ev.iter().find_map(|e| match e {
        EventKind::ProjectileHit {
            damage,
            target: rounds_sim::events::HitTarget::Player(1),
            ..
        } => Some(*damage),
        _ => None,
    });
    assert_eq!(hit, Some(55.0));
    let p1 = w.player(1);
    assert!((p1.health - 45.0).abs() < 1e-3, "health {}", p1.health);
    assert!(w.projectiles().is_empty());
    // Knockback: v += forward * force / mass (force 5000 * (55/55)^2, mass 115.7) ~ 43 to the right.
    assert!(
        ev.iter()
            .any(|e| matches!(e, EventKind::Damaged { player: 1, .. }))
    );
}

#[test]
fn knockback_velocity() {
    let c = content();
    let mut w = duel(&c);
    let idle = PlayerInput::default();
    step_n(&mut w, &c, fire(1.0), idle, 1);
    let mut vmax = 0.0f32;
    for _ in 0..40 {
        w.step(&c, &[idle, idle]);
        vmax = vmax.max(w.player(1).vel.x);
        if w.player(1).health < 100.0 {
            break;
        }
    }
    let v = w.player(1).vel.x;
    let expect = 5000.0 / w.player(1).mass;
    assert!(
        v > expect * 0.8 && v < expect * 1.1,
        "knockback vx {v}, expected ~{expect}"
    );
}

#[test]
fn block_reflects_bullet_back_to_shooter() {
    let c = content();
    let mut w = duel(&c);
    let idle = PlayerInput::default();
    step_n(&mut w, &c, fire(1.0), idle, 1);
    // Bullet flies ~0.85 units/tick: block shortly before impact.
    step_n(&mut w, &c, idle, idle, 3);
    let block = input(0.0, 0.0, PlayerInput::BTN_BLOCK);
    let ev = step_n(&mut w, &c, idle, block, 1);
    assert!(
        ev.iter()
            .any(|e| matches!(e, EventKind::BlockStarted { player: 1, .. }))
    );
    let ev = step_n(&mut w, &c, idle, idle, 60);
    assert!(
        ev.iter()
            .any(|e| matches!(e, EventKind::ProjectileBlocked { player: 1, .. })),
        "bullet blocked"
    );
    assert_eq!(w.player(1).health, 100.0);
    assert!(
        w.player(0).health < 100.0,
        "reflected bullet hurts the shooter: {}",
        w.player(0).health
    );
    // Block cooldown: 4 s.
    assert!(w.player(1).block.counter < 4.0);
}

#[test]
fn ammo_and_reload() {
    let c = content();
    let mut w = duel(&c);
    let idle = PlayerInput::default();
    let up = fire(0.0);
    let up = PlayerInput { aim_y: 32767, ..up };
    for _ in 0..3 {
        step_n(&mut w, &c, up, idle, 1);
        step_n(&mut w, &c, idle, idle, 40); // > 0.3 game s cooldown
    }
    let g = &w.player(0).gun;
    assert!(g.reloading, "magazine of 3 empties");
    // A pull while reloading is lost (and cues "can't shoot").
    let ev = step_n(&mut w, &c, up, idle, 1);
    assert!(
        ev.iter()
            .any(|e| matches!(e, EventKind::CantShoot { player: 0 }))
    );
    // Reload 2 game s = 2 / 0.85 real s ~ 168 ticks (minus the ticks already waited).
    let ev = step_n(&mut w, &c, idle, idle, 170);
    assert!(
        ev.iter().any(|e| matches!(
            e,
            EventKind::ReloadDone {
                player: 0,
                rounds: 3
            }
        )),
        "reload done"
    );
    assert_eq!(w.player(0).gun.ammo, 3);
}

#[test]
fn explosive_card_spawns_explosion_on_wall_hit() {
    let c = content();
    let mut w = duel(&c);
    let card = c.card_by_name("Explosive").unwrap();
    w.debug_give_card(&c, 0, card);
    let idle = PlayerInput::default();
    // Shoot down into the floor.
    let down = PlayerInput {
        aim_y: -32767,
        ..fire(0.0)
    };
    step_n(&mut w, &c, down, idle, 1);
    let ev = step_n(&mut w, &c, idle, idle, 20);
    assert!(
        ev.iter().any(|e| matches!(e, EventKind::Explosion { .. })),
        "explosion on impact"
    );
    // The shooter stands right next to it: explosion damage 20 * falloff.
    assert!(w.player(0).health < 100.0);
}

#[test]
fn bouncy_bullet_reflects_off_floor() {
    let c = content();
    let mut w = duel(&c);
    let card = c.card_by_name("Bouncy").unwrap();
    w.debug_give_card(&c, 0, card);
    let idle = PlayerInput::default();
    let down = PlayerInput {
        aim_y: -32767,
        aim_x: 10000,
        ..fire(0.0)
    };
    step_n(&mut w, &c, down, idle, 1);
    let ev = step_n(&mut w, &c, idle, idle, 30);
    let bounces = ev
        .iter()
        .filter(|e| matches!(e, EventKind::ProjectileBounced { .. }))
        .count();
    assert!(bounces >= 1, "bounced {bounces}");
    assert!(w.projectiles().iter().all(|p| p.pos.y > FLOOR_Y - 0.5));
}

#[test]
fn damage_over_time_and_lifesteal() {
    let c = content();
    let mut w = duel(&c);
    let decay = c.card_by_name("Decay").unwrap();
    let leech = c.card_by_name("Leech").unwrap();
    w.debug_give_card(&c, 1, decay);
    w.debug_give_card(&c, 0, leech);
    let max1 = w.player(1).stats.max_health;
    let idle = PlayerInput::default();
    // Damage the shooter a bit so lifesteal can heal.
    let p0 = w.player(0).pos;
    w.take_damage(&c, 0, v2(0.0, 30.0), p0, None, true, true);
    let h0 = w.player(0).health;
    let pos = w.player(1).pos;
    w.take_damage(&c, 1, v2(40.0, 0.0), pos, Some(0), true, true);
    // First chunk immediately: 40 / 4 * 0.25 = 2.5.
    assert!(
        (w.player(1).health - (max1 - 2.5)).abs() < 1e-3,
        "{}",
        w.player(1).health
    );
    step_n(&mut w, &c, idle, idle, 400);
    assert!(
        (w.player(1).health - (max1 - 40.0)).abs() < 0.05,
        "all DoT chunks dealt: {}",
        w.player(1).health
    );
    // Lifesteal 0.75 of 40 = 30 healed.
    assert!(
        w.player(0).health > h0 + 29.0,
        "healed {} -> {}",
        h0,
        w.player(0).health
    );
}
