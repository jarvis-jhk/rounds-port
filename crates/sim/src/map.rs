//! The current map: load / slide-in timing (match-flow.md 7.1, maps.md 1),
//! animated groups with kill boxes (maps.md 5.2), saws (5.1), moving
//! platforms and self-righting (3.4), and the prop physics step.

use crate::content::{Content, MapId};
use crate::events::{EventKind, cue};
use crate::geom::Shape;
use crate::math::{V2, clamp, v2};
use crate::player::ForceMode;
use crate::world::{DT, Hasher, World, ticks};

/// Colliders become solid when the last child has arrived (max random
/// delay 0.25 s + slide 0.7 s).
pub const SOLID_AFTER: f32 = 0.95;
/// Map ready: 1 s after the first child arrived (0.7 s): ropes, bodies released.
pub const READY_AFTER: f32 = 1.7;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MapPhase {
    #[default]
    None,
    /// Sliding in; ticks since the call-in.
    Entering(u32),
    Active,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AnimState {
    pub delay: f32,
    pub t: f32,
    pub armed: u32,
    pub scale: f32,
}

#[derive(Clone, Debug, Default)]
pub struct MapState {
    /// The map in play (geometry, props).
    pub current: Option<MapId>,
    /// Loaded and waiting off-screen right, called in next.
    pub loaded: Option<MapId>,
    /// The map sliding out (presentation only).
    pub previous: Option<MapId>,
    pub phase: MapPhase,
    pub solid: bool,
    pub ready: bool,
    /// Seed for the presentation's per-child slide delays.
    pub seed: u32,
    /// Ticks since the current map was called in.
    pub since_call_in: u32,
    pub anims: Vec<AnimState>,
    pub anim_shapes: Vec<(Shape, u16)>,
    /// Damage-box cooldowns: (saw key, unused (0), real ticks left); one
    /// cooldown per saw, shared by all targets (maps.md 5.1).
    pub saw_cd: Vec<(u32, u32, u32)>,
}

impl MapState {
    pub(crate) fn hash_into(&self, h: &mut Hasher) {
        h.u32(self.current.map(|m| m.0 as u32 + 1).unwrap_or(0));
        h.bool(self.solid);
        h.bool(self.ready);
        h.u32(self.since_call_in);
        for a in &self.anims {
            h.f32(a.t);
            h.u32(a.armed);
        }
    }
}

impl World {
    /// `LoadNextLevel`: choose a map (uniform over the rotation, re-rolled
    /// while equal to the current one, match-flow.md 6) and place it off-screen.
    pub(crate) fn load_next_map(&mut self, content: &Content) {
        let rot = &self.rules.rotation;
        let mut next = rot[self.rng.maps.below(rot.len() as u32) as usize];
        if rot.len() > 1 {
            while Some(next) == self.map.current {
                next = rot[self.rng.maps.below(rot.len() as u32) as usize];
            }
        }
        let _ = content;
        self.load_map(next);
    }

    pub(crate) fn load_map(&mut self, id: MapId) {
        self.map.loaded = Some(id);
        self.map.seed = self.rng.cosmetic.next_u32();
        self.events.push(EventKind::MapLoaded {
            map: id,
            seed: self.map.seed,
        });
    }

    /// `CallInNewMap`: the old map slides out, the new one slides in; stray
    /// temporary objects are destroyed.
    pub(crate) fn call_in_map(&mut self) {
        let Some(id) = self.map.loaded.take() else {
            return;
        };
        self.map.previous = self.map.current;
        self.map.current = Some(id);
        self.map.phase = MapPhase::Entering(0);
        self.map.solid = false;
        self.map.ready = false;
        self.map.since_call_in = 0;
        self.map.anims.clear();
        self.map.anim_shapes.clear();
        self.map.saw_cd.clear();
        self.physics.clear();
        for p in &self.projectiles {
            self.events.list.push(crate::events::Event {
                frame: self.frame,
                seq: self.events.list.len() as u32,
                kind: EventKind::ProjectileRemoved { id: p.id },
            });
        }
        self.projectiles.clear();
        // Card effect objects stay with their players.
        for o in &self.objects {
            if matches!(o.kind, crate::objects::ObjKind::PlayerEffect { .. }) {
                continue;
            }
            self.events.list.push(crate::events::Event {
                frame: self.frame,
                seq: self.events.list.len() as u32,
                kind: EventKind::ObjectRemoved { id: o.id },
            });
        }
        self.objects
            .retain(|o| matches!(o.kind, crate::objects::ObjKind::PlayerEffect { .. }));
        self.events.sound(cue::LEVEL_OUT, V2::ZERO, 1.0);
        self.events.sound(cue::LEVEL_IN, V2::ZERO, 1.0);
        self.events.push(EventKind::MapCallIn { map: id });
    }

    pub(crate) fn map_tick(&mut self, content: &Content, game_dt: f32) {
        let Some(id) = self.map.current else { return };
        if let MapPhase::Entering(t) = self.map.phase {
            let t = t + 1;
            self.map.since_call_in = t;
            self.map.phase = MapPhase::Entering(t);
            if t >= ticks(SOLID_AFTER) && !self.map.solid {
                let def = content.map(id);
                self.map.solid = true;
                self.map.anims = def
                    .animated
                    .iter()
                    .map(|a| AnimState {
                        delay: a.start_delay,
                        t: 0.0,
                        armed: 0,
                        scale: 1.0,
                    })
                    .collect();
                self.physics.build(def, self.players.len());
                self.events.push(EventKind::MapEntered { map: id });
            }
            if t >= ticks(READY_AFTER) && !self.map.ready {
                self.physics.release(content.map(id));
                self.map.ready = true;
                self.map.phase = MapPhase::Active;
                self.events.push(EventKind::MapReady { map: id });
            }
        } else if self.map.phase == MapPhase::Active {
            self.map.since_call_in = self.map.since_call_in.saturating_add(1);
        }
        if !self.map.solid {
            return;
        }
        self.anims_tick(content, id, game_dt);
        self.saws_tick(content, id);
    }

    /// `CurveAnimation` groups (y scale) and their kill boxes (maps.md 5.2).
    fn anims_tick(&mut self, content: &Content, id: MapId, game_dt: f32) {
        let def = content.map(id);
        self.map.anim_shapes.clear();
        for (k, a) in def.animated.iter().enumerate() {
            let st = &mut self.map.anims[k];
            let prev_t = st.t;
            if st.delay > 0.0 {
                st.delay -= game_dt;
            } else {
                st.t += game_dt * a.speed;
            }
            let wrapped = st.t >= 1.0;
            if wrapped {
                st.t -= 1.0;
            }
            let crossed =
                |at: f32| (prev_t < at && st.t >= at) || (wrapped && (prev_t < at || st.t >= at));
            if let Some(kb) = a.kill_box
                && crossed(kb.activate_at)
            {
                st.armed = ticks(kb.toggle_time);
            }
            let (snd_move, snd_hit) = (crossed(0.15), crossed(0.3377));
            st.scale = a.curve.eval(st.t);
            let sy = st.scale;
            let (c, s) = (crate::math::cos(a.angle), crate::math::sin(a.angle));
            for col in &a.colliders {
                if let Shape::Obb { c: lc, axis, half } = &col.shape {
                    let wc = a.pivot + v2(lc.x, lc.y * sy).rotate(c, s);
                    self.map.anim_shapes.push((
                        Shape::Obb {
                            c: wc,
                            axis: axis.rotate(c, s),
                            half: v2(half.x, half.y * sy),
                        },
                        k as u16,
                    ));
                }
            }
            if snd_move {
                self.events.sound(cue::SMASHER_MOVE, a.pivot, 1.0);
                self.events.push(EventKind::SmasherSound {
                    index: k as u16,
                    impact: false,
                });
            }
            if snd_hit {
                self.events.sound(cue::SMASHER_IMPACT, a.pivot, 1.0);
                self.events.push(EventKind::SmasherSound {
                    index: k as u16,
                    impact: true,
                });
            }
            let st = &mut self.map.anims[k];
            if st.armed > 0 {
                st.armed -= 1;
                let kb = a.kill_box.unwrap();
                let wc = a.pivot + v2(kb.center.x, kb.center.y * sy).rotate(c, s);
                let shape = Shape::Obb {
                    c: wc,
                    axis: v2(c, s),
                    half: v2(kb.half.x, kb.half.y * sy),
                };
                for i in 0..self.players.len() {
                    let p = &self.players[i];
                    if p.hittable() && p.playing && shape.contains(p.pos) {
                        self.take_damage(content, i, V2::UP * kb.damage, wc, None, true, false);
                    }
                }
            }
        }
    }

    /// `DamageBox` on statics and props (maps.md 5.1).
    fn saws_tick(&mut self, content: &Content, id: MapId) {
        self.map.saw_cd.retain_mut(|e| {
            e.2 = e.2.saturating_sub(1);
            e.2 > 0
        });
        let prepared = content.prepared(id);
        let def = content.map(id);
        let mut saws: Vec<(u32, Shape, crate::content::DamageBoxDef)> = Vec::new();
        for (k, s) in prepared.statics.iter().enumerate() {
            if let Some(db) = s.damage_box {
                saws.push((k as u32, s.shape.clone(), db));
            }
        }
        for (k, p) in self.physics.props.iter().enumerate() {
            if !p.alive {
                continue;
            }
            if let Some(db) = def.props[p.def as usize].damage_box {
                for sh in &p.shapes {
                    saws.push((0x8000_0000 | k as u32, sh.clone(), db));
                }
            }
        }
        for (key, shape, db) in saws {
            let centre = shape.center();
            for i in 0..self.players.len() {
                let p = &self.players[i];
                if !p.hittable() || !p.playing {
                    continue;
                }
                let (cp, inside) = shape.closest_surface_point(p.pos);
                let r = p.radius() + 0.05;
                if !inside && (cp - p.pos).len_sq() > r * r {
                    continue;
                }
                // One cooldown per saw for every target (maps.md 5.1).
                if self.map.saw_cd.iter().any(|e| e.0 == key) {
                    continue;
                }
                self.map.saw_cd.push((key, 0, ticks(db.cd)));
                let d = (p.pos - centre).normalized();
                let d = if d.is_zero() { V2::UP } else { d };
                self.take_force(
                    i,
                    d * db.force,
                    ForceMode::Impulse,
                    false,
                    true,
                    db.flying_for,
                );
                let pos = self.players[i].pos;
                self.call_take_damage(content, i, d * db.damage, pos, None, true);
                self.events.sound(cue::SAW_DAMAGE, centre, 1.0);
                self.events.push(EventKind::SawHit {
                    pos: centre,
                    dir: d,
                });
                self.events.shake(d * db.shake);
            }
            // Destructible props touching the saw.
            for pi in 0..self.physics.props.len() {
                let pr = &self.physics.props[pi];
                if !pr.alive || pr.hp.is_none() || key == (0x8000_0000 | pi as u32) {
                    continue;
                }
                let touching = pr.shapes.iter().any(|s| {
                    let (cp, inside) = s.closest_surface_point(centre);
                    let rr = match &shape {
                        Shape::Circle { r, .. } => *r + 0.05,
                        _ => 0.05,
                    };
                    inside || (cp - centre).len_sq() < rr * rr
                });
                if !touching {
                    continue;
                }
                if self.map.saw_cd.iter().any(|e| e.0 == key) {
                    continue;
                }
                self.map.saw_cd.push((key, 0, ticks(db.cd)));
                let d = (pr.pos - centre).normalized();
                self.damage_prop(content, pi, d * db.damage);
            }
        }
    }

    /// Prop scripts and the physics step (full fixed dt).
    pub(crate) fn props_tick(&mut self, content: &Content, game_dt: f32) {
        if !self.physics.is_active() {
            return;
        }
        let Some(id) = self.map.current else { return };
        let def = content.map(id);
        let capped = clamp(game_dt, 0.0, 0.02);
        for pi in 0..self.physics.props.len() {
            let p = &self.physics.props[pi];
            if !p.alive || !p.released {
                continue;
            }
            let pd = &def.props[p.def as usize];
            if let Some(ms) = &pd.move_sequence
                && !ms.positions.is_empty()
            {
                let idx = (p.move_index as usize) % ms.positions.len();
                let target = ms.positions[idx] + p.start_pos;
                let d = (target - p.pos).clamp_len(ms.cap);
                let f = d * (ms.spring * capped * p.mass);
                let near = (p.pos - target).len() < ms.threshold;
                self.physics.add_force(pi, f);
                let p = &mut self.physics.props[pi];
                if near {
                    p.move_timer += game_dt;
                    if p.move_timer > ms.time_at_pos {
                        p.move_index = ((idx + 1) % ms.positions.len()) as u16;
                        p.move_timer = 0.0;
                    }
                }
            }
            if let Some(force) = pd.rig_look_up {
                let p = &self.physics.props[pi];
                let up = V2::UP.rotate_angle(p.angle);
                let ang = up.angle_deg(V2::UP);
                if ang > 1e-4 {
                    let sign = if up.x > 0.0 { 1.0 } else { -1.0 };
                    let t = sign * force * p.mass * ang;
                    self.physics.add_torque(pi, t);
                }
            }
        }
        for i in 0..self.players.len() {
            let p = &self.players[i];
            let pos = if p.hittable() && p.simulated {
                Some(p.pos)
            } else {
                None
            };
            self.physics.set_player(i, pos, p.scale);
        }
        let _ = DT;
        self.physics.step(game_dt);
    }
}
