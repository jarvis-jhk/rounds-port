//! Player mechanics (player.md 6.3, 8, 11, 12, 14, 15) and card stat merging.
mod common;
use common::*;
use rounds_sim::{EventKind, PlayerInput, v2};

#[test]
fn wall_grab_slow_slide_and_wall_jump() {
    let c = content();
    let mut w = sandbox(&c, 0, 1);
    // Inner face of the right wall is x = 30.
    w.teleport(0, v2(28.8, 0.0));
    for _ in 0..40 {
        w.step(&c, &[input(1.0, 0.0, 0)]);
    }
    let p = w.player(0);
    assert!(p.wall_grab, "sticking to the wall");
    assert!(
        p.vel.y > -3.0 && p.vel.y < 0.0,
        "slow slide v.y {}",
        p.vel.y
    );
    w.step(&c, &[input(1.0, 0.0, PlayerInput::BTN_JUMP)]);
    let p = w.player(0);
    assert!(
        p.vel.y > 30.0 && p.vel.x < -10.0,
        "wall jump pushes up and away: {:?}",
        p.vel
    );
}

#[test]
fn out_of_bounds_damage_and_push() {
    let c = content();
    let mut w = sandbox(&c, 0, 1);
    w.teleport(0, v2(36.0, -5.0));
    let mut oob = false;
    for _ in 0..12 {
        w.step(&c, &[PlayerInput::default()]);
        oob |= w.events().iter().any(|e| {
            matches!(
                e.kind,
                EventKind::OutOfBounds {
                    shielded: false,
                    ..
                }
            )
        });
    }
    assert!(oob);
    let p = w.player(0);
    assert!((p.health - 49.0).abs() < 1e-3, "51 damage: {}", p.health);
    assert!(p.pos.x < 36.0, "pushed back inward: {:?}", p.pos);
}

#[test]
fn phoenix_respawn() {
    let c = content();
    let mut w = sandbox(&c, 0, 1);
    w.debug_give_card(&c, 0, c.card_by_name("Phoenix").unwrap());
    assert_eq!(w.player(0).stats.max_health, 50.0);
    let pos = w.player(0).pos;
    w.take_damage(&c, 0, v2(0.0, 500.0), pos, None, true, true);
    assert!(w.player(0).respawning && !w.player(0).dead);
    let mut revived_at = None;
    for t in 0..400 {
        w.step(&c, &[PlayerInput::default()]);
        if revived_at.is_none() && !w.player(0).respawning {
            revived_at = Some(t);
        }
    }
    let t = revived_at.expect("revived");
    assert!((180..=190).contains(&t), "2.6 s real = 186 ticks, got {t}");
    let p = w.player(0);
    assert_eq!(p.health, 50.0);
    assert_eq!(p.remaining_respawns, 0);
    // The free block after the respawn starts the cooldown.
    assert!(p.block.counter < 4.0);
}

#[test]
fn stun_freezes_and_slow_brakes() {
    let c = content();
    let mut w = sandbox(&c, 0, 1);
    w.add_stun(0, 0.5);
    let x0 = w.player(0).pos.x;
    for _ in 0..30 {
        w.step(&c, &[input(1.0, 0.0, 0)]);
    }
    assert!(w.player(0).stunned);
    assert!(
        (w.player(0).pos.x - x0).abs() < 1e-4,
        "no movement while stunned"
    );
    for _ in 0..40 {
        w.step(&c, &[input(1.0, 0.0, 0)]);
    }
    assert!(!w.player(0).stunned, "stun of 0.5 game s ends");
    // Slow 0.5 halves thrust: lower top speed.
    w.add_slow(0, 0.5, false);
    assert_eq!(
        w.player(0).get_slow(),
        0.0,
        "slow becomes effective next frame"
    );
    w.step(&c, &[input(1.0, 0.0, 0)]);
    assert!(w.player(0).get_slow() > 0.45);
}

#[test]
fn card_stats_merge_size_mass_and_gun() {
    let c = content();
    let mut w = sandbox(&c, 0, 1);
    let huge = c.card_by_name("Huge").unwrap();
    let big = c.card_by_name("Big Damage").unwrap();
    w.debug_give_card(&c, 0, huge);
    w.debug_give_card(&c, 0, big);
    let p = w.player(0);
    assert_eq!(p.stats.max_health, 200.0);
    assert_eq!(p.health, 200.0, "a pick revives to full");
    let (s, m) = (1.2 * (2.4f32).powf(0.2), 100.0 * (2.4f32).powf(0.8));
    assert!(
        (p.scale - s).abs() < 1e-4 && (p.mass - m).abs() < 1e-2,
        "scale {} mass {}",
        p.scale,
        p.mass
    );
    assert_eq!(p.gun.stats.damage, 2.0);
    assert!((p.gun.ammo_stats.reload_duration() - 2.25).abs() < 1e-5);
    assert_eq!(p.gun.stats.drag_min_speed, 2.0, "+1 per card with a gun");
    assert_eq!(p.cards.len(), 2);
}

#[test]
fn block_cooldown_and_echo() {
    let c = content();
    let mut w = sandbox(&c, 0, 1);
    w.debug_give_card(&c, 0, c.card_by_name("Echo").unwrap());
    let blk = input(0.0, 0.0, PlayerInput::BTN_BLOCK);
    let mut starts = 0;
    for k in 0..60 {
        let i = if k % 2 == 0 {
            blk
        } else {
            PlayerInput::default()
        };
        w.step(&c, &[i]);
        starts += w
            .events()
            .iter()
            .filter(|e| matches!(e.kind, EventKind::BlockStarted { player: 0, .. }))
            .count();
    }
    // One real block (cooldown 4 s) + one echo 0.2 s later.
    assert_eq!(starts, 2);
    // Recharge after 4 game s = ~336 ticks.
    let mut recharged = false;
    for _ in 0..340 {
        w.step(&c, &[PlayerInput::default()]);
        recharged |= w
            .events()
            .iter()
            .any(|e| matches!(e.kind, EventKind::BlockRecharged { player: 0 }));
    }
    assert!(recharged);
}
