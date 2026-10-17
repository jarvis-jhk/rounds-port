//! M5: every card with the real imported data (skipped without an import).
//! One test per card (or card family) checks the card's specific effect.
mod common;
mod fixture;

use common::*;
use fixture::{RealContent, real};
use rounds_sim::content::test_maps::FLOOR_Y;
use rounds_sim::events::{BlockKind, HitTarget};
use rounds_sim::{Content, EventKind, PlayerInput, V2, World, v2};

macro_rules! need_data {
    () => {
        match real() {
            Some(r) => r,
            None => {
                eprintln!("no import found: skipped");
                return;
            }
        }
    };
}

const IDLE: PlayerInput = PlayerInput {
    move_x: 0,
    move_y: 0,
    aim_x: 0,
    aim_y: 0,
    buttons: 0,
};

/// `n` players standing on the flat arena (sandbox), at `xs`.
fn arena(rc: &RealContent, xs: &[f32]) -> World {
    let c = &rc.content;
    let mut w = sandbox(c, 0, xs.len());
    let y = FLOOR_Y + 1.0;
    for (i, x) in xs.iter().enumerate() {
        w.teleport(i, v2(*x, y));
    }
    settle(&mut w, c, 40);
    w
}

fn settle(w: &mut World, c: &Content, n: usize) {
    let idle = vec![IDLE; w.players().len()];
    for _ in 0..n {
        w.step(c, &idle);
    }
}

fn give(w: &mut World, rc: &RealContent, p: usize, card: &str) {
    let id = rc.card(card);
    w.debug_give_card(&rc.content, p, id);
}

/// Step with per-player inputs; collect events.
fn run(w: &mut World, c: &Content, inputs: &[PlayerInput], n: usize) -> Vec<EventKind> {
    let mut ev = Vec::new();
    for _ in 0..n {
        w.step(c, inputs);
        ev.extend(w.events().iter().map(|e| e.kind.clone()));
    }
    ev
}

fn press(buttons: u16) -> PlayerInput {
    PlayerInput { buttons, ..IDLE }
}

fn aim(ax: f32, ay: f32, buttons: u16) -> PlayerInput {
    aim_input(0.0, ax, ay, buttons)
}

/// Inputs for `n` players: `p` gets `inp`, others idle.
fn only(n: usize, p: usize, inp: PlayerInput) -> Vec<PlayerInput> {
    let mut v = vec![IDLE; n];
    v[p] = inp;
    v
}

/// Block once with player `p` (press, then release for `after` ticks).
fn block(w: &mut World, c: &Content, p: usize, after: usize) -> Vec<EventKind> {
    let n = w.players().len();
    let mut ev = run(w, c, &only(n, p, press(PlayerInput::BTN_BLOCK)), 1);
    ev.extend(run(w, c, &vec![IDLE; n], after));
    ev
}

/// Fire once with player `p` aiming `dir`, then idle `after` ticks.
fn shoot(w: &mut World, c: &Content, p: usize, dir: V2, after: usize) -> Vec<EventKind> {
    let n = w.players().len();
    let mut ev = run(
        w,
        c,
        &only(n, p, aim(dir.x, dir.y, PlayerInput::BTN_FIRE)),
        1,
    );
    ev.extend(run(w, c, &only(n, p, aim(dir.x, dir.y, 0)), after));
    ev
}

fn damage_to(ev: &[EventKind], p: u8) -> f32 {
    ev.iter()
        .map(|e| match e {
            EventKind::Damaged { player, amount, .. } if *player == p => *amount,
            _ => 0.0,
        })
        .sum()
}

#[test]
fn every_card_applies_and_content_compiles() {
    let rc = need_data!();
    let c = &rc.content;
    for w in &c.warnings {
        eprintln!("warning: {w}");
    }
    assert_eq!(c.cards.len(), 79);
    assert_eq!(c.pool.len(), 67);
    for (i, _) in c.cards.iter().enumerate() {
        let mut w = arena(rc, &[-10.0, 10.0]);
        w.debug_give_card(c, 0, rounds_sim::CardId(i as u16));
        w.debug_give_card(c, 0, rounds_sim::CardId(i as u16));
        let inputs = [aim(1.0, 0.2, PlayerInput::BTN_FIRE), IDLE];
        run(&mut w, c, &inputs, 5);
        block(&mut w, c, 0, 120);
        shoot(&mut w, c, 0, v2(1.0, 0.1), 200);
    }
}

fn obj(c: &Content, name: &str) -> rounds_sim::ObjectId {
    c.object_by_name(name)
        .unwrap_or_else(|| panic!("no object {name}"))
}

#[test]
fn prefabs_compile_with_wiring() {
    use rounds_sim::script::{Kind, Method, Target};
    let rc = need_data!();
    let c = &rc.content;
    let saw = c.prefab(obj(c, "E_NewSaw"));
    assert!(saw.follows);
    assert!(saw.comps.iter().any(|k| matches!(k.kind, Kind::Saw { .. })));
    let start = saw
        .comps
        .iter()
        .find_map(|k| match &k.kind {
            Kind::StartEvent(ev) => Some(ev.clone()),
            _ => None,
        })
        .unwrap();
    assert!(start.iter().any(|call| call.method == Method::SetEnabled(true)
        && matches!(call.target, Target::Comp(i) if matches!(saw.comps[i as usize].kind, Kind::Saw{..}))));
    // Abyssal saw starts disabled, remnant trigger starts disabled.
    let ab = c.prefab(obj(c, "A_AbyssalCountdown"));
    let s = ab
        .comps
        .iter()
        .find(|k| matches!(k.kind, Kind::Saw { .. }))
        .unwrap();
    assert!(!s.enabled);
    let objs = ab
        .comps
        .iter()
        .find_map(|k| match &k.kind {
            Kind::AbyssalCountdown { objects, .. } => Some(objects.len()),
            _ => None,
        })
        .unwrap();
    assert_eq!(objs, 3);
    let rem = c.prefab(obj(c, "E_StaticRemnant"));
    assert!(
        rem.comps
            .iter()
            .any(|k| matches!(k.kind, Kind::PlayerInRangeTrigger { .. }) && !k.enabled)
    );
    // Effect gun of EMP resolves its projectile.
    let emp = c.prefab(obj(c, "A_EMP"));
    let g = emp
        .comps
        .iter()
        .find_map(|k| match &k.kind {
            Kind::Gun(g) if g.projectile != 0 => Some(g),
            _ => None,
        })
        .expect("EMP gun with Bullet_EMP");
    assert_eq!(c.projectiles[g.projectile as usize].name, "Bullet_EMP");
    // Add-ons.
    assert!(!c.addons(obj(c, "A_Homing")).is_empty());
    assert!(!c.addons(obj(c, "A_DrillAmmo")).is_empty());
}

#[test]
fn teleport_moves_the_player_on_block() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-10.0, 20.0]);
    give(&mut w, rc, 0, "Teleport");
    run(&mut w, c, &[aim(1.0, 0.0, 0), IDLE], 3);
    let x0 = w.player(0).pos.x;
    let ev = run(&mut w, c, &[aim(1.0, 0.0, PlayerInput::BTN_BLOCK), IDLE], 1);
    let x1 = w.player(0).pos.x;
    assert!(
        ev.iter()
            .any(|e| matches!(e, EventKind::Teleported { player: 0, .. }))
    );
    // 10 samples of distance*level/10 along the aim (6 units).
    assert!((x1 - x0 - 6.0).abs() < 0.3, "moved {}", x1 - x0);
}

#[test]
fn saw_cuts_nearby_enemy_after_block() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-10.0, -7.0]);
    give(&mut w, rc, 0, "Saw");
    let ev = block(&mut w, c, 0, 200);
    let d = damage_to(&ev, 1);
    assert!(d > 30.0, "saw damage {d}");
    assert!(ev.iter().any(|e| matches!(e, EventKind::SawHit { .. })));
    // The saw object stopped cutting after its 2 s.
    let ev = run(&mut w, c, &[IDLE, IDLE], 100);
    assert_eq!(damage_to(&ev, 1), 0.0);
}

#[test]
fn poison_turns_damage_into_dot() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-10.0, -4.0]);
    give(&mut w, rc, 0, "Poison bullets");
    let dmg = w.player(0).gun.stats.damage * 55.0;
    let ev = shoot(&mut w, c, 0, v2(1.0, 0.0), 400);
    let hits: Vec<f32> = ev
        .iter()
        .filter_map(|e| match e {
            EventKind::Damaged {
                player: 1, amount, ..
            } => Some(*amount),
            _ => None,
        })
        .collect();
    assert!(hits.len() > 5, "dot ticks {hits:?}");
    let total: f32 = hits.iter().sum();
    assert!(
        (total - (dmg + 1.0)).abs() < dmg * 0.15 + 1.0,
        "total {total} vs {dmg}"
    );
}

#[test]
fn abyssal_countdown_after_standing_still() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-10.0, 20.0]);
    give(&mut w, rc, 0, "AbyssalCountdown");
    let hp = w.player(0).stats.max_health;
    // 1 s start delay + 7 s fill (game time at 0.85).
    let ev = run(&mut w, c, &[IDLE, IDLE], 720);
    assert!(
        ev.iter()
            .any(|e| matches!(e, EventKind::AbyssalEntered { player: 0 }))
    );
    assert!((w.player(0).stats.max_health - hp * 2.0).abs() < 1.0);
    // The form ends after its duration.
    let ev = run(&mut w, c, &[IDLE, IDLE], 520);
    assert!(
        ev.iter()
            .any(|e| matches!(e, EventKind::AbyssalLeft { player: 0 }))
    );
    assert!((w.player(0).stats.max_health - hp).abs() < 1.0);
    // Moving prevents the fill.
    let mut w = arena(rc, &[-10.0, 20.0]);
    give(&mut w, rc, 0, "AbyssalCountdown");
    let mv = input(1.0, 0.0, 0);
    let back = input(-1.0, 0.0, 0);
    let mut ev = vec![];
    for k in 0..12 {
        ev.extend(run(
            &mut w,
            c,
            &[if k % 2 == 0 { mv } else { back }, IDLE],
            60,
        ));
    }
    assert!(
        !ev.iter()
            .any(|e| matches!(e, EventKind::AbyssalEntered { .. }))
    );
}

#[test]
fn phoenix_revives_after_lethal_damage() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-10.0, 10.0]);
    give(&mut w, rc, 1, "Phoenix");
    let pos = w.player(1).pos;
    w.take_damage(c, 1, V2::UP * 1000.0, pos, Some(0), true, true);
    let ev = run(&mut w, c, &[IDLE, IDLE], 1);
    assert!(w.player(1).respawning && !w.player(1).dead);
    let _ = ev;
    let ev = run(&mut w, c, &[IDLE, IDLE], 200);
    assert!(ev.iter().any(|e| matches!(
        e,
        EventKind::Revived {
            player: 1,
            full: false
        }
    )));
    assert!(!w.player(1).respawning && !w.player(1).dead);
    assert_eq!(w.player(1).health, w.player(1).stats.max_health);
    // The free block on revive.
    assert!(
        ev.iter()
            .any(|e| matches!(e, EventKind::BlockStarted { player: 1, .. }))
    );
    // Second lethal hit kills for good.
    let pos = w.player(1).pos;
    w.take_damage(c, 1, V2::UP * 10000.0, pos, Some(0), true, true);
    run(&mut w, c, &[IDLE, IDLE], 2);
    assert!(w.player(1).dead);
}

#[test]
fn empower_next_shot_triggers_block_effects_at_impact() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-10.0, 25.0]);
    give(&mut w, rc, 0, "Empower");
    give(&mut w, rc, 0, "Teleport");
    run(&mut w, c, &[aim(1.0, 0.0, 0), IDLE], 3);
    let ev = run(
        &mut w,
        c,
        &[aim(-1.0, 0.0, PlayerInput::BTN_BLOCK), IDLE],
        1,
    );
    assert!(
        ev.iter()
            .any(|e| matches!(e, EventKind::EmpowerArmed { player: 0 }))
    );
    run(&mut w, c, &[aim(1.0, 0.0, 0), IDLE], 60);
    let x0 = w.player(0).pos.x;
    let ev = shoot(&mut w, c, 0, v2(1.0, 0.05), 120);
    assert!(
        ev.iter()
            .any(|e| matches!(e, EventKind::EmpowerFired { player: 0, .. }))
    );
    let at = ev.iter().find_map(|e| match e {
        EventKind::EmpowerTriggered { player: 0, pos } => Some(*pos),
        _ => None,
    });
    let at = at.expect("empower triggered at impact");
    // Teleport brought the shooter to the impact point (the right wall).
    let x1 = w.player(0).pos.x;
    assert!(x1 > x0 + 20.0, "x {x0} -> {x1}, impact {at:?}");
}

fn count<F: Fn(&EventKind) -> bool>(ev: &[EventKind], f: F) -> usize {
    ev.iter().filter(|e| f(e)).count()
}

#[test]
fn shield_charge_dashes_into_enemy() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-10.0, -4.0]);
    give(&mut w, rc, 0, "Shield Charge");
    run(&mut w, c, &[aim(1.0, 0.0, 0), IDLE], 3);
    let x0 = w.player(0).pos.x;
    let ev = run(&mut w, c, &[aim(1.0, 0.0, PlayerInput::BTN_BLOCK), IDLE], 1);
    assert!(
        ev.iter()
            .any(|e| matches!(e, EventKind::ShieldChargeStart { player: 0, .. }))
    );
    let ev = run(&mut w, c, &[aim(1.0, 0.0, 0), IDLE], 40);
    assert!(
        ev.iter().any(|e| matches!(
            e,
            EventKind::ShieldChargeHit {
                player: 0,
                target: 1,
                ..
            }
        )),
        "x {x0} -> {}",
        w.player(0).pos.x
    );
    assert!(
        (damage_to(&ev, 1) - 25.0).abs() < 1.0,
        "{}",
        damage_to(&ev, 1)
    );
}

#[test]
fn emp_fires_a_ring_that_slows() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-10.0, -5.0]);
    give(&mut w, rc, 0, "EMP");
    let ev = run(&mut w, c, &[press(PlayerInput::BTN_BLOCK), IDLE], 1);
    let n = ev.iter().find_map(|e| match e {
        EventKind::RingFired {
            player: 0, count, ..
        } => Some(*count),
        _ => None,
    });
    assert_eq!(n, Some(30));
    assert!(w.projectiles().len() >= 30);
    let ev = run(&mut w, c, &[IDLE, IDLE], 60);
    assert!(
        ev.iter()
            .any(|e| matches!(e, EventKind::Slowed { player: 1, .. }))
    );
    // A second pick doubles the ring.
    give(&mut w, rc, 0, "EMP");
    run(&mut w, c, &[IDLE, IDLE], 400);
    let ev = run(&mut w, c, &[press(PlayerInput::BTN_BLOCK), IDLE], 1);
    assert!(
        ev.iter()
            .any(|e| matches!(e, EventKind::RingFired { count: 60, .. }))
    );
}

#[test]
fn bombs_away_jumps_and_drops_sticky_bombs() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-10.0, -7.0]);
    give(&mut w, rc, 0, "BombsAway");
    let y0 = w.player(0).pos.y;
    let ev = run(&mut w, c, &[press(PlayerInput::BTN_BLOCK), IDLE], 1);
    assert!(
        ev.iter()
            .any(|e| matches!(e, EventKind::RingFired { count: 6, .. }))
    );
    assert!(
        ev.iter()
            .any(|e| matches!(e, EventKind::Jumped { player: 0, .. }))
    );
    let ev = run(&mut w, c, &[IDLE, IDLE], 200);
    assert!(w.player(0).pos.y >= y0 - 0.5);
    let bombs = count(&ev, |e| matches!(e, EventKind::Explosion { .. }));
    assert!(bombs >= 3, "bomb explosions {bombs}");
    assert!(damage_to(&ev, 1) > 0.0);
}

#[test]
fn radar_shot_fires_at_enemy_in_range() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-10.0, 0.0]);
    give(&mut w, rc, 0, "RadarShot");
    let ammo = w.player(0).gun.ammo;
    let ev = run(
        &mut w,
        c,
        &[aim(-1.0, 0.0, PlayerInput::BTN_BLOCK), IDLE],
        1,
    );
    assert!(ev.iter().any(|e| matches!(
        e,
        EventKind::RadarPing {
            player: 0,
            target: 1,
            ..
        }
    )));
    let ev2 = run(&mut w, c, &[IDLE, IDLE], 120);
    assert_eq!(w.player(0).gun.ammo, ammo, "forced shots use no ammo");
    assert!(damage_to(&[ev, ev2].concat(), 1) > 0.0);
    // Out of range: nothing.
    let mut w = arena(rc, &[-20.0, 20.0]);
    give(&mut w, rc, 0, "RadarShot");
    let ev = block(&mut w, c, 0, 5);
    assert!(!ev.iter().any(|e| matches!(e, EventKind::RadarPing { .. })));
}

#[test]
fn implode_pulls_enemies_in() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-10.0, -4.0]);
    give(&mut w, rc, 0, "Implode");
    let d0 = (w.player(1).pos - w.player(0).pos).len();
    let ev = block(&mut w, c, 0, 30);
    assert!(ev.iter().any(|e| matches!(e, EventKind::Explosion { .. })));
    let d1 = (w.player(1).pos - w.player(0).pos).len();
    assert!(d1 < d0 - 1.0, "distance {d0} -> {d1}");
}

#[test]
fn supernova_pulls_then_stuns_and_blasts() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-10.0, -7.5]);
    give(&mut w, rc, 0, "Supernova");
    let ev = block(&mut w, c, 0, 200);
    let stun = ev.iter().any(|e| {
        matches!(
            e,
            EventKind::Stunned {
                player: 1,
                start: true
            }
        )
    });
    assert!(stun, "stunned by the second explosion");
    assert!(damage_to(&ev, 1) > 20.0, "{}", damage_to(&ev, 1));
}

fn block_spawn_hits(
    rc: &RealContent,
    card: &str,
    dist: f32,
    ticks: usize,
) -> (World, Vec<EventKind>) {
    let c = &rc.content;
    let mut w = arena(rc, &[-10.0, -10.0 + dist]);
    give(&mut w, rc, 0, card);
    let ev = block(&mut w, c, 0, ticks);
    (w, ev)
}

#[test]
fn frost_slam_slows() {
    let rc = need_data!();
    let (_, ev) = block_spawn_hits(rc, "Frost slam", 3.0, 5);
    assert!(
        ev.iter()
            .any(|e| matches!(e, EventKind::Slowed { player: 1, .. }))
    );
}

#[test]
fn silence_silences() {
    let rc = need_data!();
    let (w, ev) = block_spawn_hits(rc, "Silence", 3.0, 5);
    assert!(ev.iter().any(|e| matches!(
        e,
        EventKind::Silenced {
            player: 1,
            start: true
        }
    )));
    assert!(w.player(1).silenced);
}

#[test]
fn shockwave_launches_into_flying_state() {
    let rc = need_data!();
    let (w, ev) = block_spawn_hits(rc, "Shockwave", 3.0, 2);
    let _ = ev;
    assert!(w.player(1).vel.len() > 30.0, "{:?}", w.player(1).vel);
    assert!(w.player(1).flying_for > 0.0);
}

#[test]
fn static_field_pulses() {
    let rc = need_data!();
    let (_, ev) = block_spawn_hits(rc, "Static field", 2.0, 120);
    let n = count(&ev, |e| matches!(e, EventKind::Damaged { player: 1, .. }));
    assert!(n > 5, "pulses {n}");
}

#[test]
fn healing_field_heals_after_fill() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-10.0, 20.0]);
    give(&mut w, rc, 0, "Healing field");
    let pos = w.player(0).pos;
    w.take_damage(c, 0, V2::UP * 60.0, pos, None, true, true);
    let h0 = w.player(0).health;
    let ev = block(&mut w, c, 0, 100);
    assert_eq!(w.player(0).health, h0, "not before the counter fills");
    let _ = ev;
    run(&mut w, c, &[IDLE, IDLE], 100);
    assert!(
        w.player(0).health > h0 + 50.0,
        "{} -> {}",
        h0,
        w.player(0).health
    );
}

#[test]
fn overpower_hits_with_owner_max_hp() {
    let rc = need_data!();
    let c = &rc.content;
    let (w, ev) = block_spawn_hits(rc, "Overpower", 2.5, 60);
    let d = damage_to(&ev, 1);
    let expect = w.player(0).stats.max_health * 0.15;
    assert!(d >= expect * 0.9, "overpower damage {d} vs {expect}");
    let _ = c;
}

#[test]
fn black_hole_pulls() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-10.0, -6.0]);
    give(&mut w, rc, 0, "BlackHole");
    let d0 = (w.player(1).pos - w.player(0).pos).len();
    block(&mut w, c, 0, 40);
    let d1 = (w.player(1).pos - w.player(0).pos).len();
    assert!(d1 < d0 - 0.5, "{d0} -> {d1}");
}

#[test]
fn remnant_explodes_when_enemy_comes_close() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-10.0, 0.0]);
    give(&mut w, rc, 0, "Remnant");
    let ev = block(&mut w, c, 0, 30);
    assert!(
        ev.iter()
            .any(|e| matches!(e, EventKind::RemnantSpawned { player: 0, .. }))
    );
    // Owner walks away, enemy walks onto the remnant.
    let ev = run(&mut w, c, &[input(-1.0, 0.0, 0), input(-1.0, 0.0, 0)], 150);
    assert!(damage_to(&ev, 1) > 10.0, "{}", damage_to(&ev, 1));
}

#[test]
fn tactical_reload_on_block_and_echo_repeats() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-10.0, 25.0]);
    give(&mut w, rc, 0, "Tactical reload");
    shoot(&mut w, c, 0, v2(-1.0, 0.0), 2);
    assert!(w.player(0).gun.ammo < w.player(0).gun.ammo_stats.max_ammo);
    block(&mut w, c, 0, 2);
    assert_eq!(w.player(0).gun.ammo, w.player(0).gun.ammo_stats.max_ammo);
    let mut w = arena(rc, &[-10.0, 25.0]);
    give(&mut w, rc, 0, "Echo");
    let ev = block(&mut w, c, 0, 40);
    let kinds: Vec<BlockKind> = ev
        .iter()
        .filter_map(|e| match e {
            EventKind::BlockStarted { player: 0, kind } => Some(*kind),
            _ => None,
        })
        .collect();
    assert_eq!(kinds, vec![BlockKind::Default, BlockKind::Echo]);
}

/// Player 0 hits player 1 once with a plain bullet (6 units apart).
fn hit_once(w: &mut World, c: &Content) -> Vec<EventKind> {
    let n = w.players().len();
    let mut ev = run(w, c, &only(n, 0, aim(1.0, 0.0, PlayerInput::BTN_FIRE)), 1);
    ev.extend(run(w, c, &only(n, 0, aim(1.0, 0.0, 0)), 30));
    ev
}

#[test]
fn brawler_triples_hp_after_dealing_damage() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-10.0, -4.0]);
    give(&mut w, rc, 0, "Brawler");
    let hp = w.player(0).stats.max_health;
    let ev = hit_once(&mut w, c);
    assert!(damage_to(&ev, 1) > 0.0);
    assert!((w.player(0).stats.max_health - hp * 3.0).abs() < 1.0);
    run(&mut w, c, &[IDLE, IDLE], 300);
    assert!((w.player(0).stats.max_health - hp).abs() < 1.0);
}

#[test]
fn taste_of_blood_speeds_up_after_dealing_damage() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-10.0, -4.0]);
    give(&mut w, rc, 0, "TasteOfBlood");
    let ms = w.player(0).stats.movement_speed;
    hit_once(&mut w, c);
    assert!((w.player(0).stats.movement_speed - ms * 1.5).abs() < 1e-3);
    assert!(w.player(0).stats.life_steal > 0.0);
}

#[test]
fn pristine_perseverance_toggles_at_threshold() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-10.0, 20.0]);
    give(&mut w, rc, 0, "Pristine perseverence");
    run(&mut w, c, &[IDLE, IDLE], 2);
    let hp = w.player(0).stats.max_health;
    assert!(hp > 400.0, "full-HP bonus active: {hp}");
    let pos = w.player(0).pos;
    w.take_damage(c, 0, V2::UP * (hp * 0.2), pos, None, true, true);
    run(&mut w, c, &[IDLE, IDLE], 2);
    assert!(w.player(0).stats.max_health < hp / 4.0);
}

#[test]
fn chase_speeds_up_toward_visible_enemy() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-10.0, 10.0]);
    give(&mut w, rc, 0, "Chase");
    let ms = w.player(0).stats.movement_speed;
    run(&mut w, c, &[input(1.0, 0.0, 0), IDLE], 3);
    assert!((w.player(0).stats.movement_speed - ms * 1.6).abs() < 1e-3);
    run(&mut w, c, &[input(-1.0, 0.0, 0), IDLE], 3);
    assert!((w.player(0).stats.movement_speed - ms).abs() < 1e-3);
}

#[test]
fn chilling_presence_slows_nearby_enemy() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-10.0, -7.0]);
    give(&mut w, rc, 0, "ChillingPresence");
    let ev = run(&mut w, c, &[IDLE, IDLE], 60);
    assert!(ev.iter().any(|e| matches!(
        e,
        EventKind::RangeTriggered {
            player: 0,
            target: 1,
            ..
        }
    )));
    assert!(w.player(1).slow_slow > 0.3, "{}", w.player(1).slow_slow);
}

#[test]
fn lifestealer_drains_nearby_enemy() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-10.0, -7.0]);
    give(&mut w, rc, 0, "Lifestealer");
    let pos = w.player(0).pos;
    w.take_damage(c, 0, V2::UP * 50.0, pos, None, true, true);
    let h0 = w.player(0).health;
    let ev = run(&mut w, c, &[IDLE, IDLE], 100);
    assert!(damage_to(&ev, 1) >= 10.0);
    assert!(
        w.player(0).health > h0 + 10.0,
        "{h0} -> {}",
        w.player(0).health
    );
    assert!(
        ev.iter()
            .any(|e| matches!(e, EventKind::LifeDrain { from: 1, to: 0, .. }))
    );
}

#[test]
fn scavenger_reloads_on_damage() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-10.0, -4.0]);
    give(&mut w, rc, 0, "Scavenger");
    let ev = hit_once(&mut w, c);
    assert!(
        ev.iter()
            .any(|e| matches!(e, EventKind::ReloadDone { player: 0, .. }))
    );
    assert_eq!(w.player(0).gun.ammo, w.player(0).gun.ammo_stats.max_ammo);
}

#[test]
fn shields_up_blocks_when_out_of_ammo() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-10.0, 25.0]);
    give(&mut w, rc, 0, "Shields up");
    assert!(!w.player(0).stats.automatic_reload);
    let mut ev = vec![];
    for _ in 0..w.player(0).gun.ammo_stats.max_ammo {
        ev.extend(shoot(&mut w, c, 0, v2(-1.0, 0.3), 40));
    }
    assert!(ev.iter().any(|e| matches!(
        e,
        EventKind::BlockStarted {
            player: 0,
            kind: BlockKind::Default
        }
    )));
}

#[test]
fn refresh_resets_block_on_damage() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-10.0, -4.0]);
    give(&mut w, rc, 0, "Refresh");
    block(&mut w, c, 0, 40);
    assert!(w.player(0).block.counter < 2.0);
    hit_once(&mut w, c);
    assert!(w.player(0).block.counter > w.player(0).block.stats.cooldown());
}

#[test]
fn demonic_pact_self_damage_and_explosions() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-10.0, -4.0]);
    give(&mut w, rc, 0, "Demonic pact");
    let ev = hit_once(&mut w, c);
    assert!(
        (damage_to(&ev, 0) - 10.0).abs() < 0.5,
        "{}",
        damage_to(&ev, 0)
    );
    assert!(ev.iter().any(|e| matches!(e, EventKind::Explosion { .. })));
    // Non-lethal.
    let pos = w.player(0).pos;
    w.take_damage(
        c,
        0,
        V2::UP * (w.player(0).health - 2.0),
        pos,
        None,
        true,
        true,
    );
    shoot(&mut w, c, 0, v2(-1.0, 0.0), 30);
    assert!(!w.player(0).dead && w.player(0).health >= 1.0);
}

#[test]
fn radiance_waves_during_reload() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-10.0, -5.0]);
    give(&mut w, rc, 0, "Radiance");
    let mut ev = vec![];
    for _ in 0..w.player(0).gun.ammo_stats.max_ammo {
        ev.extend(shoot(&mut w, c, 0, v2(-1.0, 0.5), 25));
    }
    ev.extend(run(&mut w, c, &[IDLE, IDLE], 120));
    let waves = count(
        &ev,
        |e| matches!(e, EventKind::ObjectSpawned { def, .. } if c.object(*def).name == "E_Radiance"),
    );
    assert!(waves >= 2, "waves {waves}");
    assert!(damage_to(&ev, 1) > 0.0);
}

#[test]
fn divine_countdown_doubles_hp() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-10.0, 20.0]);
    give(&mut w, rc, 0, "DivineCountdown");
    let hp = w.player(0).stats.max_health;
    let ev = run(&mut w, c, &[IDLE, IDLE], 720);
    assert!(
        ev.iter()
            .any(|e| matches!(e, EventKind::AbyssalEntered { player: 0 }))
    );
    assert!((w.player(0).stats.max_health - 2.0 * hp).abs() < 1.0);
}

fn first_hit_damage(ev: &[EventKind], p: u8) -> Option<f32> {
    ev.iter().find_map(|e| match e {
        EventKind::ProjectileHit {
            damage,
            target: HitTarget::Player(t),
            ..
        } if *t == p => Some(*damage),
        _ => None,
    })
}

#[test]
fn homing_curves_toward_target() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-10.0, 2.0]);
    give(&mut w, rc, 0, "Homing");
    run(&mut w, c, &[aim(1.0, 0.6, PlayerInput::BTN_FIRE), IDLE], 1);
    // The closest player of any team is the target (cards.md 5.5): the
    // shooter itself until the bullet passes the midpoint, then p1.
    let mut locked = None;
    let mut hit = false;
    for t in 0..120 {
        let ev = run(&mut w, c, &[IDLE, IDLE], 1);
        hit |= first_hit_damage(&ev, 1).is_some();
        if locked.is_none()
            && ev
                .iter()
                .any(|e| matches!(e, EventKind::HomingLocked { target: 1, .. }))
        {
            let p = &w.projectiles()[0];
            locked = Some((t, p.vel.angle_deg(w.player(1).pos - p.pos)));
        }
        if let (Some((t0, a0)), Some(p)) = (locked, w.projectiles().first())
            && t == t0 + 6
        {
            let a1 = p.vel.angle_deg(w.player(1).pos - p.pos);
            assert!(a1 < a0 - 10.0, "turns toward the target: {a0} -> {a1}");
        }
    }
    assert!(locked.is_some(), "locks on p1");
    let _ = hit;
}

#[test]
fn remote_steers_with_aim() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-20.0, 25.0]);
    give(&mut w, rc, 0, "Remote");
    run(&mut w, c, &[aim(1.0, 0.0, PlayerInput::BTN_FIRE), IDLE], 1);
    run(&mut w, c, &[aim(1.0, 0.0, 0), IDLE], 3);
    // Steer straight up.
    run(&mut w, c, &[aim(0.0, 1.0, 0), IDLE], 25);
    let p = &w.projectiles()[0];
    assert!(p.vel.y > p.vel.x.abs(), "vel {:?}", p.vel);
}

#[test]
fn drill_passes_through_a_platform() {
    let rc = need_data!();
    let c = &rc.content;
    // Platforms arena: PlatL at (-12, -4), half height 0.5.
    let mut w = sandbox(c, 1, 2);
    w.teleport(0, v2(-12.0, FLOOR_Y + 1.0));
    w.teleport(1, v2(20.0, FLOOR_Y + 1.0));
    settle(&mut w, c, 30);
    give(&mut w, rc, 0, "DrillAmmo");
    let ev = shoot(&mut w, c, 0, v2(0.0, 1.0), 40);
    assert!(ev.iter().any(|e| matches!(e, EventKind::DrillEnter { .. })));
    assert!(ev.iter().any(|e| matches!(e, EventKind::DrillExit { .. })));
    assert!(
        w.projectiles().iter().any(|p| p.pos.y > -3.0),
        "bullet above the platform"
    );
}

#[test]
fn bash_stuns_on_hit() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-10.0, -4.0]);
    give(&mut w, rc, 0, "Bash_DMG");
    let ev = hit_once(&mut w, c);
    assert!(ev.iter().any(|e| matches!(
        e,
        EventKind::Stunned {
            player: 1,
            start: true
        }
    )));
}

#[test]
fn grow_increases_damage_with_distance() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-25.0, 25.0]);
    give(&mut w, rc, 0, "Grow");
    let base = w.player(0).gun.stats.damage * 55.0;
    shoot(&mut w, c, 0, v2(1.0, 0.6), 30);
    let d = w.projectiles()[0].damage;
    assert!(d > base * 1.5, "grown damage {d} from {base}");
}

#[test]
fn sneaky_bullets_dodge_the_ground() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-20.0, 25.0]);
    give(&mut w, rc, 0, "Sneaky bullets");
    let ev = shoot(&mut w, c, 0, v2(1.0, -0.15), 30);
    // Without dodging, the bullet hits the floor within a few units.
    let floor_hits = count(&ev, |e| {
        matches!(
            e,
            EventKind::ProjectileHit {
                target: HitTarget::Map,
                ..
            }
        )
    });
    assert_eq!(floor_hits, 0);
    assert!(!w.projectiles().is_empty());
}

#[test]
fn target_bounce_retargets_after_bounce() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-10.0, 0.0]);
    give(&mut w, rc, 0, "TargetBounce");
    // Shoot into the floor in front: it bounces and turns toward a player.
    let ev = shoot(&mut w, c, 0, v2(1.0, -0.6), 80);
    assert!(
        ev.iter()
            .any(|e| matches!(e, EventKind::TargetBounce { .. }))
    );
}

#[test]
fn screen_edge_bounce_cards() {
    let rc = need_data!();
    let c = &rc.content;
    for card in ["Bouncy", "Mayhem", "Riccochet", "Trickster"] {
        let mut w = arena(rc, &[-10.0, 25.0]);
        give(&mut w, rc, 0, card);
        // From high up, straight up: hits the top edge and bounces back.
        w.teleport(0, v2(-10.0, 16.0));
        let ev = shoot(&mut w, c, 0, v2(0.05, 1.0), 60);
        assert!(
            ev.iter().any(
                |e| matches!(e, EventKind::ProjectileBounced { normal, .. } if normal.y < -0.5)
            ),
            "{card} bounces off the top edge"
        );
    }
    // Trickster: damage grows per bounce.
    let mut w = arena(rc, &[-10.0, 25.0]);
    give(&mut w, rc, 0, "Trickster");
    let d0 = w.player(0).gun.stats.damage * 55.0;
    w.teleport(0, v2(-10.0, 16.0));
    shoot(&mut w, c, 0, v2(0.05, 1.0), 30);
    let p = w.projectiles().first().map(|p| p.damage);
    assert!(p.is_some_and(|d| d > d0 * 1.2), "{p:?} vs {d0}");
}

#[test]
fn thruster_pushes_stuck_target() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-10.0, -4.0]);
    give(&mut w, rc, 0, "Thruster");
    let ev = hit_once(&mut w, c);
    assert!(ev.iter().any(
        |e| matches!(e, EventKind::ObjectSpawned { def, .. } if c.object(*def).name == "A_Thruster")
    ));
    let x0 = w.player(1).pos.x;
    run(&mut w, c, &[IDLE, IDLE], 60);
    assert!(
        w.player(1).pos.x > x0 + 3.0,
        "{x0} -> {}",
        w.player(1).pos.x
    );
}

#[test]
fn timed_detonation_explodes_later() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-10.0, -4.0]);
    give(&mut w, rc, 0, "Timed detonation");
    let n = w.players().len();
    let mut ev = run(
        &mut w,
        c,
        &only(n, 0, aim(1.0, 0.0, PlayerInput::BTN_FIRE)),
        1,
    );
    let mut hit_at = None;
    for t in 0..80 {
        let e = run(&mut w, c, &[IDLE, IDLE], 1);
        if hit_at.is_none() && first_hit_damage(&e, 1).is_some() {
            hit_at = Some(t);
        }
        if let Some(h) = hit_at
            && e.iter().any(|x| matches!(x, EventKind::Explosion { .. }))
        {
            assert!(t >= h + 20, "explodes ~0.35 s after the hit ({h} -> {t})");
            return;
        }
        ev.extend(e);
    }
    panic!("no delayed explosion");
}

#[test]
fn toxic_cloud_pulses_damage() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-10.0, -4.0]);
    give(&mut w, rc, 0, "Toxic cloud");
    let ev = hit_once(&mut w, c);
    let ev2 = run(&mut w, c, &[IDLE, IDLE], 150);
    let n = count(&[ev, ev2].concat(), |e| {
        matches!(e, EventKind::Damaged { player: 1, .. })
    });
    assert!(n >= 5, "cloud ticks {n}");
}

#[test]
fn dazzle_stuns_repeatedly() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-10.0, -4.0]);
    give(&mut w, rc, 0, "Dazzle");
    let ev = hit_once(&mut w, c);
    let ev2 = run(&mut w, c, &[IDLE, IDLE], 120);
    let n = count(&[ev, ev2].concat(), |e| {
        matches!(
            e,
            EventKind::Stunned {
                player: 1,
                start: true
            }
        )
    });
    assert!(n >= 2, "stuns {n}");
}

#[test]
fn explosive_bullet_explodes_on_hit() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-10.0, -4.0, -2.5]);
    give(&mut w, rc, 0, "Explosive bullet");
    let ev = hit_once(&mut w, c);
    assert!(ev.iter().any(|e| matches!(e, EventKind::Explosion { .. })));
    assert!(damage_to(&ev, 2) > 0.0, "splash on the bystander");
}

#[test]
fn ensnare_pulls_target_back() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-10.0, -4.0]);
    give(&mut w, rc, 0, "Ensnare");
    let ev = hit_once(&mut w, c);
    assert!(
        ev.iter()
            .any(|e| matches!(e, EventKind::Ensnared { target: 1, .. }))
    );
}

#[test]
fn parasite_and_cold_and_big_bullet() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-10.0, -4.0]);
    give(&mut w, rc, 0, "Parasite");
    let ev = hit_once(&mut w, c);
    let ev2 = run(&mut w, c, &[IDLE, IDLE], 400);
    let n = count(&[ev, ev2].concat(), |e| {
        matches!(e, EventKind::Damaged { player: 1, .. })
    });
    assert!(n > 10, "parasite dot ticks {n}");
    let mut w = arena(rc, &[-10.0, -4.0]);
    give(&mut w, rc, 0, "Cold bullets");
    let ev = hit_once(&mut w, c);
    assert!(
        ev.iter()
            .any(|e| matches!(e, EventKind::Slowed { player: 1, .. }))
    );
    let mut w = arena(rc, &[-10.0, -4.0]);
    give(&mut w, rc, 0, "Big Bullet");
    run(&mut w, c, &[aim(1.0, 0.0, PlayerInput::BTN_FIRE), IDLE], 1);
    assert!(w.projectiles()[0].size > 0.5, "{}", w.projectiles()[0].size);
}

/// Cards with no effect object: their stats merge as cards.md 4 says.
#[test]
fn stat_cards_merge() {
    use rounds_sim::stats::{AmmoStats, GunStats, PlayerStats};
    let rc = need_data!();
    let c = &rc.content;
    let stat_cards = [
        "Barrage",
        "Big Bullet",
        "Buckshot",
        "Burst",
        "Careful planning",
        "Cold bullets",
        "Combine",
        "Decay",
        "Defender",
        "Echo",
        "Fast ball",
        "Fast forward",
        "Glasscannon",
        "HUGE",
        "Leach",
        "Phoenix",
        "Quick Reload",
        "Quick Shot",
        "SPRAY",
        "Steady shot",
        "Tank",
        "Wind up",
        "Frenzy",
        "Overcharge",
        "MultiShot",
        "Strength",
        "More ammo",
        "Ammo reg",
        "0. PlainCard",
        "Bouncy",
        "Mayhem",
        "Riccochet",
        "Trickster",
    ];
    for name in stat_cards {
        let id = rc.card(name);
        let cd = c.card(id);
        let mut w = arena(rc, &[-10.0, 10.0]);
        w.debug_give_card(c, 0, id);
        let p = w.player(0);
        let mut g = GunStats::player_reset();
        let mut a = AmmoStats::default();
        let mut s = PlayerStats::default();
        if let Some(cg) = &cd.gun {
            a.merge_card(cg);
            g.merge_card(cg);
        }
        if let Some(ch) = &cd.character {
            s.merge_card(ch);
        }
        assert_eq!(p.gun.stats.damage, g.damage, "{name} damage");
        assert_eq!(
            p.gun.stats.number_of_projectiles, g.number_of_projectiles,
            "{name}"
        );
        assert_eq!(p.gun.stats.attack_speed, g.attack_speed, "{name}");
        assert_eq!(p.gun.stats.reflects, g.reflects, "{name}");
        assert_eq!(p.gun.stats.bursts, g.bursts, "{name}");
        assert_eq!(p.gun.ammo_stats.max_ammo, a.max_ammo, "{name}");
        assert_eq!(
            p.gun.ammo_stats.reload_duration(),
            a.reload_duration(),
            "{name}"
        );
        assert_eq!(p.stats.max_health, s.max_health, "{name}");
        assert_eq!(p.stats.life_steal, s.life_steal, "{name}");
        assert_eq!(p.stats.respawns, s.respawns, "{name}");
        assert_eq!(
            p.stats.seconds_to_take_damage_over, s.seconds_to_take_damage_over,
            "{name}"
        );
        assert!(p.effects.is_empty(), "{name} adds no effect object");
    }
}

#[test]
fn barrage_burst_spray_fire_patterns() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-10.0, 25.0]);
    give(&mut w, rc, 0, "Barrage");
    let n = w.player(0).gun.stats.number_of_projectiles as usize;
    assert!(n > 1);
    run(&mut w, c, &[aim(-1.0, 0.3, PlayerInput::BTN_FIRE), IDLE], 1);
    assert_eq!(w.projectiles().len(), n);
    // Burst: later bursts in later ticks.
    let mut w = arena(rc, &[-10.0, 25.0]);
    give(&mut w, rc, 0, "Burst");
    run(&mut w, c, &[aim(-1.0, 0.3, PlayerInput::BTN_FIRE), IDLE], 1);
    let first = w.projectiles().len();
    run(&mut w, c, &[aim(-1.0, 0.3, 0), IDLE], 30);
    assert!(
        w.projectiles().len() > first,
        "bursts {first} -> {}",
        w.projectiles().len()
    );
    // Spray: holding fire repeats.
    let mut w = arena(rc, &[-10.0, 25.0]);
    give(&mut w, rc, 0, "SPRAY");
    let ev = run(
        &mut w,
        c,
        &[aim(-1.0, 0.3, PlayerInput::BTN_FIRE), IDLE],
        60,
    );
    let shots = count(&ev, |e| matches!(e, EventKind::ShotFired { player: 0, .. }));
    assert!(shots > 3, "auto fire shots {shots}");
}

#[test]
fn decay_and_leech() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-10.0, -4.0]);
    give(&mut w, rc, 1, "Decay");
    let mut ev = hit_once(&mut w, c);
    ev.extend(run(&mut w, c, &[IDLE, IDLE], 100));
    let ticks = count(&ev, |e| matches!(e, EventKind::Damaged { player: 1, .. }));
    assert!(
        ticks >= 2,
        "damage spread over time ({ticks} chunks so far)"
    );
    let mut w = arena(rc, &[-10.0, -4.0]);
    give(&mut w, rc, 0, "Leach");
    let pos = w.player(0).pos;
    w.take_damage(c, 0, V2::UP * 50.0, pos, None, true, true);
    let h0 = w.player(0).health;
    hit_once(&mut w, c);
    assert!(w.player(0).health > h0, "life steal heals");
}

#[test]
fn fast_forward_moves_bullets_faster() {
    let rc = need_data!();
    let c = &rc.content;
    let travel = |card: Option<&str>| {
        let mut w = arena(rc, &[-25.0, 25.0]);
        if let Some(k) = card {
            give(&mut w, rc, 0, k);
        }
        run(&mut w, c, &[aim(1.0, 0.3, PlayerInput::BTN_FIRE), IDLE], 1);
        run(&mut w, c, &[IDLE, IDLE], 10);
        w.projectiles()[0].distance
    };
    assert!(travel(Some("Fast forward")) > travel(None) * 1.3);
}

/// Team play (cards.md 5.5): "other player" effects target the closest
/// living enemy, never a teammate.
#[test]
fn effects_target_enemies_not_teammates() {
    use rounds_sim::{GameMode, MapId, MatchSetup, TeamMode};
    let rc = need_data!();
    let c = &rc.content;
    let mut s = MatchSetup::ffa(3, 7);
    s.mode = GameMode::Sandbox;
    s.teams = TeamMode::Teams;
    s.slots[0].team = 0;
    s.slots[1].team = 0;
    s.slots[2].team = 1;
    s.map_rotation = vec![MapId(0)];
    let mut w = World::new(c, &s);
    settle(&mut w, c, 300);
    let y = FLOOR_Y + 1.0;
    w.teleport(0, v2(-10.0, y));
    w.teleport(1, v2(-8.0, y));
    w.teleport(2, v2(-6.5, y));
    settle(&mut w, c, 20);
    give(&mut w, rc, 0, "ChillingPresence");
    give(&mut w, rc, 0, "Lifestealer");
    let ev = run(&mut w, c, &[IDLE; 3], 60);
    assert!(ev.iter().any(|e| matches!(
        e,
        EventKind::RangeTriggered {
            player: 0,
            target: 2,
            ..
        }
    )));
    assert!(
        !ev.iter()
            .any(|e| matches!(e, EventKind::RangeTriggered { target: 1, .. }))
    );
    assert_eq!(damage_to(&ev, 1), 0.0, "teammate untouched");
    assert!(damage_to(&ev, 2) > 0.0);
}

#[test]
fn picking_an_effect_card_twice_levels_it_up() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-10.0, 20.0]);
    give(&mut w, rc, 0, "Teleport");
    give(&mut w, rc, 0, "Teleport");
    assert_eq!(
        w.player(0).effects.len(),
        1,
        "AttackLevel merges into one object"
    );
    run(&mut w, c, &[aim(1.0, 0.0, 0), IDLE], 3);
    let x0 = w.player(0).pos.x;
    run(&mut w, c, &[aim(1.0, 0.0, PlayerInput::BTN_BLOCK), IDLE], 1);
    // Level 2: twice the distance.
    let dx = w.player(0).pos.x - x0;
    assert!((dx - 12.0).abs() < 0.5, "moved {dx}");
    // Objects without AttackLevel run in parallel.
    give(&mut w, rc, 0, "Pristine perseverence");
    give(&mut w, rc, 0, "Pristine perseverence");
    assert_eq!(w.player(0).effects.len(), 3);
}

#[test]
fn ensnare_holds_the_target() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-10.0, -4.0]);
    give(&mut w, rc, 0, "Ensnare");
    let ev = hit_once(&mut w, c);
    let start = ev
        .iter()
        .find_map(|e| match e {
            EventKind::Ensnared {
                target: 1, start, ..
            } => Some(*start),
            _ => None,
        })
        .expect("ensnared");
    // The target tries to run away but is held near the start point.
    run(&mut w, c, &[IDLE, input(1.0, 0.0, 0)], 40);
    let d = (w.player(1).pos - start).len();
    assert!(d < 4.0, "held within reach: {d}");
}

// ---- Regression tests for docs/spec/review/sim-vs-spec.md ----

/// M3: EMP bullets record an immunity entry but always deal their damage.
#[test]
fn review_m3_emp_bullets_all_deal_damage() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-10.0, -8.0]);
    give(&mut w, rc, 0, "EMP");
    let mut ev = run(&mut w, c, &[press(PlayerInput::BTN_BLOCK), IDLE], 1);
    ev.extend(run(&mut w, c, &[IDLE, IDLE], 60));
    let hits = count(&ev, |e| matches!(e, EventKind::Damaged { player: 1, .. }));
    assert!(hits >= 3, "damaging EMP hits {hits}");
}

/// M4: the teleport's free-space test sees players (no layer mask): a player
/// standing at the far end blocks the last samples.
#[test]
fn review_m4_teleport_does_not_end_inside_a_player() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-10.0, -4.0]);
    give(&mut w, rc, 0, "Teleport");
    run(&mut w, c, &[aim(1.0, 0.0, 0), IDLE], 3);
    let x0 = w.player(0).pos.x;
    run(&mut w, c, &[aim(1.0, 0.0, PlayerInput::BTN_BLOCK), IDLE], 1);
    let x1 = w.player(0).pos.x;
    let x_other = w.player(1).pos.x;
    let r = w.player(0).radius().max(w.player(1).radius());
    assert!(x1 - x0 > 3.0, "teleported {}", x1 - x0);
    assert!(
        x_other - x1 > 0.5 + r - 0.05,
        "landed at {x1}, other player at {x_other}"
    );
}

/// L1: Radar Shot's first forced shot waits 0.1 real s too.
#[test]
fn review_l1_radar_first_shot_is_delayed() {
    let rc = need_data!();
    let c = &rc.content;
    let mut w = arena(rc, &[-10.0, 0.0]);
    give(&mut w, rc, 0, "RadarShot");
    let mut first = None;
    for k in 0..40u32 {
        let inp = if k == 0 {
            aim(-1.0, 0.0, PlayerInput::BTN_BLOCK)
        } else {
            IDLE
        };
        let ev = run(&mut w, c, &[inp, IDLE], 1);
        if k == 0 {
            assert!(ev.iter().any(|e| matches!(e, EventKind::RadarPing { .. })));
        }
        if first.is_none()
            && ev
                .iter()
                .any(|e| matches!(e, EventKind::ShotFired { player: 0, .. }))
        {
            first = Some(k);
        }
    }
    // ticks(0.1) = 8 real ticks after the block tick.
    assert_eq!(first, Some(8));
}
