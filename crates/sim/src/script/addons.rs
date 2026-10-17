//! Projectile add-ons (`ObjectsToSpawn.AddToProjectile` prefabs): movement
//! behaviours that run before the projectile's movement integration and hit
//! effects that run in `ProjectileHit` step 9 (weapons.md 9, 16, 20).

use crate::content::{Content, SpawnEntry};
use crate::events::{EventKind, HitTarget};
use crate::geom::layers;
use crate::math::{V2, clamp, pow, rotate_towards};
use crate::world::{World, ticks};

/// `RayHitBash` parameters (weapons.md 16.4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BashDef {
    pub base_trigger_chance: f32,
    pub trigger_chance_per_ten_damage: f32,
    pub base_stun_time: f32,
    pub stun_time_per_ten_damage: f32,
    pub stun_multiplier: f32,
    pub multiplier_per_ten_meter: f32,
    pub stun_time_exponent: f32,
    pub cannot_perma_stun: bool,
    pub stun_time_threshold: f32,
}

/// One add-on behaviour with its imported parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AddonDef {
    Homing {
        amount: f32,
        scaling_drag: f32,
        drag: f32,
        spread: f32,
        /// `soundHomingFound` cue (None: the default name).
        sound: Option<&'static str>,
    },
    Remote {
        rotate_speed: f32,
    },
    Drill {
        meters: f32,
        speed_mod: f32,
        speed_mod_flat: f32,
    },
    Poison {
        time: f32,
        interval: f32,
        /// `soundEventDamageOverTime` cue (None: the default name).
        sound: Option<&'static str>,
    },
    Bash(BashDef),
    TrickShot {
        multiplier: f32,
        remove_at: f32,
    },
    DodgeGround {
        force: f32,
        ray_length: f32,
        ray_up: f32,
        mask: u32,
    },
    /// `BounceEffectRetarget` (+ its `BounceTrigger`); `sound` =
    /// `soundTargetBounceTargetPlayer` (None: the default name).
    TargetBounce {
        sound: Option<&'static str>,
    },
    ScreenEdgeBounce,
    /// `MoveTransformGravity`.
    ExtraGravity {
        amount: f32,
        pow: f32,
    },
    /// `ProjectileHitEmpower` (the Empower card's hit effect).
    EmpowerHit,
}

/// Runtime state of one add-on on one projectile.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AddonState {
    None,
    /// Homing / remote: currently steering (holds a `simulateGravity` count).
    Steer {
        active: bool,
    },
    Drill {
        meters: f32,
        since: u32,
        drilling: bool,
        done: bool,
    },
    Trick {
        last_distance: f32,
        done: bool,
    },
    Gravity {
        counter: f32,
    },
    /// Target bounce: real ticks left of the pause, chosen target.
    Retarget {
        ticks: u32,
        target: Option<u8>,
    },
    Done {
        done: bool,
    },
}

/// An add-on instance: definition, its local scale (spawn-entry scale rules,
/// weapons.md 8) and state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Addon {
    pub def: AddonDef,
    pub scale: f32,
    pub st: AddonState,
}

impl Addon {
    pub fn new(def: AddonDef, scale: f32) -> Addon {
        let st = match def {
            AddonDef::Homing { .. } | AddonDef::Remote { .. } => {
                AddonState::Steer { active: false }
            }
            AddonDef::Drill { meters, .. } => AddonState::Drill {
                meters,
                since: 0,
                drilling: false,
                done: false,
            },
            AddonDef::TrickShot { .. } => AddonState::Trick {
                last_distance: 0.0,
                done: false,
            },
            AddonDef::ExtraGravity { .. } => AddonState::Gravity { counter: 0.0 },
            AddonDef::TargetBounce { .. } => AddonState::Retarget {
                ticks: 0,
                target: None,
            },
            AddonDef::EmpowerHit => AddonState::Done { done: false },
            _ => AddonState::None,
        };
        Addon { def, scale, st }
    }
}

/// Outcome of a hit effect (weapons.md 16).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitResult {
    CanContinue,
    HasToReturn,
    HasToReturnNow,
}

/// Build the add-on list of a new projectile from its spawn entries
/// (weapons.md 8: one instance per entry, scale from damage and stacks).
pub fn build_addons(content: &Content, spawns: &[SpawnEntry], damage: f32) -> Vec<Addon> {
    let mut out = Vec::new();
    let mut edge = false;
    for e in spawns {
        let Some(o) = e.add_to_projectile else {
            continue;
        };
        if e.remove_scripts {
            continue;
        }
        let mut scale = (1.0 - e.scale_from_damage) + damage / 55.0 * e.scale_from_damage;
        if e.scale_stacks {
            scale *= 1.0 + e.stacks as f32 * e.scale_stack_m;
        }
        scale *= content.object(o).scale;
        for d in content.addons(o) {
            if *d == AddonDef::ScreenEdgeBounce {
                // Duplicates remove themselves down to one (weapons.md 20.5).
                if edge {
                    continue;
                }
                edge = true;
            }
            out.push(Addon::new(*d, scale));
        }
    }
    out
}

impl World {
    /// Closest living player of any team (owner included) to `pos`,
    /// optionally only among players in line of sight (`GetClosestPlayer`,
    /// cards.md 5.5).
    pub(crate) fn closest_player(&self, content: &Content, pos: V2, vision: bool) -> Option<usize> {
        let mut best: Option<(f32, usize)> = None;
        for (i, p) in self.players.iter().enumerate() {
            if !p.hittable() || !p.playing {
                continue;
            }
            let d = (p.pos - pos).len_sq();
            // Vision is part of the search: the nearest *visible* player
            // (cards.md 5.5, Clarified).
            if best.is_none_or(|b| d < b.0) && (!vision || self.can_see(content, pos, i)) {
                best = Some((d, i));
            }
        }
        best.map(|(_, i)| i)
    }

    /// Closest living enemy of player `of` to `pos` (cards.md 5.5: N teams).
    pub(crate) fn closest_enemy(
        &self,
        content: &Content,
        pos: V2,
        of: usize,
        vision: bool,
    ) -> Option<usize> {
        let team = self.players.get(of).map(|p| p.team);
        let mut best: Option<(f32, usize)> = None;
        for (i, p) in self.players.iter().enumerate() {
            if !p.hittable() || !p.playing || Some(p.team) == team {
                continue;
            }
            let d = (p.pos - pos).len_sq();
            // Vision is part of the search: the nearest *visible* player
            // (cards.md 5.5, Clarified).
            if best.is_none_or(|b| d < b.0) && (!vision || self.can_see(content, pos, i)) {
                best = Some((d, i));
            }
        }
        best.map(|(_, i)| i)
    }

    /// `CanSeePlayer` (cards.md 5.4): only map geometry blocks.
    pub(crate) fn can_see(&self, content: &Content, from: V2, player: usize) -> bool {
        let to = self.players[player].pos;
        self.scene(content, &[])
            .line_of_sight_masked(from, to, content.tuning.line_of_sight_mask)
    }

    /// Movement behaviours of the add-ons, before movement integration.
    pub(crate) fn addons_update(&mut self, content: &Content, i: usize, dt: f32) {
        let n = self.projectiles[i].addons.len();
        for k in 0..n {
            let a = self.projectiles[i].addons[k];
            match a.def {
                AddonDef::Homing {
                    amount,
                    scaling_drag,
                    drag,
                    ..
                } => self.homing(content, i, k, amount, scaling_drag, drag, dt),
                AddonDef::Remote { rotate_speed } => self.remote(i, k, rotate_speed, dt),
                AddonDef::TrickShot {
                    multiplier,
                    remove_at,
                } => {
                    let p = &mut self.projectiles[i];
                    if let AddonState::Trick {
                        last_distance,
                        done: false,
                    } = a.st
                    {
                        let d = p.distance - last_distance;
                        p.damage *= 1.0 + d * dt * a.scale * multiplier;
                        let done = p.distance > remove_at;
                        p.addons[k].st = AddonState::Trick {
                            last_distance: p.distance,
                            done,
                        };
                    }
                }
                AddonDef::DodgeGround {
                    force,
                    ray_length,
                    ray_up,
                    mask,
                } => self.dodge_ground(content, i, force, ray_length, ray_up, mask, dt),
                AddonDef::ExtraGravity { amount, pow: pw } => {
                    let p = &mut self.projectiles[i];
                    if let AddonState::Gravity { counter } = a.st {
                        let c = counter + dt;
                        p.vel.y -= pow(amount * c, pw);
                        p.addons[k].st = AddonState::Gravity { counter: c };
                    }
                }
                AddonDef::TargetBounce { .. } => self.retarget_tick(content, i, k),
                AddonDef::Drill {
                    speed_mod,
                    speed_mod_flat,
                    ..
                } => {
                    // Exit detection (16.2): more than 2 ticks without a wall hit.
                    let p = &mut self.projectiles[i];
                    if let AddonState::Drill {
                        meters,
                        since,
                        drilling,
                        done,
                    } = a.st
                    {
                        let since = since + 1;
                        let mut meters = meters;
                        let mut drilling = drilling;
                        if drilling && since > 2 {
                            drilling = false;
                            p.allowed_to_spawn = true;
                            p.sim_speed =
                                p.sim_speed / speed_mod_flat * (p.start_speed * speed_mod);
                            if meters < 2.0 {
                                meters = -1.0;
                            }
                            let (id, pos) = (p.id, p.pos);
                            self.events.push(EventKind::DrillExit { id, pos });
                        }
                        self.projectiles[i].addons[k].st = AddonState::Drill {
                            meters,
                            since,
                            drilling,
                            done,
                        };
                    }
                }
                _ => {}
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn homing(
        &mut self,
        content: &Content,
        i: usize,
        k: usize,
        amount: f32,
        scaling_drag: f32,
        drag: f32,
        dt: f32,
    ) {
        let pos = self.projectiles[i].pos;
        let target = self.closest_player(content, pos, true);
        let was = matches!(
            self.projectiles[i].addons[k].st,
            AddonState::Steer { active: true }
        );
        let mut on = false;
        if let Some(t) = target {
            let desired = self.players[t].pos;
            let p = &mut self.projectiles[i];
            let to = desired - pos;
            let angle = p.vel.angle_deg(to);
            if angle < 70.0 {
                on = true;
                p.vel -= p.vel * (angle * dt * scaling_drag);
                p.vel -= p.vel * (dt * drag);
                p.vel += to.clamp_len(1.0) * (dt * p.start_speed * 0.025 * amount);
                p.vel.y += dt * p.gravity * p.multiplier;
                if !was {
                    p.simulate_gravity += 1;
                    let id = p.id;
                    let cue = match self.projectiles[i].addons[k].def {
                        AddonDef::Homing { sound: Some(s), .. } => s,
                        _ => crate::events::cue::HOMING_FOUND,
                    };
                    self.events.sound(cue, pos, 1.0);
                    self.events.push(EventKind::HomingLocked {
                        id,
                        target: t as u8,
                    });
                }
            }
        }
        if was && !on {
            self.projectiles[i].simulate_gravity -= 1;
        }
        self.projectiles[i].addons[k].st = AddonState::Steer { active: on };
    }

    fn remote(&mut self, i: usize, k: usize, rotate_speed: f32, dt: f32) {
        let Some(o) = self.projectiles[i].owner else {
            return;
        };
        let steer = self.players[o as usize].controls.aim;
        let was = matches!(
            self.projectiles[i].addons[k].st,
            AddonState::Steer { active: true }
        );
        let p = &mut self.projectiles[i];
        let on = steer.len() > 0.2 && p.since_reflect > 2.0;
        if on {
            let target = steer.normalized() * p.start_speed;
            p.vel = rotate_towards(p.vel, target, rotate_speed * dt, rotate_speed * dt * 10.0);
            if !was {
                p.simulate_gravity += 1;
            }
        } else if was {
            p.simulate_gravity -= 1;
        }
        p.addons[k].st = AddonState::Steer { active: on };
    }

    #[allow(clippy::too_many_arguments)]
    fn dodge_ground(
        &mut self,
        content: &Content,
        i: usize,
        force: f32,
        ray_length: f32,
        ray_up: f32,
        mask: u32,
        dt: f32,
    ) {
        let (pos, vel) = (self.projectiles[i].pos, self.projectiles[i].vel);
        let speed = vel.len();
        if speed < 1e-6 {
            return;
        }
        let fwd = vel / speed;
        let right = fwd.perp_right();
        let mask = mask & !layers::bit(layers::PLAYER);
        let mut hits = [None, None];
        {
            let scene = self.scene(content, &[]);
            for (n, s) in [1.0f32, -1.0].iter().enumerate() {
                let d = (fwd + right * (ray_up * s)).normalized();
                hits[n] = scene.raycast_filtered(pos, d, ray_length, mask, None, |_, t| {
                    // Damageable colliders (destructible props) do not count.
                    match t {
                        crate::scene::Target::Prop(pi) => self
                            .physics
                            .props
                            .get(pi as usize)
                            .is_none_or(|p| p.hp.is_none()),
                        crate::scene::Target::Player(_) => false,
                        _ => true,
                    }
                });
            }
        }
        if let (Some(a), Some(b)) = (hits[0], hits[1])
            && a.1 == b.1
        {
            if a.0.t <= b.0.t {
                hits[1] = None;
            } else {
                hits[0] = None;
            }
        }
        let mut v = vel;
        for (h, _) in hits.iter().flatten() {
            v += h.normal * (force * speed * dt);
        }
        if hits.iter().any(|h| h.is_some()) {
            self.projectiles[i].vel = v.normalized() * speed;
        }
    }

    fn retarget_tick(&mut self, content: &Content, i: usize, k: usize) {
        let AddonState::Retarget { ticks, target } = self.projectiles[i].addons[k].st else {
            return;
        };
        if ticks == 0 {
            return;
        }
        let t = ticks - 1;
        self.projectiles[i].addons[k].st = AddonState::Retarget { ticks: t, target };
        if t > 0 {
            return;
        }
        self.projectiles[i].paused = false;
        let Some(tp) = target else { return };
        let tp = tp as usize;
        if !self.players[tp].hittable() {
            return;
        }
        let _ = content;
        let p = &self.projectiles[i];
        let to = self.players[tp].pos;
        let speed = p.vel.len();
        // MoveTransform.GetUpwardsCompensation: horizontal distance only.
        let dx = (to.x - p.pos.x).abs();
        let comp = if speed > 1e-6 {
            pow(dx, 2.06) * p.gravity / speed * 0.012
        } else {
            0.0
        };
        let dir = (to + V2::UP * comp - p.pos).normalized();
        let id = p.id;
        let pos = p.pos;
        self.projectiles[i].vel = dir * speed;
        let cue = match self.projectiles[i].addons[k].def {
            AddonDef::TargetBounce { sound: Some(s) } => s,
            _ => crate::events::cue::TARGET_BOUNCE,
        };
        self.events.sound(cue, pos, 1.0);
        self.events.push(EventKind::TargetBounce {
            id,
            target: tp as u8,
        });
    }

    /// Reflect hooks of the add-ons (target bounce, weapons.md 20.4).
    pub(crate) fn addons_on_reflect(&mut self, content: &Content, i: usize, normal: V2) {
        let n = self.projectiles[i].addons.len();
        for k in 0..n {
            if matches!(
                self.projectiles[i].addons[k].def,
                AddonDef::TargetBounce { .. }
            ) {
                let pos = self.projectiles[i].pos;
                let t = self.closest_player(content, pos + normal * 0.1, false);
                let t = t.filter(|t| self.can_see(content, pos, *t));
                self.projectiles[i].addons[k].st = AddonState::Retarget {
                    ticks: ticks(0.1).max(1),
                    target: t.map(|x| x as u8),
                };
                self.projectiles[i].paused = true;
            }
        }
    }

    /// The drill hit effect (priority 100, runs first; 16.2).
    pub(crate) fn addons_drill_hit(
        &mut self,
        i: usize,
        point: V2,
        target: HitTarget,
        dt: f32,
    ) -> Option<HitResult> {
        let k = self.projectiles[i]
            .addons
            .iter()
            .position(|a| matches!(a.def, AddonDef::Drill { .. }))?;
        let a = self.projectiles[i].addons[k];
        let AddonDef::Drill {
            speed_mod,
            speed_mod_flat,
            ..
        } = a.def
        else {
            return None;
        };
        let AddonState::Drill {
            mut meters,
            drilling,
            done,
            ..
        } = a.st
        else {
            return None;
        };
        if done {
            return Some(HitResult::CanContinue);
        }
        let p = &mut self.projectiles[i];
        p.can_push_box = false;
        if matches!(target, HitTarget::Player(_) | HitTarget::Nothing) {
            return Some(HitResult::CanContinue);
        }
        let mut drilling = drilling;
        if !drilling && p.allowed_to_spawn {
            p.pos = point;
            p.sim_speed = p.sim_speed * speed_mod_flat / (p.start_speed * speed_mod).max(1e-6);
            drilling = true;
            let (id, pos) = (p.id, p.pos);
            self.events.push(EventKind::DrillEnter { id, pos });
        }
        let p = &mut self.projectiles[i];
        p.allowed_to_spawn = false;
        meters -= p.vel.len() * dt * p.sim_speed;
        let done = meters <= 0.0;
        p.addons[k].st = AddonState::Drill {
            meters,
            since: 0,
            drilling,
            done,
        };
        Some(if done {
            HitResult::CanContinue
        } else {
            HitResult::HasToReturnNow
        })
    }

    /// The other (priority 0) hit effects after the reflect effect: poison,
    /// bash, empower (16.3, 16.4, cards.md 6.5).
    pub(crate) fn addons_hit(&mut self, content: &Content, i: usize, point: V2, target: HitTarget) {
        let n = self.projectiles[i].addons.len();
        let poisons = self.projectiles[i]
            .addons
            .iter()
            .filter(|a| matches!(a.def, AddonDef::Poison { .. }))
            .count()
            .max(1);
        for k in 0..n {
            let a = self.projectiles[i].addons[k];
            let p = &self.projectiles[i];
            let (owner, fwd, damage) = (p.owner, p.forward(), p.damage);
            match a.def {
                AddonDef::Poison {
                    time,
                    interval,
                    sound,
                } => {
                    if let HitTarget::Player(pl) = target {
                        self.take_damage_over_time_cue(
                            content,
                            pl as usize,
                            fwd * (damage / poisons as f32),
                            time,
                            interval,
                            owner,
                            sound.unwrap_or(crate::events::cue::DAMAGE_POISON),
                        );
                    }
                }
                AddonDef::Bash(b) => {
                    if let HitTarget::Player(pl) = target {
                        self.bash(i, pl as usize, &b, a.scale);
                    }
                }
                AddonDef::EmpowerHit => {
                    if let AddonState::Done { done: false } = a.st {
                        self.projectiles[i].addons[k].st = AddonState::Done { done: true };
                        if let Some(o) = owner {
                            let o = o as usize;
                            let p = &self.players[o];
                            if !p.dead && !p.respawning {
                                let at = point - fwd * 0.05;
                                self.events.push(EventKind::EmpowerTriggered {
                                    player: o as u8,
                                    pos: at,
                                });
                                self.do_block(
                                    content,
                                    o,
                                    true,
                                    true,
                                    crate::events::BlockKind::Empower,
                                    Some(at),
                                    true,
                                );
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn bash(&mut self, i: usize, pl: usize, b: &BashDef, scale: f32) {
        let p = &self.projectiles[i];
        let (damage, distance, owner) = (p.damage, p.distance, p.owner);
        let chance = b.trigger_chance_per_ten_damage * damage * 0.1 + b.base_trigger_chance;
        let roll = self.rng.hit.next_f32();
        if roll >= chance {
            return;
        }
        let mut t = b.base_stun_time + b.stun_time_per_ten_damage * damage * 0.1;
        let mut mult = b.stun_multiplier;
        if b.multiplier_per_ten_meter != 0.0 {
            mult = distance * b.multiplier_per_ten_meter * 0.1;
        }
        t *= mult;
        t = pow(t, b.stun_time_exponent);
        t *= scale;
        if b.cannot_perma_stun
            && let Some(o) = owner
        {
            let g = &self.players[o as usize];
            t = clamp(
                t,
                0.0,
                g.gun.stats.attack_speed * g.stats.attack_speed_multiplier() + 0.3,
            );
        }
        if t > b.stun_time_threshold {
            self.add_stun(pl, t);
        }
    }

    /// Screen-edge bounce check (20.5); the add-on is present when
    /// `Projectile::screen_edge_bounce` is set.
    pub(crate) fn addons_after_sweep(&mut self, content: &Content, i: usize) {
        if self.projectiles[i].screen_edge_bounce {
            self.screen_edge(content, i);
        }
    }
}

/// Real seconds as a tick count for add-on timers.
pub fn real_ticks(s: f32) -> u32 {
    ticks(s).max(1)
}
