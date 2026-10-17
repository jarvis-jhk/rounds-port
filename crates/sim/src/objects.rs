//! Spawned objects (hit-spawned, block-spawned, effect objects) and the
//! generic `Explosion` (weapons.md 18-19). Prefab behaviour components run
//! in `crate::script` (cards.md 5.2, 6).

use crate::content::{Content, ExplosionDef, ObjectId, SpawnEntry};
use crate::events::{EventKind, HitTarget, cue};
use crate::math::{V2, clamp, v2};
use crate::player::ForceMode;
use crate::world::{Hasher, World};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Attach {
    None,
    /// Follows a player with a fixed offset.
    Player(u8, V2),
    /// Follows a prop with an offset in its local frame.
    Prop(u32, V2),
}

/// Sim lifetime of purely visual spawned prefabs that have no
/// `RemoveAfterSeconds` (game seconds).
pub const VISUAL_LIFETIME: f32 = 2.0;

/// What an object is to the game.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjKind {
    /// Spawned into the world (hit, block, effect spawns).
    Spawned,
    /// A card's effect object added to a player (`AddObjectToPlayer`): it
    /// sits on the player, survives deaths and rounds (inactive while the
    /// player is dead) and receives the player's hooks.
    PlayerEffect { owner: u8 },
}

/// Companion components of an `Explosion` on the same object (weapons.md 19.4).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ExplodeHooks {
    /// `Explosion_Overpower.dmgPer100Hp`.
    pub overpower: Option<f32>,
    /// `Implosion` (force, clampDist pre-scaled; drag, time).
    pub implosion: Option<(f32, f32, f32, f32)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpawnedObject {
    pub id: u32,
    pub def: ObjectId,
    pub pos: V2,
    pub dir: V2,
    pub scale: f32,
    /// `SpawnedAttack.spawner`.
    pub spawner: Option<u8>,
    pub level: i32,
    /// Game seconds alive.
    pub age: f32,
    /// Remaining life in game seconds (`RemoveAfterSeconds`), None = forever.
    pub life: Option<f32>,
    /// `RemoveAfterSeconds.scaleWithScale`: life runs at `1 / scale`.
    pub life_scale: bool,
    pub attach: Attach,
    pub kind: ObjKind,
    /// Component runtime state (None for prefabs without behaviour).
    pub st: Option<Box<crate::script::ObjState>>,
    /// Destroyed; removed at the end of the object pass.
    pub dead: bool,
}

impl SpawnedObject {
    pub(crate) fn hash_into(&self, h: &mut Hasher) {
        h.u32(self.id);
        h.u32(self.def.0 as u32);
        h.v2(self.pos);
        h.f32(self.scale);
        h.f32(self.age);
        h.bool(self.dead);
        if let Some(st) = &self.st {
            h.u64(st.active);
            h.u64(st.enabled);
            h.u32(st.comps.len() as u32);
        }
    }
}

impl World {
    /// Spawn a prefab instance and run its components' `Start` (an `auto`
    /// explosion explodes immediately). Returns the object id.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_object(
        &mut self,
        content: &Content,
        def: ObjectId,
        pos: V2,
        dir: V2,
        scale_mul: f32,
        spawner: Option<u8>,
        level: i32,
        attach: Option<Attach>,
    ) -> u32 {
        let pf = content.prefab(def);
        let id = self.alloc_id();
        let scale = pf.nodes[0].scale * scale_mul;
        let obj = SpawnedObject {
            id,
            def,
            pos,
            dir,
            scale,
            spawner,
            level,
            age: 0.0,
            life: None,
            life_scale: false,
            attach: attach.unwrap_or(Attach::None),
            kind: ObjKind::Spawned,
            st: crate::script::run::new_state(pf),
            dead: false,
        };
        self.events.push(EventKind::ObjectSpawned {
            id,
            def,
            pos,
            dir,
            scale,
        });
        self.objects.push(obj);
        let oi = self.objects.len() - 1;
        self.object_start(content, oi);
        // PORT DECISION (cards.md 5.2): a purely visual prefab without a
        // lifetime (its particles end it in Unity, e.g. VE_Radar) is kept for
        // VISUAL_LIFETIME game seconds so the presentation can draw it.
        if pf.trivial && self.objects[oi].life.is_none() {
            self.objects[oi].life = Some(VISUAL_LIFETIME);
        }
        id
    }

    /// Hit-spawned objects of one `ObjectsToSpawn` entry (weapons.md 18.1).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn spawn_hit_objects(
        &mut self,
        content: &Content,
        e: &SpawnEntry,
        point: V2,
        normal: V2,
        fwd: V2,
        damage: f32,
        target: HitTarget,
        owner: Option<u8>,
        blocked: bool,
    ) {
        let Some(effect) = e.effect else { return };
        if blocked && e.stick_to_all_targets {
            return;
        }
        let is_player = matches!(target, HitTarget::Player(_));
        let ok = match e.spawn_on {
            0 => true,
            1 => is_player,
            _ => !is_player,
        };
        if !ok {
            return;
        }
        let mut scale = 1.0;
        if e.scale_from_damage != 0.0 {
            scale *= (1.0 - e.scale_from_damage) + damage / 55.0 * e.scale_from_damage;
        }
        if e.scale_stacks {
            scale *= 1.0 + e.stacks as f32 * e.scale_stack_m;
        }
        for _ in 0..e.number_of_spawns.max(0) {
            let pos = point + normal * e.normal_offset;
            let dir = match e.direction {
                0 => fwd,
                1 => (normal + v2(0.0001, 0.0)).normalized(),
                _ => V2::RIGHT,
            };
            let big_static = match target {
                HitTarget::Map => true,
                HitTarget::Prop(pp) => self
                    .physics
                    .props
                    .get(pp as usize)
                    .is_some_and(|p| p.mass > 500.0),
                _ => false,
            };
            let stick = (e.stick_to_big_targets && !is_player && big_static)
                || e.stick_to_all_targets
                || e.spawn_as_child;
            let attach = if stick {
                match target {
                    HitTarget::Player(pl) => {
                        let pp = self.players[pl as usize].pos;
                        let side = (pos - pp).normalized();
                        Some(Attach::Player(pl, side * 1.1))
                    }
                    HitTarget::Prop(pr) => {
                        let p = &self.physics.props[pr as usize];
                        let (c, s) = (crate::math::cos(-p.angle), crate::math::sin(-p.angle));
                        Some(Attach::Prop(pr, (pos - p.pos).rotate(c, s)))
                    }
                    _ => None,
                }
            } else {
                None
            };
            let pos = match attach {
                Some(Attach::Player(pl, off)) => self.players[pl as usize].pos + off,
                _ => pos,
            };
            self.spawn_object(content, effect, pos, dir, scale, owner, 1, attach);
        }
    }

    pub(crate) fn objects_tick(&mut self, content: &Content, game_dt: f32) {
        // Objects spawned during this pass get their first update next tick.
        let n = self.objects.len();
        for k in 0..n {
            if self.objects[k].dead {
                continue;
            }
            let o = &mut self.objects[k];
            o.age += game_dt;
            let mut remove = false;
            if let Some(l) = o.life.as_mut() {
                *l -= if o.life_scale {
                    game_dt / o.scale.max(1e-6)
                } else {
                    game_dt
                };
                if *l < 0.0 {
                    remove = true;
                }
            }
            match o.attach {
                Attach::None => {}
                Attach::Player(pl, off) => {
                    let p = &self.players[pl as usize];
                    if p.dead || p.respawning {
                        remove = true;
                    } else {
                        o.pos = p.pos + off;
                    }
                }
                Attach::Prop(pr, local) => match self.physics.props.get(pr as usize) {
                    Some(p) if p.alive => {
                        let (c, s) = (crate::math::cos(p.angle), crate::math::sin(p.angle));
                        o.pos = p.pos + local.rotate(c, s);
                    }
                    _ => remove = true,
                },
            }
            if remove {
                self.destroy_object(content, k);
                continue;
            }
            self.object_update(content, k, game_dt);
        }
        self.sweep_objects();
    }

    /// Remove destroyed objects (and emit their removal events).
    pub(crate) fn sweep_objects(&mut self) {
        if !self.objects.iter().any(|o| o.dead) {
            return;
        }
        for o in self.objects.iter().filter(|o| o.dead) {
            let seq = self.events.list.len() as u32;
            self.events.list.push(crate::events::Event {
                frame: self.frame,
                seq,
                kind: EventKind::ObjectRemoved { id: o.id },
            });
        }
        self.objects.retain(|o| !o.dead);
    }

    /// `Explosion.Explode` (weapons.md 19).
    #[allow(clippy::too_many_arguments)]
    pub fn explode(
        &mut self,
        content: &Content,
        centre: V2,
        e: &ExplosionDef,
        s: f32,
        spawner: Option<u8>,
        def: Option<ObjectId>,
        name: u32,
    ) {
        self.explode_with(
            content,
            centre,
            e,
            s,
            spawner,
            def,
            name,
            ExplodeHooks::default(),
        );
    }

    /// `Explode` with the companion hooks (Overpower, Implosion).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn explode_with(
        &mut self,
        content: &Content,
        centre: V2,
        e: &ExplosionDef,
        s: f32,
        spawner: Option<u8>,
        def: Option<ObjectId>,
        name: u32,
        hooks: ExplodeHooks,
    ) {
        let r = e.range * if e.scale_radius { s } else { 1.0 };
        self.events.push(EventKind::Explosion {
            pos: centre,
            radius: r,
            def,
        });
        let falloff = |dist: f32| {
            if e.static_range_multiplier {
                1.0
            } else {
                clamp(1.0 - dist / r, 0.0, 1.0)
            }
        };
        let sd = if e.scale_dmg { s } else { 1.0 };
        let sf = if e.scale_force { s } else { 1.0 };
        let sslow = if e.scale_slow { s } else { 1.0 };
        let ssil = if e.scale_silence { s } else { 1.0 };
        let sstun = if e.scale_stun { (1.0 + s) * 0.5 } else { 1.0 };
        let unit = |v: V2| {
            let n = v.normalized();
            if n.is_zero() { V2::UP } else { n }
        };
        // Props.
        for pi in 0..self.physics.props.len() {
            let p = &self.physics.props[pi];
            if !p.alive {
                continue;
            }
            // Candidates by their real shapes (OverlapCircleAll); the
            // falloff distance uses the bounds (weapons.md 19.1).
            if !p
                .shapes
                .iter()
                .any(|sh| shape_overlaps_circle(sh, centre, r))
            {
                continue;
            }
            let cp = p.aabb.closest_point(centre);
            let dist = (cp - centre).len();
            let f = falloff(dist);
            let ppos = p.pos;
            // `NetworkPhysicsObject.BulletPush` multiplies by the prop's
            // `bulletPushMultiplier` (weapons.md 19.1, Clarified; 1 for
            // hand-built props without the data).
            let push = self
                .map
                .current
                .and_then(|m| content.map(m).props.get(p.def as usize))
                .and_then(|d| d.pushable)
                .map_or(1.0, |d| d.bullet_push_multiplier);
            let j = unit(ppos - centre) * (e.object_force_multiplier * f * e.force * sf * push);
            self.physics.impulse(pi, j);
            let u = unit(cp - centre);
            if e.damage > 0.0 && self.physics.props[pi].hp.is_some() {
                self.damage_prop(content, pi, u * (sd * e.damage * f));
            }
            self.physics.impulse(pi, u * (f * e.force * sf));
        }
        // Players.
        for i in 0..self.players.len() {
            let p = &self.players[i];
            if !p.hittable() || !p.playing {
                continue;
            }
            if e.ignore_team && spawner == Some(i as u8) {
                continue;
            }
            let rad = p.radius();
            if (p.pos - centre).len() > r + rad {
                continue;
            }
            let cp = crate::geom::Aabb::around(p.pos, rad).closest_point(centre);
            let dist = (cp - centre).len();
            let f = falloff(dist);
            if e.immunity > 0.0 && self.is_immune(i, e.immunity, sd * e.damage * f, name) {
                continue;
            }
            if !e.ignore_walls {
                let pos = self.players[i].pos;
                if !self.scene(content, &[]).line_of_sight_masked(
                    centre,
                    pos,
                    content.tuning.line_of_sight_mask,
                ) {
                    continue;
                }
            }
            let blocking = self.players[i].is_blocking();
            if e.slow != 0.0 && !blocking {
                self.add_slow(i, e.slow * f * sslow, e.fast_slow);
            }
            if e.silence != 0.0 && !blocking {
                self.add_silence(i, e.silence * f * ssil);
            }
            self.explosion_player_hooks(content, i, centre, s, spawner, &hooks, 0);
            let u = unit(cp - centre);
            if e.damage < 0.0 {
                self.heal(i, -e.damage);
            } else if e.damage > 0.0 {
                self.take_damage(
                    content,
                    i,
                    u * (sd * e.damage * f),
                    centre,
                    spawner,
                    true,
                    false,
                );
            }
            self.explosion_player_hooks(content, i, centre, s, spawner, &hooks, 1);
            if e.force != 0.0 {
                let imp = u * (f * e.force * sf);
                let fly = if e.locally_simulated {
                    0.0
                } else {
                    e.flying_for * f
                };
                self.take_force(i, imp, ForceMode::Impulse, e.force_ignore_mass, false, fly);
            }
            if e.stun > 0.0 {
                self.add_stun(i, e.stun * sstun);
            }
        }
    }

    /// Damage a destructible prop (maps.md 5.3).
    pub fn damage_prop(&mut self, content: &Content, pi: usize, dmg: V2) {
        let Some(mid) = self.map.current else { return };
        let p = &mut self.physics.props[pi];
        let Some(hp) = p.hp.as_mut() else { return };
        if !p.alive {
            return;
        }
        *hp -= dmg.len();
        if *hp > 0.0 {
            self.events.push(EventKind::PropDamaged { prop: pi as u32 });
            return;
        }
        let pos = p.pos;
        let rng = &mut self.rng.misc;
        self.physics
            .destroy(content.map(mid), pi, dmg, || rng.next_f32());
        self.events.sound(cue::BOX_DESTRUCTION, pos, 1.0);
        self.events.push(EventKind::PropDestroyed {
            prop: pi as u32,
            pos,
        });
    }
}

/// Whether `shape` overlaps the circle (`centre`, `r`) (`OverlapCircle`).
pub(crate) fn shape_overlaps_circle(shape: &crate::geom::Shape, centre: V2, r: f32) -> bool {
    let (q, inside) = shape.closest_surface_point(centre);
    inside || (q - centre).len_sq() <= r * r
}
