//! The player's gun: magazine and reload (weapons.md 4), trigger gating (5),
//! the shooting sequence with bursts (6), spread (7) and projectile
//! initialisation (8).

use crate::content::{Content, SpawnEntry};
use crate::effects::Hook;
use crate::events::{EventKind, cue};
use crate::math::{V2, clamp, lerp_unclamped, pow};
use crate::projectile::Projectile;
use crate::stats::{AmmoStats, GunStats};
use crate::world::{Hasher, World, ticks};

/// A burst scheduled `timeBetweenBullets` real seconds after the previous.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PendingBurst {
    pub remaining: i32,
    pub ticks: u32,
    pub interval: u32,
    pub damage_m: f32,
    pub use_ammo: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Gun {
    pub stats: GunStats,
    pub ammo_stats: AmmoStats,
    /// Merged `objectsToSpawn` (cards.md 4.2).
    pub spawns: std::sync::Arc<[SpawnEntry]>,
    /// Index into `Content::projectiles`.
    pub projectile: u16,
    pub since_attack: f32,
    pub attack_speed_multiplier: f32,
    pub ammo: i32,
    pub reloading: bool,
    pub reload_counter: f32,
    pub free_reload_counter: f32,
    pub reg_counter: f32,
    last_max: i32,
    pub bursts: Vec<PendingBurst>,
    /// `forceShootDir` (radar shot), zero = unused.
    pub force_shoot_dir: V2,
    cant_shoot_latch: bool,
}

impl Default for Gun {
    fn default() -> Self {
        Gun::new()
    }
}

impl Gun {
    pub fn new() -> Gun {
        let ammo_stats = AmmoStats::default();
        Gun {
            stats: GunStats::player_reset(),
            ammo_stats,
            spawns: std::sync::Arc::from(Vec::new()),
            projectile: 0,
            since_attack: 10.0,
            attack_speed_multiplier: 1.0,
            ammo: ammo_stats.max_ammo,
            reloading: false,
            reload_counter: 0.0,
            free_reload_counter: 0.0,
            reg_counter: 0.0,
            last_max: ammo_stats.max_ammo,
            bursts: Vec::new(),
            force_shoot_dir: V2::ZERO,
            cant_shoot_latch: false,
        }
    }

    pub fn cooldown(&self) -> f32 {
        if self.stats.lock_gun_to_default {
            self.stats.default_cooldown
        } else {
            self.stats.attack_speed
        }
    }

    pub fn is_ready(&self) -> bool {
        self.since_attack > self.cooldown()
    }

    /// Silent full magazine (revive).
    pub fn reload_silent(&mut self) {
        self.ammo = self.ammo_stats.max_ammo;
        self.last_max = self.ammo;
        self.reloading = false;
        self.reload_counter = 0.0;
        self.free_reload_counter = 0.0;
        self.reg_counter = 0.0;
        self.bursts.clear();
    }

    pub(crate) fn hash_into(&self, h: &mut Hasher) {
        h.f32(self.since_attack);
        h.u32(self.ammo as u32);
        h.bool(self.reloading);
        h.f32(self.reload_counter);
        h.f32(self.free_reload_counter);
        h.f32(self.stats.damage);
        h.f32(self.stats.attack_speed);
        h.u32(self.bursts.len() as u32);
    }
}

impl World {
    /// Weapon handler + ammo update, frame phase (player.md 13; weapons.md 4-5).
    pub(crate) fn gun_frame(&mut self, content: &Content, i: usize, dtu: f32, battle: bool) {
        // Cooldown clock: `Gun.Update` runs whether or not the player is
        // simulated (weapons.md 5, Clarified).
        {
            let g = &mut self.players[i].gun;
            g.since_attack += dtu * g.attack_speed_multiplier;
        }
        if !self.players[i].simulated {
            return;
        }
        self.ammo_tick(content, i, dtu);
        // Bursts in flight (real time).
        let mut k = 0;
        while k < self.players[i].gun.bursts.len() {
            let b = &mut self.players[i].gun.bursts[k];
            b.ticks = b.ticks.saturating_sub(1);
            if b.ticks == 0 {
                let b = *b;
                self.fire_one_burst(content, i, b.damage_m, b.use_ammo, b.remaining == 1);
                let bb = &mut self.players[i].gun.bursts[k];
                bb.remaining -= 1;
                if bb.remaining <= 0 {
                    self.players[i].gun.bursts.remove(k);
                    continue;
                }
                bb.ticks = bb.interval;
            }
            k += 1;
        }
        // Trigger gating.
        let p = &self.players[i];
        if !battle || !p.playing || p.dead {
            return;
        }
        let c = p.controls;
        let g = &p.gun;
        let auto = g.stats.attack_speed / g.attack_speed_multiplier < 0.3
            && c.shoot_held
            && !g.stats.use_charge
            && !g.stats.dont_allow_auto_fire;
        let trigger = (c.shoot_pressed && !g.stats.use_charge)
            || (c.shoot_released && g.stats.use_charge)
            || auto;
        if !c.shoot_held {
            self.players[i].gun.cant_shoot_latch = false;
        }
        if !trigger {
            return;
        }
        let g = &self.players[i].gun;
        if g.reloading {
            if c.shoot_pressed && !g.cant_shoot_latch {
                self.players[i].gun.cant_shoot_latch = true;
                let pos = self.players[i].pos;
                self.events.sound(cue::CANT_SHOOT, pos, 1.0);
                self.events.push(EventKind::CantShoot { player: i as u8 });
            }
            return;
        }
        if !g.is_ready() {
            return;
        }
        self.gun_attack(content, i, false, 1.0, true);
    }

    fn ammo_tick(&mut self, content: &Content, i: usize, dt: f32) {
        let auto = self.players[i].stats.automatic_reload;
        let g = &mut self.players[i].gun;
        let max = g.ammo_stats.max_ammo;
        if g.reloading {
            g.reload_counter -= dt;
            if g.reload_counter < 0.0 {
                self.reload_ammo(content, i);
            }
        } else if g.ammo != max {
            g.free_reload_counter += dt;
            if g.free_reload_counter > g.ammo_stats.reload_duration() && auto {
                g.ammo = max;
            }
            g.reg_counter += g.ammo_stats.ammo_reg * dt * max as f32;
            if g.reg_counter > 1.0 {
                g.ammo += 1;
                g.reg_counter = 0.0;
            }
        }
        let g = &mut self.players[i].gun;
        if g.ammo_stats.max_ammo != g.last_max {
            g.last_max = g.ammo_stats.max_ammo;
            g.ammo = g.ammo_stats.max_ammo;
        }
    }

    /// `ReloadAmmo` (weapons.md 4).
    pub fn reload_ammo(&mut self, content: &Content, i: usize) {
        let g = &mut self.players[i].gun;
        let rounds = g.ammo_stats.max_ammo - g.ammo;
        g.reloading = false;
        g.ammo = g.ammo_stats.max_ammo;
        let pos = self.players[i].pos;
        self.fire_hook(content, i, Hook::ReloadDone { rounds });
        self.events.sound(cue::RELOAD_COMPLETE, pos, 1.0);
        self.events.push(EventKind::ReloadDone {
            player: i as u8,
            rounds,
        });
    }

    /// `GunAmmo.Shoot` (once per projectile).
    fn ammo_shoot(&mut self, content: &Content, i: usize) {
        let g = &mut self.players[i].gun;
        g.ammo -= 1;
        g.free_reload_counter = 0.0;
        if g.ammo <= 0 {
            g.reload_counter = g.ammo_stats.reload_duration();
            g.reloading = true;
            let (max, dur) = (g.ammo_stats.max_ammo, g.reload_counter);
            self.fire_hook(content, i, Hook::OutOfAmmo { max_ammo: max });
            self.events.push(EventKind::OutOfAmmo { player: i as u8 });
            self.events.push(EventKind::ReloadStarted {
                player: i as u8,
                duration: dur,
            });
        }
    }

    /// `Gun.Attack` (weapons.md 6.1) with `charge = 0`.
    pub fn gun_attack(
        &mut self,
        content: &Content,
        i: usize,
        force: bool,
        damage_m: f32,
        use_ammo: bool,
    ) -> bool {
        let g = &self.players[i].gun;
        if !force && (g.since_attack < g.cooldown() || g.reloading) {
            return false;
        }
        self.players[i].gun.since_attack = 0.0;
        // DoAttack: visual recoil on the held gun (player.md 4.4).
        {
            let p = &mut self.players[i];
            let kick = p.gun.stats.recoil * clamp(p.gun.cooldown(), 0.0, 1.0);
            let dir = p.aim.normalized();
            p.gun_vel -= dir * kick;
        }
        self.fire_hook(content, i, Hook::Attack);
        let g = &self.players[i].gun;
        let bursts = g.stats.bursts.clamp(1, 100);
        let tbb = g.stats.time_between_bullets;
        self.fire_one_burst(content, i, damage_m, use_ammo, bursts == 1);
        if bursts > 1 {
            let interval = ticks(tbb).max(if tbb > 0.0 { 1 } else { 0 });
            if interval == 0 {
                for b in 1..bursts {
                    self.fire_one_burst(content, i, damage_m, use_ammo, b == bursts - 1);
                }
            } else {
                self.players[i].gun.bursts.push(PendingBurst {
                    remaining: bursts - 1,
                    ticks: interval,
                    interval,
                    damage_m,
                    use_ammo,
                });
            }
        }
        self.players[i].gun.force_shoot_dir = V2::ZERO;
        true
    }

    /// One burst of `numberOfProjectiles` (weapons.md 6.3).
    fn fire_one_burst(
        &mut self,
        content: &Content,
        i: usize,
        damage_m: f32,
        use_ammo: bool,
        _last: bool,
    ) {
        let p = &self.players[i];
        if p.dead || p.respawning {
            return;
        }
        let n = if p.gun.stats.lock_gun_to_default {
            1
        } else {
            p.gun.stats.number_of_projectiles.max(0)
        };
        let (pos, aim) = (p.pos, p.aim);
        self.events.push(EventKind::ShotFired {
            player: i as u8,
            projectiles: n as u16,
            pos,
            dir: aim.normalized(),
        });
        for _ in 0..n {
            self.spawn_player_projectile(content, i, damage_m, use_ammo, n);
        }
    }

    /// Spawning one projectile of the player's gun (weapons.md 6.4): the
    /// shared spawn, then the gun's per-projectile callbacks and the ammo.
    fn spawn_player_projectile(
        &mut self,
        content: &Content,
        i: usize,
        damage_m: f32,
        use_ammo: bool,
        volley: i32,
    ) {
        let p = &self.players[i];
        let f = if p.gun.force_shoot_dir.is_zero() {
            p.aim.normalized()
        } else {
            p.gun.force_shoot_dir
        };
        let (stats, spawns, proj, pos) =
            (p.gun.stats, p.gun.spawns.clone(), p.gun.projectile, p.pos);
        let id =
            self.spawn_gun_projectile(content, i, &stats, &spawns, proj, pos, f, damage_m, volley);
        self.fire_hook(content, i, Hook::ShootProjectile { projectile: id });
        if use_ammo {
            self.ammo_shoot(content, i);
        }
    }

    /// Spawn one projectile of a gun (player or effect gun; weapons.md 6.4,
    /// 8, 21.1): RNG draws #1 and #2, initialisation, add-ons. `f` is the
    /// (unnormalised for forced shots) aim direction. Returns the id.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn spawn_gun_projectile(
        &mut self,
        content: &Content,
        owner: usize,
        g: &GunStats,
        spawns: &std::sync::Arc<[SpawnEntry]>,
        projectile: u16,
        pos: V2,
        f: V2,
        damage_m: f32,
        volley: i32,
    ) -> u32 {
        // RNG draw #1 and #2 (always consumed).
        let r0 = self.rng.gun.range(-g.spread, g.spread);
        let s = self.rng.gun.next_f32();
        let dir = if g.lock_gun_to_default {
            f.normalized()
        } else {
            let r = r0 / ((1.0 + g.projectile_speed * 0.5) * 0.5);
            (f + f.perp_right() * (r * g.multiply_spread)).normalized()
        };
        let def = &content.projectiles[(projectile as usize).min(content.projectiles.len() - 1)];
        let id = self.next_id;
        let team = self.players[owner].team;
        let mut pr = Projectile::from_gun(
            id,
            def,
            g,
            spawns,
            owner as u8,
            team,
            pos,
            dir,
            damage_m,
            s,
            volley,
        );
        pr.addons = crate::script::addons::build_addons(content, spawns, pr.damage);
        self.alloc_id();
        let (ppos, pvel, psize) = (pr.pos, pr.vel, pr.size);
        self.projectiles.push(pr);
        self.events.push(EventKind::ProjectileSpawned {
            id,
            player: Some(owner as u8),
            pos: ppos,
            vel: pvel,
            size: psize,
        });
        id
    }
}

impl Projectile {
    /// Projectile initialisation from a gun (weapons.md 8).
    #[allow(clippy::too_many_arguments)]
    pub fn from_gun(
        id: u32,
        def: &crate::content::ProjectileDef,
        g: &GunStats,
        spawns: &std::sync::Arc<[SpawnEntry]>,
        owner: u8,
        team: u8,
        pos: V2,
        dir: V2,
        damage_m: f32,
        seed: f32,
        _volley: i32,
    ) -> Projectile {
        let mut damage = def.damage * g.damage * damage_m;
        let deal_mult = def.deal_damage_multiplier * g.bullet_damage_multiplier;
        let mut force = def.force * g.knockback;
        let vs = clamp(g.spread * 50.0, 0.0, 50.0);
        let speed = def.speed
            * g.projectile_speed
            * lerp_unclamped(1.0 - vs / 100.0, 1.0 + vs / 100.0, seed);
        let world_force = def.world_force * g.gravity;
        // Start-up (local scale 1).
        damage *= 1.0;
        force *= pow(damage / 55.0, 2.0);
        let size = clamp(pow(damage, 0.85) / 400.0, 0.0, 100.0) + 0.3 + g.size;
        let k = (damage / 55.0 + 1.0) * 0.5;
        Projectile {
            id,
            owner: Some(owner),
            team,
            pos,
            last_pos: pos,
            vel: dir * speed + world_force,
            start_speed: speed,
            gravity: def.gravity * g.gravity,
            drag: clamp(g.drag, 0.0, 45.0),
            drag_min_speed: g.drag_min_speed,
            sim_speed: g.projectile_simulation_speed,
            multiplier: 1.0,
            simulate_gravity: 0,
            distance: 0.0,
            damage,
            force,
            deal_mult,
            percentage_damage: g.percentage_damage,
            slow: g.slow,
            unblockable: g.unblockable,
            wall_mask: if g.ignore_walls {
                def.ignore_walls_mask
            } else {
                def.wall_mask
            },
            size,
            hold_player_for: def.hold_player_for,
            age_real: 0.0,
            lifetime: if g.destroy_bullet_after != 0.0 {
                g.destroy_bullet_after
            } else {
                def.lifetime
            },
            held: {
                let mut h = crate::projectile::HoldList::default();
                h.hold(owner, ticks(def.hold_player_for));
                h
            },
            reflects: g.reflects.max(0),
            speed_m_on_bounce: g.speed_m_on_bounce,
            dmg_m_on_bounce: g.dmg_m_on_bounce,
            since_bounce_real: 10.0,
            since_reflect: 10.0,
            spawns: spawns.clone(),
            shake: def.shake,
            can_push_box: def.can_push_box,
            destroy_on_block: def.destroy_on_block,
            collision: def
                .collision_radius
                .map(|r| crate::projectile::BulletCircle {
                    hp: damage,
                    threshold: 0.1 * damage,
                    radius: r * k,
                    collided: false,
                }),
            fresh: true,
            damage_after_distance: if g.damage_after_distance_multiplier != 1.0 {
                Some(g.damage_after_distance_multiplier)
            } else {
                None
            },
            cos: g.cos,
            screen_edge_bounce: false,
            last_edge: 0,
            immunity_key: if def.bullet_immunity.is_empty() {
                None
            } else {
                Some(crate::projectile::name_key(&def.bullet_immunity))
            },
            can_deal_damage: true,
            allowed_to_spawn: true,
            dead: false,
            addons: Vec::new(),
            paused: false,
        }
    }
}
