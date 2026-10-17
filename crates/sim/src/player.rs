//! The player character (docs/spec/player.md): movement physics, ground and
//! wall interaction, collision/push-out, knockback, out-of-bounds, health,
//! damage, death/phoenix/revive, block and status effects.
//!
//! Tick structure (player.md 0.3): `player_frame` (the original's `Update`,
//! uses `dtU`) runs for every player, then projectiles/objects, then
//! `player_fixed` (the original's `FixedUpdate`, `dtF`) for every player.

use crate::content::{CardId, Content};
use crate::effects::{BlockPhase, Hook};
use crate::events::{BlockKind, EventKind, cue};
use crate::geom::{RayHit, layers};
use crate::gun::Gun;
use crate::input::{Controls, derive_controls};
use crate::math::{V2, clamp, eval_curve, key, pow, v2};
use crate::scene::{Circle, Scene, Target};
use crate::stats::{BlockStats, PlayerStats};
use crate::world::{DT, Hasher, World, ticks};

/// Hit-circle radius at scale 1 (player.md 1.1).
pub const RADIUS: f32 = 0.53;
pub const PARK: V2 = v2(0.0, 200.0);

/// `ForceMode2D` as used by `TakeForce` (player.md 0.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForceMode {
    Impulse,
    Force,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BlockState {
    pub stats: BlockStats,
    pub since_block: f32,
    /// Time since the cooldown started.
    pub counter: f32,
    /// Recharge latch: false after a block until the cooldown elapsed.
    pub active: bool,
}

/// A pending damage-over-time instance (player.md 11.4, weapons.md 14.3).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dot {
    pub total: f32,
    pub dealt: f32,
    pub chunk: f32,
    pub dir: V2,
    pub attacker: Option<u8>,
    /// Real ticks until the next chunk: `WaitForSeconds(interval / ts)` is
    /// evaluated once per chunk, when the chunk is dealt (player.md 11.4).
    pub wait_ticks: u32,
    pub interval: f32,
    pub lethal: bool,
    /// Tick sound (passive damage or poison).
    pub cue: &'static str,
}

/// A "force toward point over time" request of an Implosion (weapons.md 19.4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pull {
    pub point: V2,
    pub force: f32,
    pub clamp: f32,
    pub drag: f32,
    /// Remaining game seconds.
    pub left: f32,
}

/// A pending block repetition (echo / delayed actions), in real ticks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PendingBlock {
    pub ticks: u32,
    pub first: bool,
    pub dont_set_cd: bool,
    pub kind: BlockKind,
    pub only_effects: bool,
    /// true: the second half of a block delayed by `delayOtherActions`.
    pub continuation: bool,
    /// Override position (Empower) carried into a delayed continuation.
    pub at: Option<V2>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounce {
    pub target_vel: V2,
    pub ticks: u32,
}

/// Flight to the spawn point (player.md 10, match-flow.md 7.2).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpawnMove {
    pub from: V2,
    pub to: V2,
    pub c: f32,
    pub hold: u8,
}

/// Minimal leg-step state driving the walk bob (player.md 5.6, 18.7).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Leg {
    pub stepping: bool,
    pub foot_down_time: f32,
    pub step_time: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Immunity {
    pub key: u32,
    pub damage: f32,
    pub time: f32,
}

#[derive(Clone, Debug)]
pub struct Player {
    pub id: u8,
    pub team: u8,
    pub skin: u8,
    pub pos: V2,
    pub vel: V2,
    pub last_pos: V2,
    pub scale: f32,
    pub mass: f32,
    pub stats: PlayerStats,
    pub health: f32,
    pub dead: bool,
    pub playing: bool,
    /// Forfeited / disconnected: out of the match for good (never revived,
    /// spawned, offered cards, or counted for its team). docs/spec/match-flow.md 13.
    pub disconnected: bool,
    pub simulated: bool,
    pub kinematic: bool,
    pub respawning: bool,
    pub remaining_respawns: i32,
    pub phoenix_ticks: u32,
    pub flying_for: f32,
    pub bounce: Option<Bounce>,
    pub since_grounded: f32,
    pub since_jump: f32,
    pub since_wall_grab: f32,
    pub grounded: bool,
    pub wall_grab: bool,
    ground_marker: bool,
    wall_marker: bool,
    pub current_jumps: i32,
    pub wall_normal: V2,
    pub wall_pos: V2,
    pub wall_distance: f32,
    pub ground_pos: V2,
    /// Prop index stood on (non-NetworkPhysicsObject bodies only, 7.6).
    pub stand_on: Option<u32>,
    stand_last: Option<(u32, V2)>,
    pub stun_time: f32,
    pub stunned: bool,
    pub silence_time: f32,
    pub silenced: bool,
    pub slow_slow: f32,
    pub fast_slow: f32,
    pub slow: f32,
    pub block: BlockState,
    pub pending_blocks: Vec<PendingBlock>,
    pub dots: Vec<Dot>,
    pub immunity: Vec<Immunity>,
    pub ignore_wall_frames: i32,
    pub since_dealt_damage: f32,
    pub last_source_of_damage: Option<u8>,
    pub last_damaged_player: Option<u8>,
    /// The Aim script's retained aim (player.md 4.1).
    pub aim: V2,
    pub controls: Controls,
    pub gun: Gun,
    /// Held-gun body (player.md 4.3; visual, but part of the state).
    pub gun_pos: V2,
    pub gun_vel: V2,
    pub spawn_move: Option<SpawnMove>,
    pub cards: Vec<CardId>,
    /// Ids of the card effect objects on this player, in pick order.
    pub effects: Vec<u32>,
    /// Active Implosion pulls.
    pub pulls: Vec<Pull>,
    pub legs: [Leg; 2],
    oob_acc: f32,
    status_cue_frame: u32,
    heal_sound_frame: u32,
    lifesteal_sound_frame: u32,
}

impl Player {
    pub fn new(id: u8, team: u8, skin: u8) -> Player {
        let stats = PlayerStats::default();
        Player {
            id,
            team,
            skin,
            pos: PARK,
            vel: V2::ZERO,
            last_pos: PARK,
            scale: 1.2,
            mass: 100.0,
            stats,
            health: 100.0,
            dead: false,
            playing: false,
            disconnected: false,
            simulated: false,
            kinematic: true,
            respawning: false,
            remaining_respawns: 0,
            phoenix_ticks: 0,
            flying_for: 0.0,
            bounce: None,
            since_grounded: 0.0,
            since_jump: 1.0,
            since_wall_grab: f32::INFINITY,
            grounded: true,
            wall_grab: false,
            ground_marker: false,
            wall_marker: false,
            current_jumps: 1,
            wall_normal: V2::ZERO,
            wall_pos: V2::ZERO,
            wall_distance: 1.0,
            ground_pos: V2::ZERO,
            stand_on: None,
            stand_last: None,
            stun_time: 0.0,
            stunned: false,
            silence_time: 0.0,
            silenced: false,
            slow_slow: 0.0,
            fast_slow: 0.0,
            slow: 0.0,
            block: BlockState {
                stats: BlockStats::default(),
                since_block: 100.0,
                counter: 1000.0,
                active: true,
            },
            pending_blocks: Vec::new(),
            dots: Vec::new(),
            immunity: Vec::new(),
            ignore_wall_frames: 0,
            since_dealt_damage: 100.0,
            last_source_of_damage: None,
            last_damaged_player: None,
            aim: V2::RIGHT,
            controls: Controls::default(),
            gun: Gun::new(),
            gun_pos: PARK,
            gun_vel: V2::ZERO,
            spawn_move: None,
            cards: Vec::new(),
            effects: Vec::new(),
            pulls: Vec::new(),
            legs: [Leg::default(); 2],
            oob_acc: 0.0,
            status_cue_frame: 0,
            heal_sound_frame: 0,
            lifesteal_sound_frame: 0,
        }
    }

    /// `FullReset`: gun, character and block stats back to base, cards
    /// cleared (player.md 10, 11.2, 12.5; cards.md 4.3).
    pub fn full_reset(&mut self, content: &Content) {
        let keep = (
            self.stats.gravity_force,
            self.stats.regeneration,
            self.stats.jumps,
        );
        self.stats = PlayerStats::default();
        if !content.tuning.reset_quirk_stats {
            self.stats.gravity_force = keep.0;
            self.stats.regeneration = keep.1;
            self.stats.jumps = keep.2;
        }
        self.health = 100.0;
        self.slow = 0.0;
        self.slow_slow = 0.0;
        self.fast_slow = 0.0;
        self.block = BlockState {
            stats: BlockStats::default(),
            since_block: 10.0,
            counter: 1000.0,
            active: true,
        };
        self.pending_blocks.clear();
        self.gun = Gun::new();
        self.cards.clear();
        self.configure_mass_and_size();
    }

    /// `ConfigureMassAndSize` (player.md 11.1).
    pub fn configure_mass_and_size(&mut self) {
        let (s, m) = self.stats.scale_and_mass();
        self.scale = s;
        self.mass = m;
    }

    pub fn radius(&self) -> f32 {
        RADIUS * self.scale
    }
    pub fn is_blocking(&self) -> bool {
        self.block.since_block < 0.3
    }
    /// Part of the round: not dead (phoenix limbo still counts as alive).
    pub fn alive(&self) -> bool {
        !self.dead
    }
    /// Has an active collider / can be hit and targeted.
    pub fn hittable(&self) -> bool {
        !self.dead && !self.respawning && self.pos.y < 150.0
    }
    /// `GetSlow()`.
    pub fn get_slow(&self) -> f32 {
        clamp(self.slow, 0.0, 0.9)
    }
    pub fn hand_point(&self) -> V2 {
        self.pos + self.aim.normalized() * self.scale
    }

    fn touch_ground(&mut self, point: V2, normal: V2, prop: Option<(u32, bool)>) -> (bool, f32) {
        if self.since_jump > 0.2 {
            self.current_jumps = self.stats.jumps;
        }
        let old_sg = self.since_grounded;
        match prop {
            None => self.stand_on = None,
            Some((idx, pushable)) => {
                if !pushable {
                    self.stand_on = Some(idx);
                }
            }
        }
        let landed = self.vel.y < -20.0 && !self.grounded;
        self.ground_pos = point;
        self.ground_marker = true;
        let was = self.grounded;
        self.grounded = true;
        self.since_grounded = 0.0;
        let _ = normal;
        (landed || (!was && old_sg > 0.05), old_sg)
    }

    fn touch_wall(&mut self, normal: V2, point: V2) -> Option<f32> {
        if self.grounded {
            return None;
        }
        self.wall_normal = normal;
        self.wall_pos = point;
        self.ground_pos = point;
        self.wall_distance = (self.pos - point).len();
        if self.since_jump < 0.15 {
            return None;
        }
        self.current_jumps = self.stats.jumps;
        let old = self.since_wall_grab;
        self.since_wall_grab = 0.0;
        self.wall_marker = true;
        let was = self.wall_grab;
        self.wall_grab = true;
        if was { None } else { Some(old) }
    }

    pub(crate) fn hash_into(&self, h: &mut Hasher) {
        h.v2(self.pos);
        h.v2(self.vel);
        h.f32(self.health);
        h.f32(self.stats.max_health);
        h.f32(self.scale);
        h.bool(self.dead);
        h.bool(self.playing);
        h.bool(self.disconnected);
        h.bool(self.simulated);
        h.bool(self.respawning);
        h.f32(self.since_grounded);
        h.f32(self.since_jump);
        h.f32(self.since_wall_grab);
        h.bool(self.grounded);
        h.u32(self.current_jumps as u32);
        h.f32(self.stun_time);
        h.f32(self.silence_time);
        h.f32(self.slow_slow);
        h.f32(self.fast_slow);
        h.f32(self.block.since_block);
        h.f32(self.block.counter);
        h.f32(self.flying_for);
        h.v2(self.aim);
        h.v2(self.gun_pos);
        h.u32(self.dots.len() as u32);
        h.u32(self.cards.len() as u32);
        h.u32(self.effects.len() as u32);
        h.u32(self.pulls.len() as u32);
        h.f32(self.stats.movement_speed);
        h.f32(self.stats.size_multiplier);
        self.gun.hash_into(h);
    }
}

/// Player movement curve of the spawn flight (match-flow.md 7.2).
const SPAWN_CURVE: [crate::math::Key; 4] = [
    key(0.0, 0.0, 0.0, 0.0),
    key(0.39, 0.4839, 5.912, 5.912),
    key(0.8, 1.0, 0.0, 0.0),
    key(0.9, 1.0, 0.0, 0.0),
];
/// Leg walk-bob force curve (player.md 5.6).
const BOB_CURVE: [crate::math::Key; 3] = [
    key(0.0, -99.65, -1.5805, -1.5805),
    key(0.5, 70.54, 0.0, 0.0),
    key(1.0, -99.65, 1.0155, 1.0155),
];

/// Support spring curve `c(a)` (player.md 5.6).
fn support_curve(a: f32) -> f32 {
    let (a0, c0, a1) = (-0.1837f32, 0.1584f32, 0.402f32);
    if a <= a0 {
        c0
    } else if a >= a1 {
        0.0
    } else {
        c0 * (a1 - a) / (a1 - a0)
    }
}

impl World {
    /// Player circles for scene queries (index = player id).
    pub(crate) fn player_circles(&self) -> Vec<Option<Circle>> {
        self.players
            .iter()
            .map(|p| {
                if p.hittable() {
                    Some(Circle {
                        c: p.pos,
                        r: p.radius(),
                    })
                } else {
                    None
                }
            })
            .collect()
    }

    pub(crate) fn scene<'a>(
        &'a self,
        content: &'a Content,
        circles: &'a [Option<Circle>],
    ) -> Scene<'a> {
        Scene {
            statics: if self.map.solid {
                self.map.current.map(|m| content.prepared(m))
            } else {
                None
            },
            animated: &self.map.anim_shapes,
            props: &self.physics.props,
            players: circles,
        }
    }

    /// Frame phase of one player (player.md 0.3 step 2).
    pub(crate) fn player_frame(&mut self, content: &Content, i: usize, ts: f32) {
        let dtu = DT * ts;
        // Spawn move (real time, runs while the player is parked/flying).
        if let Some(mut sm) = self.players[i].spawn_move {
            let p = &mut self.players[i];
            if sm.c < 0.9 {
                sm.c = (sm.c + 0.014).min(0.9);
                p.pos = sm.from + (sm.to - sm.from) * eval_curve(&SPAWN_CURVE, sm.c);
                p.last_pos = p.pos;
                p.ignore_wall_frames = 2;
                p.spawn_move = Some(sm);
            } else if sm.hold > 0 {
                sm.hold -= 1;
                p.pos = sm.to;
                p.last_pos = p.pos;
                p.spawn_move = Some(sm);
            } else {
                p.spawn_move = None;
                p.pos = sm.to;
                p.last_pos = sm.to;
                p.simulated = true;
                p.kinematic = false;
                p.playing = true;
                self.revive(content, i, true);
                let p = &self.players[i];
                self.events.push(EventKind::PlayerSpawned {
                    player: i as u8,
                    pos: p.pos,
                });
                self.events.sound(cue::SPAWN[i % 4], p.pos, 1.0);
            }
        }
        // Phoenix limbo timer (real time).
        if self.players[i].respawning {
            let p = &mut self.players[i];
            if p.phoenix_ticks > 0 {
                p.phoenix_ticks -= 1;
            }
            if p.phoenix_ticks == 0 {
                self.revive(content, i, false);
                self.do_block(content, i, true, false, BlockKind::Default, None, false);
            }
            return;
        }
        if self.players[i].dead {
            return;
        }
        let battle = self.flow.battle_ongoing();
        // 1. Input.
        {
            let p = &mut self.players[i];
            let locked = p.stunned || !p.playing;
            p.controls = derive_controls(
                &self.inputs[i],
                &self.prev_inputs[i],
                locked,
                p.silenced,
                p.gun.stats.projectile_speed,
            );
        }
        // CharacterData frame part.
        {
            let p = &mut self.players[i];
            p.since_jump += dtu;
            if !p.simulated {
                p.since_grounded = 0.0;
            }
            if !p.wall_grab {
                p.since_wall_grab += dtu;
            }
            if !p.wall_marker {
                p.wall_grab = false;
            }
            p.wall_marker = false;
        }
        // Stun / silence timers (15.1, 15.2).
        {
            let p = &mut self.players[i];
            if p.stun_time > 0.0 {
                p.stun_time -= dtu;
                p.since_grounded = 0.0;
                if p.stun_time <= 0.0 {
                    p.stun_time = 0.0;
                    p.stunned = false;
                    p.kinematic = false;
                    self.events.push(EventKind::Stunned {
                        player: i as u8,
                        start: false,
                    });
                }
            }
            let p = &mut self.players[i];
            if p.silence_time > 0.0 {
                p.silence_time -= dtu;
                if p.silence_time <= 0.0 {
                    p.silence_time = 0.0;
                    p.silenced = false;
                    self.events.push(EventKind::Silenced {
                        player: i as u8,
                        start: false,
                    });
                }
            }
        }
        // Stat modifiers (14.2, 14.3).
        {
            let p = &mut self.players[i];
            p.gun.attack_speed_multiplier = p.stats.attack_speed_multiplier();
            p.since_dealt_damage = if p.playing {
                p.since_dealt_damage + dtu
            } else {
                100.0
            };
            p.slow = if p.fast_slow > p.slow_slow {
                p.fast_slow
            } else {
                p.slow_slow
            };
            p.slow_slow = clamp(p.slow_slow - dtu * 0.3, 0.0, 10.0);
            p.fast_slow = clamp(p.fast_slow - dtu * 2.0, 0.0, 1.0);
            // Immunity timers (11.9).
            p.immunity.retain_mut(|e| {
                e.time -= dtu;
                e.time > 0.0
            });
        }
        // Health handler: flying, regeneration (11.8).
        {
            let p = &mut self.players[i];
            p.flying_for -= dtu;
            let regen = p.stats.regeneration * dtu;
            if regen != 0.0 && p.playing {
                self.heal(i, regen);
            }
        }
        // Damage over time chunks.
        self.tick_dots(content, i, true);
        if self.players[i].dead || self.players[i].respawning {
            return;
        }
        // Bounce restore (real time).
        if let Some(mut b) = self.players[i].bounce {
            b.ticks = b.ticks.saturating_sub(1);
            if b.ticks == 0 {
                self.players[i].vel = b.target_vel;
                self.players[i].bounce = None;
            } else {
                self.players[i].bounce = Some(b);
            }
        }
        // Block (12.3).
        if self.players[i].simulated {
            let p = &mut self.players[i];
            p.block.since_block += dtu;
            p.block.counter += dtu;
            let cd = p.block.stats.cooldown();
            if p.block.counter > cd && !p.block.active {
                p.block.active = true;
                let pos = p.pos;
                self.events
                    .push(EventKind::BlockRecharged { player: i as u8 });
                self.events.sound(cue::BLOCK_RECHARGED, pos, 1.0);
                self.fire_hook(content, i, Hook::BlockRecharged);
            }
            let p = &self.players[i];
            if p.controls.block_pressed && p.block.counter >= p.block.stats.cooldown() {
                self.do_block(content, i, true, false, BlockKind::Default, None, false);
                self.players[i].block.counter = 0.0;
            }
        }
        self.tick_pending_blocks(content, i);
        // Jump (6.1).
        {
            let p = &self.players[i];
            if p.controls.jump_pressed {
                self.jump(content, i, false, 1.0);
            }
            let p = &mut self.players[i];
            if p.controls.jump_held && p.since_jump < 0.2 {
                p.vel.y += dtu * 2.0 * p.stats.jump * (1.0 - p.get_slow()) * 5500.0 * 0.02;
            }
        }
        // Aim (4.1).
        {
            let p = &mut self.players[i];
            let c = p.controls;
            if c.aim.len() > 0.2 {
                p.aim = c.aim;
            } else if c.direction.len() > 0.2 && c.aim.is_zero() {
                p.aim = c.direction;
            }
        }
        // Weapon handler (13) and ammo (weapons.md 4).
        self.gun_frame(content, i, dtu, battle);
        // Out of bounds (8).
        self.out_of_bounds(content, i, dtu);
    }

    /// Fixed phase of one player (player.md 0.3 step 3).
    pub(crate) fn player_fixed(&mut self, content: &Content, i: usize, ts: f32) {
        let dtf = DT * ts;
        if self.players[i].dead || self.players[i].respawning {
            return;
        }
        // Holding: gun spring (4.3), engine step 0.014 unscaled.
        {
            let p = &mut self.players[i];
            let target = p.hand_point() + p.vel * 0.04;
            let acc = (target - p.gun_pos) * 500.0 - p.gun_vel * 25.0;
            p.gun_vel += acc * DT;
            p.gun_pos += p.gun_vel * DT;
        }
        if !self.players[i].playing {
            // IkLeg keeps stepping (player.md 18.7).
            self.leg_steps(i, dtf);
            // LegRaycasters runs during the spawn flight too: hovering on the
            // spawn point refills the jumps and sets the ground flag (no
            // forces: the player is kinematic; player.md 5.6, Clarified).
            if self.players[i].spawn_move.is_some() {
                let circles = self.player_circles();
                self.leg_ground_contacts(content, i, &circles);
                let p = &mut self.players[i];
                if !p.ground_marker {
                    p.grounded = false;
                }
                p.ground_marker = false;
            }
            // Not standing on anything while parked/flying: a stale
            // ground-follow anchor would jump the player on its first tick.
            self.players[i].stand_last = None;
            return;
        }
        let circles = self.player_circles();
        // Gravity (5.3) and movement (5.1, 5.2).
        {
            let p = &mut self.players[i];
            let s = p.since_grounded.min(p.since_wall_grab);
            let g = 0.02 * ts * p.stats.gravity_force;
            if s > 0.0 {
                p.vel.y -= g * pow(s, 1.5);
            } else {
                p.vel.y -= g * s;
            }
            let mult = if p.grounded { 1.0 } else { 0.8 };
            if !p.stunned {
                let mut d = p.controls.direction;
                d.y = clamp(d.y, -1.0, 0.0) * 2.0;
                p.vel += d * (ts * (1.0 - p.get_slow()) * p.stats.movement_speed * 3.4 * mult);
            }
            p.vel -= p.vel * (ts * 0.2 * mult);
        }
        // Leg rays (5.6).
        self.legs(content, i, &circles, dtf);
        // Integration (5.4).
        {
            let p = &mut self.players[i];
            if p.kinematic {
                p.vel = V2::ZERO;
            } else if p.simulated {
                p.vel.y -= dtf * 20.0;
                p.pos += p.vel * dtf;
            }
        }
        // Collision (7.3).
        self.player_collision(content, i, &circles, ts);
        // CharacterData ground bookkeeping.
        {
            let p = &mut self.players[i];
            if !p.grounded {
                let k = if p.wall_grab && p.wall_distance < 0.7 {
                    0.5
                } else {
                    1.0
                };
                p.since_grounded += dtf * k;
                if p.since_grounded < 0.0 {
                    p.since_grounded = crate::math::lerp(p.since_grounded, 0.0, dtf * 15.0);
                }
            }
            if !p.ground_marker {
                p.grounded = false;
            }
            p.ground_marker = false;
        }
        // Follow ground (7.6).
        {
            let p = &self.players[i];
            let mut np = None;
            if let (Some(idx), true) = (p.stand_on, p.grounded) {
                let body = self
                    .physics
                    .props
                    .get(idx as usize)
                    .filter(|b| b.alive)
                    .map(|b| b.pos);
                if let Some(bp) = body {
                    let delta = match p.stand_last {
                        Some((li, lp)) if li == idx => bp - lp,
                        _ => V2::ZERO,
                    };
                    np = Some((idx, bp, delta));
                }
            }
            let p = &mut self.players[i];
            match np {
                Some((idx, bp, delta)) => {
                    p.pos += delta;
                    p.stand_last = Some((idx, bp));
                }
                None => p.stand_last = None,
            }
        }
    }

    /// Leg step bookkeeping (IkLeg, 18.7), drives the bob term. Runs every
    /// fixed step, grounded or not; only the footstep sound needs ground.
    fn leg_steps(&mut self, i: usize, dtf: f32) {
        let (pos, scale) = (self.players[i].pos, self.players[i].scale);
        {
            let p = &mut self.players[i];
            let speed = 10.0 / scale;
            let grounded = p.grounded;
            let playing = p.playing;
            let vx = p.vel.x.abs();
            for l in 0..2 {
                let other_planted = !p.legs[1 - l].stepping;
                let leg = &mut p.legs[l];
                if leg.stepping {
                    leg.step_time += dtf * speed;
                    if leg.step_time > 1.0 {
                        leg.stepping = false;
                        leg.step_time = 0.0;
                        leg.foot_down_time = 0.0;
                    }
                } else {
                    leg.foot_down_time += dtf * speed;
                    if leg.foot_down_time > 1.0 && other_planted && vx > 1.0 {
                        leg.stepping = true;
                        leg.step_time = 0.0;
                        if grounded && playing && !p.wall_grab {
                            let c = if l == 0 {
                                "SFX_Character_Step_Left_SE"
                            } else {
                                "SFX_Character_Step_Right_SE"
                            };
                            let c = if scale > 1.5 {
                                if l == 0 {
                                    "SFX_Character_Step_Big_Left_SE"
                                } else {
                                    "SFX_Character_Step_Big_Right_SE"
                                }
                            } else {
                                c
                            };
                            self.events.sound(c, pos, vx);
                        }
                        break;
                    }
                }
            }
        }
    }

    /// Leg rays (5.6): the supporting ground hit under each leg
    /// (`(leg point, hit, target)`), or None.
    fn leg_rays(
        &self,
        content: &Content,
        i: usize,
        circles: &[Option<Circle>],
    ) -> [Option<(V2, RayHit, Target)>; 2] {
        let p = &self.players[i];
        let (pos, scale) = (p.pos, p.scale);
        let mut hits = [None, None];
        let scene = self.scene(content, circles);
        for (k, sx) in [-1.0f32, 1.0].iter().enumerate() {
            let leg = pos + v2(0.24 * sx, -0.36) * scale;
            let o = leg + v2(0.0, 0.5);
            if let Some((h, t)) =
                scene.raycast(o, V2::DOWN, 1.0 * scale, layers::LEG_RAY, Some(i as u8))
                && p.since_jump >= 0.2
                && h.normal.angle_deg(V2::UP) <= 70.0
            {
                hits[k] = Some((leg, h, t));
            }
        }
        hits
    }

    /// `TouchGround` for one supporting leg hit; returns `(landed, old
    /// sinceGrounded)`.
    fn leg_touch_ground(
        &mut self,
        content: &Content,
        i: usize,
        h: &RayHit,
        t: Target,
    ) -> (bool, f32) {
        let prop = match t {
            Target::Prop(pi) => {
                let def = self.physics.props[pi as usize].def;
                let pushable = self
                    .map
                    .current
                    .map(|m| content.map(m).props[def as usize].pushable.is_some())
                    .unwrap_or(true);
                Some((pi, pushable))
            }
            _ => None,
        };
        let r = self.players[i].touch_ground(h.point, h.normal, prop);
        self.fire_hook(
            content,
            i,
            Hook::TouchGround {
                since_grounded: r.1,
                point: h.point,
                normal: h.normal,
            },
        );
        r
    }

    /// Ground contact only (no spring forces), for the spawn flight.
    fn leg_ground_contacts(&mut self, content: &Content, i: usize, circles: &[Option<Circle>]) {
        for (_, h, t) in self.leg_rays(content, i, circles).into_iter().flatten() {
            self.leg_touch_ground(content, i, &h, t);
        }
    }

    fn legs(&mut self, content: &Content, i: usize, circles: &[Option<Circle>], dtf: f32) {
        let dir_x = self.players[i].controls.direction.x;
        let scale = self.players[i].scale;
        self.leg_steps(i, dtf);
        let total_step: f32 = self.players[i]
            .legs
            .iter()
            .filter(|l| l.stepping)
            .map(|l| l.step_time)
            .sum();
        let hits = self.leg_rays(content, i, circles);
        for (leg, h, t) in hits.into_iter().flatten() {
            let vy = self.players[i].vel.y;
            let (landed, old_sg) = self.leg_touch_ground(content, i, &h, t);
            if landed && old_sg > 0.05 {
                let big = self.players[i].scale > 1.5;
                self.events
                    .sound(if big { cue::LAND_BIG } else { cue::LAND }, h.point, old_sg);
                self.events.push(EventKind::Landed {
                    player: i as u8,
                    pos: h.point,
                    normal: h.normal,
                    impact: -vy,
                });
                if vy < -20.0 {
                    self.events.shake(V2::DOWN * clamp(old_sg - 0.5, 0.0, 4.0));
                }
            }
            let p = &mut self.players[i];
            let hh = (h.point.y - leg.y) / scale;
            let a = hh.abs();
            if dir_x != 0.0 {
                p.vel.y += 0.02 * eval_curve(&BOB_CURVE, total_step);
            }
            let c = support_curve(a);
            p.vel.y += 0.02 * 1000.0 * c;
            p.vel.y -= p.vel.y * 0.02 * 100.0 * c;
        }
    }

    fn player_collision(
        &mut self,
        content: &Content,
        i: usize,
        circles: &[Option<Circle>],
        ts: f32,
    ) {
        // 1. Tunnelling guard.
        let (pos, last, ignore) = {
            let p = &self.players[i];
            (p.pos, p.last_pos, p.ignore_wall_frames)
        };
        if ignore <= 0 {
            let d = pos - last;
            let l = d.len();
            if l > 1e-6 {
                let dir = d / l;
                let hit = self.scene(content, circles).raycast_filtered(
                    last,
                    dir,
                    l,
                    layers::PLAYER_COLLISION,
                    Some(i as u8),
                    // Port decision (7.3): a ray starting inside a collider
                    // (t = 0, normal = -dir) is not a tunnelling hit; the
                    // push-out below resolves embedded centres. Accepting it
                    // pushed an embedded player 0.5 deeper every tick.
                    |h, _| h.t > 0.0 && h.normal.dot(dir) <= 0.0,
                );
                if let Some((h, _)) = hit {
                    self.players[i].pos = h.point + h.normal * 0.5;
                    if self.players[i].flying_for > 0.0 {
                        self.flying_bounce(content, i, h.normal);
                    }
                }
            }
        }
        // 2.
        {
            let p = &mut self.players[i];
            p.ignore_wall_frames -= 1;
            p.last_pos = p.pos;
        }
        // 3. Push-out. Port decision (7.3): a centre inside map geometry
        // first moves to the nearest point outside all overlapping colliders
        // (per-collider nearest faces can point into a neighbour and cancel
        // out); the circle contacts then push as usual.
        {
            let (pos, movable) = {
                let p = &self.players[i];
                (p.pos, p.simulated || !p.kinematic)
            };
            if movable
                && let Some(q) = self
                    .scene(content, circles)
                    .exit_point(pos, layers::PLAYER_COLLISION)
            {
                self.players[i].pos = q;
            }
        }
        let mut hits = Vec::new();
        let (pos, r) = (self.players[i].pos, self.players[i].radius());
        self.scene(content, circles).overlaps(
            pos,
            r,
            layers::PLAYER_COLLISION,
            Some(i as u8),
            &mut hits,
        );
        let r2 = 0.75 * r;
        for (t, point, inside) in hits {
            let cur = self.players[i].pos;
            let (mut n, dist) = if inside {
                // Centre inside the collider (another player's circle, an
                // animated collider): push toward the nearest surface as seen
                // from the query position (port decision; the original's
                // contact point is engine-defined). The penetration is
                // measured along that direction from the current position,
                // so earlier pushes of this tick are not undone.
                let n = (point - pos).normalized();
                (n, (cur - point).dot(n))
            } else {
                let delta = cur - point;
                (delta.normalized(), delta.len())
            };
            if n.is_zero() {
                n = V2::UP;
            }
            let pen = clamp(r - dist, 0.0, 10.0);
            let pen2 = clamp(r2 - dist, 0.0, 10.0);
            if let Target::Prop(pi) = t {
                self.prop_push(content, i, pi as usize);
            }
            let p = &mut self.players[i];
            if p.simulated || !p.kinematic {
                p.pos += n * pen2;
                let dirx = p.controls.direction;
                if n.y.abs() < 0.45
                    && dirx.x.abs() > 0.1
                    && dirx.angle_deg(n) > 90.0
                    && let Some(old) = p.touch_wall(n, point)
                {
                    let s = old.min(p.since_grounded);
                    let ppos = p.pos;
                    let big = p.scale > 1.5;
                    if s > 0.05 {
                        self.events.sound(
                            if big {
                                cue::STICK_WALL_BIG
                            } else {
                                cue::STICK_WALL
                            },
                            point,
                            s,
                        );
                        self.events.push(EventKind::WallStick {
                            player: i as u8,
                            pos: ppos,
                            normal: n,
                        });
                    }
                    self.fire_hook(
                        content,
                        i,
                        Hook::TouchWall {
                            since_wall_grab: old,
                            point,
                            normal: n,
                        },
                    );
                }
                let p = &mut self.players[i];
                p.vel += n * (pen * 10.0 * ts);
                p.vel -= p.vel * (pen * ts);
            }
            if let Target::Player(o) = t {
                self.fire_hook(
                    content,
                    i,
                    Hook::PlayerCollision {
                        point,
                        push: n * pen,
                        other: o,
                    },
                );
            }
            if self.players[i].flying_for > 0.0 {
                self.flying_bounce(content, i, n);
            }
        }
        // The contacts of this tick (another player's push in particular) may
        // have moved the centre back into map geometry: resolve it again so a
        // tick never ends with the centre embedded (port decision, 7.3).
        let p = &self.players[i];
        if (p.simulated || !p.kinematic)
            && let Some(q) = self
                .scene(content, circles)
                .exit_point(p.pos, layers::PLAYER_COLLISION)
        {
            self.players[i].pos = q;
        }
        self.players[i].last_pos = self.players[i].pos;
    }

    /// Flying bounce (7.4).
    fn flying_bounce(&mut self, content: &Content, i: usize, n: V2) {
        let p = &self.players[i];
        if p.bounce.is_some() || p.vel.angle_deg(n) < 90.0 || p.vel.len() <= 1.0 {
            return;
        }
        let target = p.vel.reflect(n);
        self.start_bounce(content, i, target);
    }

    pub(crate) fn start_bounce(&mut self, content: &Content, i: usize, target: V2) {
        let pos = self.players[i].pos;
        self.events.sound(cue::BOUNCE, pos, 1.0);
        self.events.push(EventKind::Bounced {
            player: i as u8,
            dir: target.normalized(),
        });
        self.events.shake(target.normalized() * 4.0);
        self.players[i].bounce = Some(Bounce {
            target_vel: target,
            ticks: ticks(0.25),
        });
        self.add_stun(i, 0.2);
        self.call_take_damage(content, i, target.normalized() * 5.0, pos, None, true);
    }

    /// Player pushes a prop and the prop's impact check (player.md 7.5).
    fn prop_push(&mut self, content: &Content, i: usize, pi: usize) {
        let Some(mid) = self.map.current else { return };
        let prop = &self.physics.props[pi];
        let Some(push) = content.map(mid).props[prop.def as usize].pushable else {
            return;
        };
        let p = &self.players[i];
        let dir_in = p.controls.direction * 8.0;
        let q = prop.aabb.closest_point(p.pos);
        let angle = dir_in.angle_deg(q - p.pos);
        let k = (90.0 - angle) / 90.0;
        let dtf = DT * self.time.scale();
        let force = dir_in * (dtf * k * push.speed * 1000.0);
        let collision = prop.vel
            * clamp(
                (prop.vel.angle_deg(prop.pos - p.pos) - 90.0) / 90.0,
                0.0,
                1.0,
            );
        let (mass, frame_ok) = (
            prop.mass,
            self.frame.wrapping_sub(prop.last_player_impact) >= ticks(1.0)
                || prop.last_player_impact == 0,
        );
        if !force.is_zero() {
            self.physics.add_force_at(pi, force, q);
        }
        // Impact check.
        if !frame_ok {
            return;
        }
        let vec = collision * push.dmg_amount;
        if vec.len() < push.player_col_threshold {
            return;
        }
        self.physics.props[pi].last_player_impact = self.frame;
        let m = mass;
        let pos = self.players[i].pos;
        let dmg = vec * (0.3 * (m / 20000.0) * (m / 20000.0));
        self.call_take_damage(content, i, dmg, pos, None, true);
        let f = collision * (push.force_amount * crate::math::sqrt(m / 20000.0));
        self.take_force(i, f, ForceMode::Impulse, false, false, vec.len() * 0.05);
        if self.players[i].is_blocking() {
            self.physics.scale_velocity(pi, -1.1);
        } else {
            if m < 80000.0 {
                self.physics.scale_velocity(pi, -0.5 * 20000.0 / m);
            }
            if self.players[i].bounce.is_none() {
                let v = self.players[i].vel;
                self.start_bounce(content, i, v);
            }
        }
    }

    /// `Jump(force, multiplier)` (6.1).
    pub(crate) fn jump(&mut self, content: &Content, i: usize, force: bool, multiplier: f32) {
        let p = &self.players[i];
        if !force && (p.since_jump < 0.1 || (p.current_jumps <= 0 && p.since_wall_grab > 0.1)) {
            return;
        }
        self.fire_hook(content, i, Hook::Jump);
        let p = &mut self.players[i];
        let wall = p.since_wall_grab < 0.1 && !p.grounded;
        let dir = if wall {
            v2(0.0, 0.8) + p.wall_normal * 0.4
        } else {
            V2::UP
        };
        p.current_jumps = p.stats.jumps;
        if p.vel.y < 0.0 {
            p.vel.y = 0.0;
        }
        let dust_at = if wall {
            p.wall_pos
        } else if p.since_grounded > 0.05 {
            p.pos
        } else {
            p.ground_pos
        };
        p.since_grounded = 0.0;
        p.since_jump = 0.0;
        p.grounded = false;
        p.wall_grab = false;
        p.current_jumps -= 1;
        let k = multiplier * 0.01 * p.stats.jump * (1.0 - p.get_slow());
        p.vel += dir * (k * 5500.0);
        if !wall {
            p.vel.x += k * 1.0 * p.vel.x;
        }
        let big = p.scale > 1.5;
        let v = p.vel;
        self.events
            .sound(if big { cue::JUMP_BIG } else { cue::JUMP }, dust_at, 1.0);
        self.events.push(EventKind::Jumped {
            player: i as u8,
            pos: dust_at,
            dir: v.normalized(),
            wall,
        });
    }

    /// Out of bounds (8).
    fn out_of_bounds(&mut self, content: &Content, i: usize, dtu: f32) {
        let b = content.tuning.bounds_half;
        let p = &mut self.players[i];
        if !p.simulated || !p.playing {
            return;
        }
        let w = b.x * 2.0;
        let h = b.y * 2.0;
        let nx = clamp((p.pos.x + b.x) / w, 0.0, 1.0);
        let ny = clamp((p.pos.y + b.y) / h, 0.0, 1.0);
        let outside = nx <= 0.0 || nx >= 1.0 || ny <= 0.0 || ny >= 1.0;
        let warn = !outside && (nx < 0.1 || nx > 0.9 || ny < 0.1 || ny > 0.9);
        let mut m = v2(nx * w - b.x, ny * h - b.y);
        if warn {
            if nx < 0.1 {
                m.x = -b.x;
            } else if nx > 0.9 {
                m.x = b.x;
            }
            if ny < 0.1 {
                m.y = -b.y;
            } else if ny > 0.9 {
                m.y = b.y;
            }
        }
        let up = (m - p.pos).normalized();
        p.oob_acc += dtu;
        if warn && p.oob_acc > 0.1 {
            p.oob_acc = 0.0;
            let pos = m;
            self.events.push(EventKind::OutOfBoundsWarning {
                player: i as u8,
                pos,
                inward: up,
            });
            return;
        }
        if !outside {
            return;
        }
        p.since_grounded = 0.0;
        if p.oob_acc <= 0.1 {
            return;
        }
        p.oob_acc = 0.0;
        let mass = p.mass;
        let blocking = p.is_blocking();
        self.events.sound(cue::DAMAGE_SCREEN_EDGE, m, 1.0);
        self.events.push(EventKind::OutOfBounds {
            player: i as u8,
            pos: m,
            inward: up,
            shielded: blocking,
        });
        if blocking {
            self.players[i].vel = V2::ZERO;
            self.take_force(i, up * (400.0 * mass), ForceMode::Impulse, false, true, 0.0);
            let p = &mut self.players[i];
            p.pos = m;
            p.last_pos = m;
        } else {
            self.take_force(i, up * (200.0 * mass), ForceMode::Impulse, false, true, 0.0);
            let pos = self.players[i].pos;
            self.call_take_damage(content, i, up * 51.0, pos, None, true);
        }
    }

    // ------------------------------------------------------------------
    // Forces, status effects
    // ------------------------------------------------------------------

    fn status_blocked_cue(&mut self, i: usize) {
        let p = &mut self.players[i];
        if self.frame.wrapping_sub(p.status_cue_frame) >= ticks(0.25) || p.status_cue_frame == 0 {
            p.status_cue_frame = self.frame;
            let pos = p.pos;
            self.events.sound(cue::BLOCK_STATUS, pos, 1.0);
            self.events
                .push(EventKind::StatusEffectBlocked { player: i as u8 });
        }
    }

    /// `HealthHandler.TakeForce` (7.1).
    pub fn take_force(
        &mut self,
        i: usize,
        f: V2,
        mode: ForceMode,
        ignore_mass: bool,
        ignore_block: bool,
        set_flying: f32,
    ) {
        let p = &self.players[i];
        if !p.playing || !p.simulated || p.dead || p.respawning {
            return;
        }
        if p.is_blocking() {
            if !ignore_block {
                self.status_blocked_cue(i);
                return;
            }
        } else if set_flying > p.flying_for && set_flying > 0.25 {
            let pos = p.pos;
            self.players[i].flying_for = set_flying;
            self.events.sound(cue::BOUNCE, pos, 1.0);
        }
        let p = &mut self.players[i];
        let mut f = f;
        if ignore_mass {
            f *= p.mass / 100.0;
        }
        match mode {
            ForceMode::Impulse => p.vel += f / p.mass,
            ForceMode::Force => p.vel += f * 0.02 / p.mass,
        }
        if f.y > 0.0 {
            let mut y = f.y;
            if !ignore_mass {
                y /= p.mass / 100.0;
            }
            if mode == ForceMode::Force {
                y *= 0.003;
            }
            p.since_grounded -= y * 0.003;
        }
        p.since_grounded = clamp(p.since_grounded, -0.5, 100.0);
    }

    /// `AddStun` (15.1).
    pub fn add_stun(&mut self, i: usize, t: f32) {
        let p = &self.players[i];
        if p.dead || p.respawning {
            return;
        }
        if p.is_blocking() {
            self.status_blocked_cue(i);
            return;
        }
        let p = &mut self.players[i];
        p.stun_time = p.stun_time.max(t);
        if !p.stunned && p.stun_time > 0.0 {
            p.stunned = true;
            p.vel = V2::ZERO;
            p.kinematic = true;
            self.events.push(EventKind::Stunned {
                player: i as u8,
                start: true,
            });
        }
    }

    /// `RPCA_AddSilence` (15.2); callers check blocking.
    pub fn add_silence(&mut self, i: usize, t: f32) {
        let p = &mut self.players[i];
        if p.dead || p.respawning {
            return;
        }
        p.silence_time = p.silence_time.max(t);
        if !p.silenced && p.silence_time > 0.0 {
            p.silenced = true;
            self.events.push(EventKind::Silenced {
                player: i as u8,
                start: true,
            });
        }
    }

    fn do_slow_down(&mut self, i: usize, new: f32) {
        let p = &mut self.players[i];
        let delta = clamp(new - p.slow, 0.0, 1.0);
        let particles = clamp((new * 0.1 + delta * 0.7) * 50.0, 1.0, 50.0) as u32;
        p.vel *= 1.0 - delta;
        p.since_grounded *= 1.0 - delta;
        let pos = p.pos;
        self.events.sound(cue::SLOW_FREEZE, pos, new);
        self.events.push(EventKind::Slowed {
            player: i as u8,
            amount: new,
            particles,
        });
    }

    /// `RPCA_AddSlow` (14.3; bullets): callers check blocking.
    pub fn add_slow(&mut self, i: usize, amount: f32, fast: bool) {
        if self.players[i].dead {
            return;
        }
        self.do_slow_down(i, amount);
        let p = &mut self.players[i];
        if fast {
            p.fast_slow = clamp(p.fast_slow, amount, 1.0);
        } else {
            p.slow_slow = clamp(p.slow_slow, amount, 10.0);
        }
    }

    /// `AddSlowAddative` (14.3; area effects).
    pub fn add_slow_additive(&mut self, i: usize, amount: f32, max: f32, fast: bool) {
        if self.players[i].is_blocking() {
            self.status_blocked_cue(i);
            return;
        }
        self.do_slow_down(i, amount);
        let p = &mut self.players[i];
        if fast {
            if p.fast_slow < max {
                // Original quirk: assigns from `slow` (player.md 22.3).
                p.fast_slow = clamp(p.slow, 0.0, max);
            }
        } else if p.slow_slow < max {
            p.slow_slow = clamp(p.slow_slow + amount, 0.0, max);
        }
    }

    // ------------------------------------------------------------------
    // Health
    // ------------------------------------------------------------------

    /// `Heal` (11.8).
    pub fn heal(&mut self, i: usize, amount: f32) {
        let p = &mut self.players[i];
        if amount == 0.0 || p.health == p.stats.max_health || p.dead {
            return;
        }
        p.health = (p.health + amount).min(p.stats.max_health);
        let pos = p.pos;
        if self.frame.wrapping_sub(p.heal_sound_frame) >= ticks(0.1) || p.heal_sound_frame == 0 {
            p.heal_sound_frame = self.frame;
            self.events.sound(cue::HEAL, pos, amount);
            self.events.push(EventKind::Healed {
                player: i as u8,
                amount,
            });
        }
    }

    /// `CallTakeDamage` (11.3): no-op while blocking; else `TakeDamage`
    /// with the block already decided.
    pub fn call_take_damage(
        &mut self,
        content: &Content,
        i: usize,
        dmg: V2,
        pos: V2,
        attacker: Option<u8>,
        lethal: bool,
    ) {
        if dmg.is_zero() || self.players[i].is_blocking() {
            return;
        }
        self.take_damage(content, i, dmg, pos, attacker, lethal, true);
    }

    /// `TakeDamage` (11.3) including the DoT conversion (11.4).
    #[allow(clippy::too_many_arguments)]
    pub fn take_damage(
        &mut self,
        content: &Content,
        i: usize,
        dmg: V2,
        pos: V2,
        attacker: Option<u8>,
        lethal: bool,
        ignore_block: bool,
    ) {
        let p = &self.players[i];
        if dmg.is_zero()
            || !p.playing
            || !p.simulated
            || p.dead
            || (p.is_blocking() && !ignore_block)
        {
            return;
        }
        let t = p.stats.seconds_to_take_damage_over;
        if t == 0.0 {
            self.do_damage(content, i, dmg, pos, attacker, lethal, ignore_block);
        } else {
            let total = dmg.len();
            let interval = 0.25;
            let chunk = total / t * interval;
            self.players[i].dots.push(Dot {
                total,
                dealt: 0.0,
                chunk,
                dir: dmg.normalized(),
                attacker,
                wait_ticks: 0,
                interval,
                lethal,
                cue: cue::DAMAGE_PASSIVE,
            });
            self.tick_dots(content, i, false);
        }
    }

    /// Damage over time (weapons.md 14.3): first chunk immediately, then
    /// every `interval` game seconds.
    pub fn take_damage_over_time(
        &mut self,
        content: &Content,
        i: usize,
        dmg: V2,
        time: f32,
        interval: f32,
        attacker: Option<u8>,
    ) {
        self.take_damage_over_time_cue(
            content,
            i,
            dmg,
            time,
            interval,
            attacker,
            cue::DAMAGE_PASSIVE,
        );
    }

    /// `take_damage_over_time` with a tick sound (poison).
    #[allow(clippy::too_many_arguments)]
    pub fn take_damage_over_time_cue(
        &mut self,
        content: &Content,
        i: usize,
        dmg: V2,
        time: f32,
        interval: f32,
        attacker: Option<u8>,
        tick_cue: &'static str,
    ) {
        let p = &self.players[i];
        if !p.playing || p.dead || p.respawning {
            return;
        }
        let total = dmg.len();
        if total == 0.0 || time <= 0.0 {
            return;
        }
        let chunk = total / time * interval;
        self.players[i].dots.push(Dot {
            total,
            dealt: 0.0,
            chunk,
            dir: dmg.normalized(),
            attacker,
            wait_ticks: 0,
            interval,
            lethal: true,
            cue: tick_cue,
        });
        self.tick_dots(content, i, false);
    }

    /// Deals due DoT chunks; `advance` counts one real tick down first.
    fn tick_dots(&mut self, content: &Content, i: usize, advance: bool) {
        let mut k = 0;
        while k < self.players[i].dots.len() {
            if self.players[i].dead || self.players[i].respawning {
                self.players[i].dots.clear();
                return;
            }
            let mut d = self.players[i].dots[k];
            if advance {
                d.wait_ticks = d.wait_ticks.saturating_sub(1);
            }
            let mut finished = false;
            if d.wait_ticks == 0 {
                let pos = self.players[i].pos;
                if self.players[i].playing {
                    self.events.sound(d.cue, pos, d.chunk);
                }
                d.dealt += d.chunk;
                d.wait_ticks = ticks(d.interval / self.time.scale().max(1e-6)).max(1);
                finished = d.dealt >= d.total;
                if let Some(slot) = self.players[i].dots.get_mut(k) {
                    *slot = d;
                }
                self.do_damage(
                    content,
                    i,
                    d.dir * d.chunk,
                    pos,
                    d.attacker,
                    d.lethal,
                    false,
                );
                if self.players[i].dots.is_empty() {
                    return;
                }
            } else if let Some(slot) = self.players[i].dots.get_mut(k) {
                *slot = d;
            }
            if finished {
                self.players[i].dots.remove(k);
            } else {
                k += 1;
            }
        }
    }

    /// `DoDamage` (11.3).
    #[allow(clippy::too_many_arguments)]
    pub fn do_damage(
        &mut self,
        content: &Content,
        i: usize,
        dmg: V2,
        pos: V2,
        attacker: Option<u8>,
        lethal: bool,
        ignore_block: bool,
    ) {
        let p = &self.players[i];
        if dmg.is_zero()
            || !p.playing
            || p.dead
            || (p.is_blocking() && !ignore_block)
            || p.respawning
        {
            return;
        }
        let amount = dmg.len();
        let self_damage = attacker == Some(i as u8);
        if let Some(a) = attacker {
            self.dealt_damage(content, a as usize, dmg, self_damage, i as u8);
        }
        let p = &mut self.players[i];
        // `StopAllCoroutines()` on the HealthHandler: running force pulls
        // (Implosion) end (player.md 11.3, Clarified).
        p.pulls.clear();
        p.last_source_of_damage = attacker;
        p.health -= amount;
        self.fire_hook(
            content,
            i,
            Hook::WasDealtDamage {
                damage: dmg,
                self_damage,
            },
        );
        let p = &mut self.players[i];
        if !lethal {
            p.health = clamp(p.health, 1.0, p.stats.max_health);
        }
        let killed = p.health < 0.0 && !p.dead;
        self.events.push(EventKind::Damaged {
            player: i as u8,
            amount,
            dir: dmg.normalized(),
            pos,
            attacker,
            lethal,
        });
        if killed {
            if self.players[i].remaining_respawns > 0 {
                self.phoenix_die(content, i, dmg);
            } else {
                self.die(content, i, dmg);
            }
        }
        if let Some(a) = attacker {
            let ai = a as usize;
            if self.players[ai].stats.life_steal != 0.0 && !self_damage {
                let ap = &mut self.players[ai];
                if self.frame.wrapping_sub(ap.lifesteal_sound_frame) >= ticks(0.15)
                    || ap.lifesteal_sound_frame == 0
                {
                    ap.lifesteal_sound_frame = self.frame;
                    let apos = ap.pos;
                    self.events.sound(cue::DAMAGE_LIFE_STEAL, apos, amount);
                }
            }
        }
    }

    /// `DealtDamage` on the attacker (14.4).
    fn dealt_damage(
        &mut self,
        content: &Content,
        a: usize,
        dmg: V2,
        self_damage: bool,
        victim: u8,
    ) {
        if self.players[a].stats.life_steal != 0.0 && !self_damage {
            let ls = self.players[a].stats.life_steal;
            self.heal(a, dmg.len() * ls);
        }
        if self.players[a].stats.refresh_on_damage {
            self.players[a].gun.since_attack = f32::INFINITY;
        }
        self.fire_hook(
            content,
            a,
            Hook::DealtDamage {
                damage: dmg,
                self_damage,
                victim,
            },
        );
        let p = &mut self.players[a];
        p.last_damaged_player = Some(victim);
        if !self_damage {
            p.since_dealt_damage = 0.0;
        }
    }

    /// `Die` (11.5).
    pub fn die(&mut self, content: &Content, i: usize, dir: V2) {
        let p = &mut self.players[i];
        if !p.playing || p.dead {
            return;
        }
        p.dead = true;
        p.dots.clear();
        p.pulls.clear();
        p.stun_time = 0.0;
        p.stunned = false;
        p.silence_time = 0.0;
        p.silenced = false;
        p.bounce = None;
        p.pending_blocks.clear();
        let pos = p.pos;
        let killer = p.last_source_of_damage;
        let d = if dir.len() < 30.0 {
            dir.normalized() * 30.0
        } else {
            dir
        };
        self.events.sound(cue::DIE, pos, 1.0);
        self.events.shake(dir.normalized() * 3.0);
        self.events.push(EventKind::Died {
            player: i as u8,
            dir: d,
            killer,
        });
        self.fire_hook(content, i, Hook::Death);
        self.flow.deaths.push(i as u8);
    }

    /// Phoenix death (11.6).
    fn phoenix_die(&mut self, content: &Content, i: usize, dir: V2) {
        let p = &mut self.players[i];
        p.remaining_respawns -= 1;
        p.respawning = true;
        p.dots.clear();
        p.pulls.clear();
        p.stun_time = 0.0;
        p.stunned = false;
        p.silence_time = 0.0;
        p.silenced = false;
        p.bounce = None;
        p.phoenix_ticks = ticks(2.6);
        let pos = p.pos;
        self.events.sound(cue::DIE, pos, 1.0);
        self.events.shake(dir.normalized() * 3.0);
        self.events.push(EventKind::PhoenixDied {
            player: i as u8,
            dir,
        });
        self.fire_hook(content, i, Hook::Death);
    }

    /// `Revive(full)` (11.7).
    pub fn revive(&mut self, content: &Content, i: usize, full: bool) {
        if self.players[i].disconnected {
            return;
        }
        self.fire_hook(content, i, Hook::Revive { full });
        let p = &mut self.players[i];
        p.flying_for = 0.0;
        if full {
            p.remaining_respawns = p.stats.respawns;
        }
        p.respawning = false;
        p.health = p.stats.max_health;
        p.vel = V2::ZERO;
        p.stun_time = 0.0;
        p.stunned = false;
        if p.simulated {
            p.kinematic = false;
        }
        p.silence_time = 0.0;
        p.silenced = false;
        p.block.active = true;
        p.block.counter = p.block.stats.cooldown() + 1.0;
        p.gun.reload_silent();
        p.ignore_wall_frames = 5;
        p.dead = false;
        p.slow = 0.0;
        p.slow_slow = 0.0;
        p.fast_slow = 0.0;
        p.block.since_block = f32::INFINITY;
        p.dots.clear();
        p.pulls.clear();
        p.bounce = None;
        p.last_pos = p.pos;
        self.events.push(EventKind::Revived {
            player: i as u8,
            full,
        });
    }

    // ------------------------------------------------------------------
    // Block
    // ------------------------------------------------------------------

    /// `DoBlock` (12.4).
    #[allow(clippy::too_many_arguments)]
    pub fn do_block(
        &mut self,
        content: &Content,
        i: usize,
        first: bool,
        dont_set_cd: bool,
        kind: BlockKind,
        at: Option<V2>,
        only_effects: bool,
    ) {
        if kind == BlockKind::Default && first {
            let n = self.players[i].block.stats.additional_blocks.max(0);
            for k in 0..n {
                self.players[i].pending_blocks.push(PendingBlock {
                    ticks: ticks(0.2 * (k + 1) as f32),
                    first: false,
                    dont_set_cd: true,
                    kind: BlockKind::Echo,
                    only_effects: false,
                    continuation: false,
                    at: None,
                });
            }
        }
        self.players[i].block.active = false;
        let saved = at.map(|a| {
            let p = &mut self.players[i];
            let s = p.pos;
            p.pos = a;
            s
        });
        self.fire_hook(
            content,
            i,
            Hook::Block {
                phase: BlockPhase::SuperFirst,
                kind,
            },
        );
        self.fire_hook(
            content,
            i,
            Hook::Block {
                phase: BlockPhase::FirstDelaysOthers,
                kind,
            },
        );
        if !only_effects {
            self.players[i].block.since_block = 0.0;
        }
        if let Some(s) = saved {
            self.players[i].pos = s;
        }
        if self.players[i].block.stats.delay_other_actions {
            self.players[i].pending_blocks.push(PendingBlock {
                ticks: ticks(0.2),
                first,
                dont_set_cd,
                kind,
                only_effects,
                continuation: true,
                at,
            });
            return;
        }
        self.finish_block(content, i, first, dont_set_cd, kind, at, only_effects);
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_block(
        &mut self,
        content: &Content,
        i: usize,
        first: bool,
        dont_set_cd: bool,
        kind: BlockKind,
        at: Option<V2>,
        only_effects: bool,
    ) {
        let saved = at.map(|a| {
            let p = &mut self.players[i];
            let s = p.pos;
            p.pos = a;
            s
        });
        self.fire_hook(
            content,
            i,
            Hook::Block {
                phase: BlockPhase::Early,
                kind,
            },
        );
        self.fire_hook(
            content,
            i,
            Hook::Block {
                phase: BlockPhase::Block,
                kind,
            },
        );
        if first {
            // forceToAdd / forceToAddUp: rejected while blocking (player.md 22.4).
            let p = &self.players[i];
            let (f1, f2, m, aim) = (
                p.block.stats.force_to_add,
                p.block.stats.force_to_add_up,
                p.mass,
                p.aim.normalized(),
            );
            if f1 != 0.0 {
                self.take_force(
                    i,
                    aim * (f1 * m * 0.01),
                    ForceMode::Impulse,
                    false,
                    false,
                    0.0,
                );
            }
            if f2 != 0.0 {
                self.take_force(
                    i,
                    V2::UP * (f2 * m * 0.01),
                    ForceMode::Impulse,
                    false,
                    false,
                    0.0,
                );
            }
        }
        let silent = kind == BlockKind::ShieldCharge
            || self.players[i]
                .cards
                .iter()
                .any(|c| content.card(*c).sound_disable_block_basic);
        let pos = self.players[i].pos;
        if !silent {
            self.events.sound(cue::BLOCK_START, pos, 1.0);
        }
        if !only_effects {
            self.events.push(EventKind::BlockStarted {
                player: i as u8,
                kind,
            });
        }
        let p = &mut self.players[i];
        if !dont_set_cd {
            p.block.counter = 0.0;
        }
        if !only_effects {
            p.block.since_block = 0.0;
        }
        // Block-spawned objects (cards).
        let spawns: Vec<_> = p
            .cards
            .iter()
            .flat_map(|c| content.card_block_spawns(*c).iter().copied())
            .collect();
        for o in spawns {
            self.spawn_object(content, o, pos, V2::RIGHT, 1.0, Some(i as u8), 1, None);
        }
        let heal = self.players[i].block.stats.healing;
        self.heal(i, heal);
        if let Some(s) = saved {
            self.players[i].pos = s;
        }
    }

    fn tick_pending_blocks(&mut self, content: &Content, i: usize) {
        let mut k = 0;
        while k < self.players[i].pending_blocks.len() {
            let pb = &mut self.players[i].pending_blocks[k];
            pb.ticks = pb.ticks.saturating_sub(1);
            if pb.ticks == 0 {
                let pb = self.players[i].pending_blocks.remove(k);
                if pb.continuation {
                    self.finish_block(
                        content,
                        i,
                        pb.first,
                        pb.dont_set_cd,
                        pb.kind,
                        pb.at,
                        pb.only_effects,
                    );
                } else {
                    self.do_block(
                        content,
                        i,
                        pb.first,
                        pb.dont_set_cd,
                        pb.kind,
                        None,
                        pb.only_effects,
                    );
                }
            } else {
                k += 1;
            }
        }
    }

    /// `PlayerImmunity.IsImmune` (11.9): true = immune.
    pub fn is_immune(&mut self, i: usize, time: f32, damage: f32, key: u32) -> bool {
        let p = &mut self.players[i];
        if let Some(e) = p.immunity.iter_mut().find(|e| e.key == key) {
            if damage > e.damage {
                e.damage = damage;
                e.time = time;
                false
            } else {
                true
            }
        } else {
            p.immunity.push(Immunity { key, damage, time });
            false
        }
    }

    /// Teleport a player (moves, zero velocity, ignore walls 2 frames).
    /// Mutable access to a player for tests and debugging tools (bypasses
    /// the rules; not for gameplay code or presentation).
    pub fn player_mut(&mut self, i: usize) -> &mut Player {
        &mut self.players[i]
    }

    pub fn teleport(&mut self, i: usize, to: V2) {
        let p = &mut self.players[i];
        p.pos = to;
        p.last_pos = to;
        p.vel = V2::ZERO;
        p.ignore_wall_frames = 2;
        p.stand_last = None;
    }
}
