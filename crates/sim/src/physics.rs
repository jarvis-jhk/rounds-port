//! Physics props on rapier2d (maps.md 3-5; docs/sim-design.md "Physics").
//!
//! The rapier sets live inside the world and are cloned for rollback. Props
//! step at the full fixed step 0.014 s regardless of the gameplay time scale
//! (match-flow.md 8 recommendation). Players exist in rapier only as
//! kinematic `PlayerObjectCollider` circles (radius 0.15 * scale) so props
//! are pushed by them; their gameplay collision is the custom push-out.
//!
//! After each step the props' world shapes are cached (`Prop::shapes`) and
//! all gameplay queries use those through geom.rs.

use crate::content::{BodyKind, ColliderDef, JointKind, MapDef, RopeKind};
use crate::geom::{Aabb, Shape, layers};
use crate::math::{V2, v2};
use rapier2d::prelude::*;

/// Unity's 2D physics gravity (maps.md 2.1).
pub const GRAVITY: f32 = -50.0;
pub const PHYS_DT: f32 = 0.014;

/// Physics layer matrix rows we need (maps.md 2.1, player.md 1.3).
fn layer_filter(layer: u8) -> u32 {
    use layers::*;
    let b = bit;
    match layer {
        IGNORE_PLAYER => {
            b(DEFAULT)
                | b(IGNORE_PLAYER)
                | b(CORPSE)
                | b(PLAYER_OBJECT_COLLIDER)
                | b(IGNORE_MAP)
                | b(BACKGROUND_OBJECT)
        }
        BACKGROUND_OBJECT => b(DEFAULT) | b(IGNORE_PLAYER) | b(BACKGROUND_OBJECT),
        PLAYER_OBJECT_COLLIDER => !(b(BACKGROUND_OBJECT)),
        PLAYER => !(b(CORPSE) | b(PROJECTILE) | b(IGNORE_PLAYER) | b(BACKGROUND_OBJECT)),
        PROJECTILE => b(UI_TOUCH) | b(PLAYER_OBJECT_COLLIDER) | b(SPAWN_POINT) | b(IGNORE_MAP),
        _ => u32::MAX,
    }
}

fn groups(layer: u8) -> InteractionGroups {
    InteractionGroups::new(
        Group::from_bits_truncate(layers::bit(layer)),
        Group::from_bits_truncate(layer_filter(layer)),
        InteractionTestMode::And,
    )
}

fn vec(v: V2) -> Vector {
    Vector::new(v.x, v.y)
}
fn v(x: Vector) -> V2 {
    v2(x.x, x.y)
}

/// rapier's pipeline holds only workspace buffers; a clone gets a fresh one.
struct FreshPipeline(PhysicsPipeline);
impl Clone for FreshPipeline {
    fn clone(&self) -> Self {
        FreshPipeline(PhysicsPipeline::new())
    }
}

#[derive(Clone)]
struct Rapier {
    params: IntegrationParameters,
    islands: IslandManager,
    broad: BroadPhaseBvh,
    narrow: NarrowPhase,
    bodies: RigidBodySet,
    colliders: ColliderSet,
    joints: ImpulseJointSet,
    multibody: MultibodyJointSet,
    soft: SoftBodySet,
    ccd: CCDSolver,
    pipeline: FreshPipeline,
    world_body: RigidBodyHandle,
}

impl Rapier {
    fn new() -> Rapier {
        let mut bodies = RigidBodySet::new();
        let world_body = bodies.insert(RigidBodyBuilder::fixed().build());
        let params = IntegrationParameters {
            dt: PHYS_DT,
            ..IntegrationParameters::default()
        };
        Rapier {
            params,
            islands: IslandManager::new(),
            broad: BroadPhaseBvh::new(),
            narrow: NarrowPhase::new(),
            bodies,
            colliders: ColliderSet::new(),
            joints: ImpulseJointSet::new(),
            multibody: MultibodyJointSet::new(),
            soft: SoftBodySet::new(),
            ccd: CCDSolver::new(),
            pipeline: FreshPipeline(PhysicsPipeline::new()),
            world_body,
        }
    }

    fn step(&mut self) {
        self.pipeline.0.step(
            Vector::new(0.0, GRAVITY),
            &self.params,
            &mut self.islands,
            &mut self.broad,
            &mut self.narrow,
            &mut self.bodies,
            &mut self.colliders,
            &mut self.joints,
            &mut self.multibody,
            &mut self.soft,
            &mut self.ccd,
            &(),
            &(),
        );
    }

    fn remove_body(&mut self, h: RigidBodyHandle) {
        self.bodies.remove(
            h,
            &mut self.islands,
            &mut self.colliders,
            &mut self.joints,
            &mut self.multibody,
            &mut self.soft,
            true,
        );
    }
}

fn collider_builder(
    c: &ColliderDef,
    body_pos: V2,
    body_c: f32,
    body_s: f32,
    mass_share: f32,
) -> Option<ColliderBuilder> {
    // Shapes are body-local; rapier wants them relative to the body frame too.
    let _ = (body_pos, body_c, body_s);
    let b = match &c.shape {
        Shape::Circle { c: cc, r } => ColliderBuilder::ball(*r).translation(vec(*cc)),
        Shape::Obb { c: cc, axis, half } => ColliderBuilder::cuboid(half.x, half.y)
            .translation(vec(*cc))
            .rotation(axis.angle()),
        Shape::Poly { pts } => {
            let p: Vec<Vector> = pts.iter().map(|p| vec(*p)).collect();
            ColliderBuilder::convex_hull(&p)?
        }
    };
    Some(
        b.collision_groups(groups(c.layer))
            .sensor(c.trigger)
            .friction(c.friction)
            .friction_combine_rule(CoefficientCombineRule::GeometricMean)
            .restitution(c.bounciness)
            .restitution_combine_rule(CoefficientCombineRule::Max)
            .mass(mass_share),
    )
}

/// State of one physics prop.
#[derive(Clone, Debug)]
pub struct Prop {
    /// Index into `MapDef::props`.
    pub def: u32,
    pub alive: bool,
    pub released: bool,
    /// Ticks since release (velocity damping window, maps.md 3.3).
    pub since_release: u32,
    pub pos: V2,
    pub angle: f32,
    pub vel: V2,
    pub angvel: f32,
    pub mass: f32,
    /// World shapes and their layers (cache, refreshed after each step).
    pub shapes: Vec<Shape>,
    pub layers: Vec<u8>,
    pub aabb: Aabb,
    /// Destructible HP (None if not destructible).
    pub hp: Option<f32>,
    /// Frame of the last prop->player impact (once per second rule).
    pub last_player_impact: u32,
    pub last_sound_tick: u32,
    /// MoveSequence state.
    pub move_index: u16,
    pub move_timer: f32,
    pub start_pos: V2,
    handle: RigidBodyHandle,
}

/// A debris piece of a destroyed box (maps.md 5.3).
#[derive(Clone, Debug)]
pub struct Piece {
    /// Index of the destroyed prop in [`Physics::props`] (its `def` is the
    /// `MapDef::props` index).
    pub prop: u32,
    /// Index of the piece in the prop's `DestructibleDef::pieces` (its
    /// sprite: the piece object's renderer).
    pub piece: u32,
    pub pos: V2,
    pub angle: f32,
    pub half: V2,
    /// Game seconds before shrinking starts.
    pub delay: f32,
    /// Shrink progress 0..1 and speed per game second.
    pub shrink: f32,
    pub shrink_speed: f32,
    handle: RigidBodyHandle,
}

impl Piece {
    pub fn scale(&self) -> f32 {
        let t = self.shrink.clamp(0.0, 1.0);
        1.0 - t * t * (3.0 - 2.0 * t)
    }
}

/// What one end of a rope is attached to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RopeAnchor {
    /// A prop (index into [`Physics::props`]).
    Prop(u32),
    /// The static world: the `MapDef::statics` collider containing the
    /// authored end point, if any.
    Static(Option<u32>),
}

/// A rope joint created at map release (maps.md 4).
#[derive(Clone, Debug)]
pub struct Rope {
    /// Index into `MapDef::ropes`.
    pub def: u32,
    /// Attachment of the rope's start (`RopeDef::start`) and end.
    pub anchors: [RopeAnchor; 2],
    /// Anchor points: prop-local for props, world for statics.
    pub local: [V2; 2],
    joint: ImpulseJointHandle,
}

/// A rope for drawing: both end points in world space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RopeView {
    /// Index into `MapDef::ropes`.
    pub def: u32,
    /// World positions of the start and end.
    pub ends: [V2; 2],
    /// What each end is attached to.
    pub anchors: [RopeAnchor; 2],
}

#[derive(Clone, Default)]
pub struct Physics {
    rapier: Option<Box<Rapier>>,
    pub props: Vec<Prop>,
    pub pieces: Vec<Piece>,
    /// Ropes (created by [`Physics::release`]).
    pub ropes: Vec<Rope>,
    /// Kinematic circle per player and the scale its radius was built for.
    player_bodies: Vec<Option<(RigidBodyHandle, f32)>>,
}

impl std::fmt::Debug for Physics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Physics")
            .field("props", &self.props.len())
            .field("pieces", &self.pieces.len())
            .finish()
    }
}

impl Physics {
    pub fn is_active(&self) -> bool {
        self.rapier.is_some()
    }

    pub fn clear(&mut self) {
        *self = Physics::default();
    }

    /// Build the rapier world for a map whose children have arrived. Props are
    /// held kinematic until [`Physics::release`] (map ready).
    pub fn build(&mut self, map: &MapDef, players: usize) {
        self.clear();
        if map.props.is_empty() {
            return;
        }
        let mut r = Rapier::new();
        // Static geometry.
        for s in &map.statics {
            if let Some(cb) = collider_builder(&s.collider, V2::ZERO, 1.0, 0.0, 0.0) {
                let cb = cb;
                r.colliders
                    .insert_with_parent(cb, r.world_body, &mut r.bodies);
            }
        }
        for (i, p) in map.props.iter().enumerate() {
            let rb = match p.body.kind {
                BodyKind::Static => RigidBodyBuilder::fixed(),
                _ => RigidBodyBuilder::kinematic_position_based(),
            }
            .translation(vec(p.pos))
            .rotation(p.angle)
            .linear_damping(p.body.linear_drag)
            .angular_damping(p.body.angular_drag)
            .gravity_scale(p.body.gravity_scale);
            let rb = if p.body.freeze_rotation {
                rb.lock_rotations()
            } else {
                rb
            };
            let h = r.bodies.insert(rb.build());
            let total_area: f32 = p
                .colliders
                .iter()
                .map(|c| c.shape.area())
                .sum::<f32>()
                .max(1e-6);
            for c in &p.colliders {
                let share = p.body.mass * c.shape.area() / total_area;
                if let Some(cb) = collider_builder(c, p.pos, 1.0, 0.0, share) {
                    r.colliders.insert_with_parent(cb, h, &mut r.bodies);
                }
            }
            self.props.push(Prop {
                def: i as u32,
                alive: true,
                released: false,
                since_release: 0,
                pos: p.pos,
                angle: p.angle,
                vel: V2::ZERO,
                angvel: 0.0,
                mass: p.body.mass,
                shapes: Vec::new(),
                layers: p.colliders.iter().map(|c| c.layer).collect(),
                aabb: Aabb::default(),
                hp: p.destructible.as_ref().map(|d| d.hp),
                last_player_impact: 0,
                last_sound_tick: 0,
                move_index: 0,
                move_timer: 0.0,
                start_pos: p.pos,
                handle: h,
            });
        }
        // Hand-authored joints.
        for j in &map.joints {
            let (Some(a), b) = (
                self.props.get(j.body_a as usize).map(|p| p.handle),
                j.body_b
                    .and_then(|b| self.props.get(b as usize).map(|p| p.handle)),
            ) else {
                continue;
            };
            let b_handle = b.unwrap_or(r.world_body);
            let anchor_b = j.anchor_b;
            let data: GenericJoint = match j.kind {
                JointKind::Fixed => {
                    // Keep the current relative pose.
                    let ra = *r.bodies[a].position();
                    let rbp = *r.bodies[b_handle].position();
                    let rel = ra.inverse() * rbp;
                    let _ = anchor_b;
                    FixedJointBuilder::new()
                        .local_frame1(rel)
                        .local_frame2(Pose::IDENTITY)
                        .build()
                        .into()
                }
                JointKind::Hinge { limits } => {
                    let mut jb = RevoluteJointBuilder::new()
                        .local_anchor1(vec(j.anchor_a))
                        .local_anchor2(vec(anchor_b));
                    if let Some((lo, hi)) = limits {
                        jb = jb.limits([lo * crate::math::DEG2RAD, hi * crate::math::DEG2RAD]);
                    }
                    jb.build().into()
                }
                JointKind::Spring {
                    frequency,
                    damping_ratio,
                    distance,
                } => {
                    let d = distance.unwrap_or_else(|| {
                        world_anchor(&r, a, j.anchor_a).dist(world_anchor(&r, b_handle, anchor_b))
                    });
                    spring_joint(
                        &r,
                        a,
                        b_handle,
                        j.anchor_a,
                        anchor_b,
                        d,
                        frequency,
                        damping_ratio,
                    )
                }
                JointKind::Distance { distance, max_only } => {
                    let d = distance.unwrap_or_else(|| {
                        world_anchor(&r, a, j.anchor_a).dist(world_anchor(&r, b_handle, anchor_b))
                    });
                    distance_joint(j.anchor_a, anchor_b, d, max_only)
                }
            };
            let mut data = data;
            data.contacts_enabled = j.collide_connected;
            r.joints.insert(a, b_handle, data, true);
        }
        self.player_bodies = vec![None; players];
        self.rapier = Some(Box::new(r));
        self.sync();
    }

    /// Map ready: create ropes and make bodies dynamic (maps.md 1, 3.3, 4).
    pub fn release(&mut self, map: &MapDef) {
        let Some(r) = self.rapier.as_mut() else {
            return;
        };
        for (ri, rope) in map.ropes.iter().enumerate() {
            let find = |p: V2, props: &[Prop]| {
                props
                    .iter()
                    .position(|pr| pr.alive && pr.shapes.iter().any(|s| s.contains(p)))
            };
            let find_static = |p: V2| {
                map.statics
                    .iter()
                    .position(|s| s.collider.shape.contains(p))
                    .map(|i| i as u32)
            };
            let sa = find(rope.start, &self.props);
            let sb = find(rope.end, &self.props);
            // (body1, anchor1, body2, anchor2, swapped: body1 holds the end)
            let (body1, anchor1, body2, anchor2, swapped) = match (sa, sb) {
                (Some(a), Some(b)) => {
                    let ha = self.props[a].handle;
                    let hb = self.props[b].handle;
                    (
                        ha,
                        local_point(r, ha, rope.start),
                        hb,
                        local_point(r, hb, rope.end),
                        false,
                    )
                }
                (Some(a), None) => {
                    let ha = self.props[a].handle;
                    (
                        ha,
                        local_point(r, ha, rope.start),
                        r.world_body,
                        rope.end,
                        false,
                    )
                }
                (None, Some(b)) => {
                    let hb = self.props[b].handle;
                    (
                        hb,
                        local_point(r, hb, rope.end),
                        r.world_body,
                        rope.start,
                        true,
                    )
                }
                (None, None) => continue,
            };
            let d = world_anchor(r, body1, anchor1).dist(world_anchor(r, body2, anchor2));
            let mut data = match rope.kind {
                RopeKind::Spring => spring_joint(r, body1, body2, anchor1, anchor2, d, 1.0, 0.0),
                RopeKind::Distance => distance_joint(anchor1, anchor2, d, false),
            };
            data.contacts_enabled = true;
            let joint = r.joints.insert(body1, body2, data, true);
            let anchor = |p: Option<usize>, pt: V2| match p {
                Some(i) => RopeAnchor::Prop(i as u32),
                None => RopeAnchor::Static(find_static(pt)),
            };
            let (l_start, l_end) = if swapped {
                (anchor2, anchor1)
            } else {
                (anchor1, anchor2)
            };
            self.ropes.push(Rope {
                def: ri as u32,
                anchors: [anchor(sa, rope.start), anchor(sb, rope.end)],
                local: [l_start, l_end],
                joint,
            });
        }
        for (i, p) in self.props.iter_mut().enumerate() {
            let def = &map.props[i];
            if def.body.kind == BodyKind::Dynamic && p.alive {
                if let Some(b) = r.bodies.get_mut(p.handle) {
                    b.set_body_type(RigidBodyType::Dynamic, true);
                }
                p.released = true;
            }
        }
    }

    /// Body handle of the player's kinematic circle; created on demand.
    pub fn set_player(&mut self, idx: usize, pos: Option<V2>, scale: f32) {
        let Some(r) = self.rapier.as_mut() else {
            return;
        };
        if idx >= self.player_bodies.len() {
            self.player_bodies.resize(idx + 1, None);
        }
        match (pos, self.player_bodies[idx]) {
            (Some(p), Some((h, built))) => {
                if let Some(b) = r.bodies.get_mut(h) {
                    b.set_next_kinematic_translation(vec(p));
                    // Size changes mid-round (player.md 7.5): radius follows.
                    if built != scale {
                        let cs: Vec<ColliderHandle> = b.colliders().to_vec();
                        for c in cs {
                            if let Some(col) = r.colliders.get_mut(c) {
                                col.set_shape(SharedShape::ball(0.15 * scale));
                            }
                        }
                        self.player_bodies[idx] = Some((h, scale));
                    }
                }
            }
            (Some(p), None) => {
                let h = r.bodies.insert(
                    RigidBodyBuilder::kinematic_position_based()
                        .translation(vec(p))
                        .build(),
                );
                let c = ColliderBuilder::ball(0.15 * scale)
                    .collision_groups(groups(layers::PLAYER_OBJECT_COLLIDER))
                    .friction(0.4);
                r.colliders.insert_with_parent(c, h, &mut r.bodies);
                self.player_bodies[idx] = Some((h, scale));
            }
            (None, Some((h, _))) => {
                r.remove_body(h);
                self.player_bodies[idx] = None;
            }
            (None, None) => {}
        }
    }

    /// Radius of player `idx`'s kinematic prop-collision circle (tests).
    #[cfg(test)]
    pub(crate) fn player_circle_radius(&self, idx: usize) -> Option<f32> {
        let r = self.rapier.as_ref()?;
        let (h, _) = (*self.player_bodies.get(idx)?)?;
        let c = *r.bodies.get(h)?.colliders().first()?;
        Some(r.colliders.get(c)?.shape().as_ball()?.radius)
    }

    pub fn add_force_at(&mut self, prop: usize, f: V2, point: V2) {
        let Some(r) = self.rapier.as_mut() else {
            return;
        };
        let p = &self.props[prop];
        if !p.alive || !p.released {
            return;
        }
        if let Some(b) = r.bodies.get_mut(p.handle) {
            b.add_force_at_point(vec(f), vec(point), true);
        }
    }
    pub fn add_force(&mut self, prop: usize, f: V2) {
        let Some(r) = self.rapier.as_mut() else {
            return;
        };
        let p = &self.props[prop];
        if !p.alive || !p.released {
            return;
        }
        if let Some(b) = r.bodies.get_mut(p.handle) {
            b.add_force(vec(f), true);
        }
    }
    pub fn add_torque(&mut self, prop: usize, t: f32) {
        let Some(r) = self.rapier.as_mut() else {
            return;
        };
        let p = &self.props[prop];
        if !p.alive || !p.released {
            return;
        }
        if let Some(b) = r.bodies.get_mut(p.handle) {
            b.add_torque(t, true);
        }
    }
    pub fn impulse_at(&mut self, prop: usize, j: V2, point: V2) {
        let Some(r) = self.rapier.as_mut() else {
            return;
        };
        let p = &self.props[prop];
        if !p.alive || !p.released {
            return;
        }
        if let Some(b) = r.bodies.get_mut(p.handle) {
            b.apply_impulse_at_point(vec(j), vec(point), true);
        }
    }
    pub fn impulse(&mut self, prop: usize, j: V2) {
        let Some(r) = self.rapier.as_mut() else {
            return;
        };
        let p = &self.props[prop];
        if !p.alive || !p.released {
            return;
        }
        if let Some(b) = r.bodies.get_mut(p.handle) {
            b.apply_impulse(vec(j), true);
        }
    }
    /// Multiply the prop's linear and angular velocity (prop-player impact).
    pub fn scale_velocity(&mut self, prop: usize, k: f32) {
        let Some(r) = self.rapier.as_mut() else {
            return;
        };
        let p = &mut self.props[prop];
        if let Some(b) = r.bodies.get_mut(p.handle) {
            let lv = b.linvel();
            let av = b.angvel();
            b.set_linvel(lv * k, true);
            b.set_angvel(av * k, true);
            p.vel *= k;
            p.angvel *= k;
        }
    }

    /// Destroy a destructible box: remove its body and spawn debris (maps.md 5.3).
    /// `rand` yields uniform [0,1) numbers from the sim PRNG.
    pub fn destroy(
        &mut self,
        map: &MapDef,
        prop: usize,
        damage: V2,
        mut rand: impl FnMut() -> f32,
    ) {
        let Some(r) = self.rapier.as_mut() else {
            return;
        };
        let p = &mut self.props[prop];
        if !p.alive {
            return;
        }
        p.alive = false;
        p.shapes.clear();
        let (pos, angle, vel) = (p.pos, p.angle, p.vel);
        r.remove_body(p.handle);
        let Some(d) = map.props[p.def as usize].destructible.as_ref() else {
            return;
        };
        let (c, s) = (crate::math::cos(angle), crate::math::sin(angle));
        for (pi, pc) in d.pieces.iter().enumerate() {
            let wp = pos + pc.pos.rotate(c, s);
            let h = r.bodies.insert(
                RigidBodyBuilder::dynamic()
                    .translation(vec(wp))
                    .rotation(angle + pc.angle)
                    .linvel(vec(vel))
                    .linear_damping(pc.body.linear_drag)
                    .angular_damping(pc.body.angular_drag)
                    .build(),
            );
            let mut cdef = pc.collider.clone();
            cdef.layer = layers::BACKGROUND_OBJECT;
            if let Some(cb) = collider_builder(&cdef, wp, 1.0, 0.0, pc.body.mass) {
                r.colliders.insert_with_parent(cb, h, &mut r.bodies);
            }
            let imp = damage * (rand() * 500.0);
            let torque = (rand() * 2.0 - 1.0) * 1000.0;
            if let Some(b) = r.bodies.get_mut(h) {
                b.apply_impulse(vec(imp), true);
                b.apply_torque_impulse(torque, true);
            }
            let half = match &pc.collider.shape {
                Shape::Obb { half, .. } => *half,
                Shape::Circle { r, .. } => v2(*r, *r),
                s => {
                    let a = s.aabb();
                    (a.max - a.min) * 0.5
                }
            };
            let delay = rand() * 0.5;
            let shrink_speed = 1.5 + rand();
            self.pieces.push(Piece {
                prop: prop as u32,
                piece: pi as u32,
                pos: wp,
                angle: angle + pc.angle,
                half,
                delay,
                shrink: 0.0,
                shrink_speed,
                handle: h,
            });
        }
    }

    /// Advance one fixed step. `game_dt` drives the debris timers.
    pub fn step(&mut self, game_dt: f32) {
        let Some(r) = self.rapier.as_mut() else {
            return;
        };
        // Release damping: v -= v * t * 0.05 for 1 s after release.
        for p in self.props.iter_mut() {
            if !p.alive || !p.released {
                continue;
            }
            if p.since_release < 72 {
                let t = p.since_release as f32 / 71.43;
                if let Some(b) = r.bodies.get_mut(p.handle) {
                    let lv = b.linvel();
                    b.set_linvel(lv - lv * (t * 0.05), false);
                }
                p.since_release += 1;
            }
        }
        r.step();
        for p in self.props.iter() {
            if let Some(b) = r.bodies.get_mut(p.handle) {
                b.reset_forces(false);
                b.reset_torques(false);
            }
        }
        // Debris timers.
        let mut i = 0;
        while i < self.pieces.len() {
            let pc = &mut self.pieces[i];
            if pc.delay > 0.0 {
                pc.delay -= game_dt;
            } else {
                pc.shrink += game_dt * pc.shrink_speed;
            }
            if pc.shrink >= 1.0 {
                let h = pc.handle;
                r.remove_body(h);
                self.pieces.remove(i);
            } else {
                i += 1;
            }
        }
        self.sync();
    }

    /// Refresh the cached poses/shapes from rapier.
    fn sync(&mut self) {
        let Some(r) = self.rapier.as_ref() else {
            return;
        };
        for p in self.props.iter_mut() {
            if !p.alive {
                continue;
            }
            let Some(b) = r.bodies.get(p.handle) else {
                continue;
            };
            let t = b.translation();
            let rot = b.rotation();
            p.pos = v(t);
            p.angle = rot.angle();
            p.vel = v(b.linvel());
            p.angvel = b.angvel();
            p.shapes.clear();
            let (c, s) = (rot.re, rot.im);
            let mut aabb: Option<Aabb> = None;
            for ch in b.colliders() {
                let Some(col) = r.colliders.get(*ch) else {
                    continue;
                };
                let local = col.position_wrt_parent().copied().unwrap_or(Pose::IDENTITY);
                let lc = v(local.translation);
                let (lcs, lsn) = (local.rotation.re, local.rotation.im);
                let shape = col.shape();
                let world = if let Some(ball) = shape.as_ball() {
                    Shape::Circle {
                        c: p.pos + lc.rotate(c, s),
                        r: ball.radius,
                    }
                } else if let Some(cub) = shape.as_cuboid() {
                    let axis = v2(lcs, lsn).rotate(c, s);
                    Shape::Obb {
                        c: p.pos + lc.rotate(c, s),
                        axis,
                        half: v(cub.half_extents),
                    }
                } else if let Some(poly) = shape.as_convex_polygon() {
                    Shape::Poly {
                        pts: poly
                            .points()
                            .iter()
                            .map(|q| p.pos + (lc + v(*q).rotate(lcs, lsn)).rotate(c, s))
                            .collect(),
                    }
                } else {
                    continue;
                };
                let a = world.aabb();
                aabb = Some(aabb.map(|b| b.union(&a)).unwrap_or(a));
                p.shapes.push(world);
            }
            p.aabb = aabb.unwrap_or_default();
        }
        for pc in self.pieces.iter_mut() {
            if let Some(b) = r.bodies.get(pc.handle) {
                pc.pos = v(b.translation());
                pc.angle = b.rotation().angle();
            }
        }
    }

    /// The ropes that still hold (their joint exists and every attached prop
    /// is alive), with world end points from the props' current poses.
    pub fn rope_views(&self) -> Vec<RopeView> {
        let Some(r) = self.rapier.as_ref() else {
            return Vec::new();
        };
        let mut out = Vec::with_capacity(self.ropes.len());
        'ropes: for rope in &self.ropes {
            if r.joints.get(rope.joint).is_none() {
                continue;
            }
            let mut ends = [V2::ZERO; 2];
            for ((end, anchor), local) in ends.iter_mut().zip(rope.anchors).zip(rope.local) {
                *end = match anchor {
                    RopeAnchor::Prop(i) => {
                        let Some(p) = self.props.get(i as usize).filter(|p| p.alive) else {
                            continue 'ropes;
                        };
                        let (c, s) = (crate::math::cos(p.angle), crate::math::sin(p.angle));
                        p.pos + local.rotate(c, s)
                    }
                    RopeAnchor::Static(_) => local,
                };
            }
            out.push(RopeView {
                def: rope.def,
                ends,
                anchors: rope.anchors,
            });
        }
        out
    }

    /// Hash the dynamic state (for `World::checksum`).
    pub fn hash_into(&self, h: &mut crate::world::Hasher) {
        for p in &self.props {
            h.bool(p.alive);
            h.v2(p.pos);
            h.f32(p.angle);
            h.v2(p.vel);
            h.f32(p.angvel);
            h.f32(p.hp.unwrap_or(-1.0));
            h.u32(p.move_index as u32);
        }
        for pc in &self.pieces {
            h.v2(pc.pos);
            h.f32(pc.angle);
        }
    }
}

fn local_point(r: &Rapier, h: RigidBodyHandle, world: V2) -> V2 {
    let b = &r.bodies[h];
    let p = b.position();
    v(p.inverse_transform_point(vec(world)))
}

fn world_anchor(r: &Rapier, h: RigidBodyHandle, local: V2) -> V2 {
    let b = &r.bodies[h];
    v(b.position().transform_point(vec(local)))
}

/// Unity `SpringJoint2D` (frequency in Hz, damping ratio) as a rapier spring:
/// stiffness from the reduced mass of the pair.
#[allow(clippy::too_many_arguments)]
fn spring_joint(
    r: &Rapier,
    a: RigidBodyHandle,
    b: RigidBodyHandle,
    la: V2,
    lb: V2,
    rest: f32,
    freq: f32,
    ratio: f32,
) -> GenericJoint {
    let ma = r.bodies[a].mass();
    let mb = r.bodies[b].mass();
    let m = if r.bodies[b].is_fixed() || mb <= 0.0 {
        ma
    } else if r.bodies[a].is_fixed() || ma <= 0.0 {
        mb
    } else {
        ma * mb / (ma + mb)
    };
    let omega = 2.0 * core::f32::consts::PI * freq;
    let k = m * omega * omega;
    let c = 2.0 * m * ratio * omega;
    SpringJointBuilder::new(rest, k, c)
        .local_anchor1(vec(la))
        .local_anchor2(vec(lb))
        .build()
        .into()
}

/// Unity `DistanceJoint2D`: rigid both ways (or max only).
fn distance_joint(la: V2, lb: V2, d: f32, max_only: bool) -> GenericJoint {
    let mut j: GenericJoint = RopeJointBuilder::new(d)
        .local_anchor1(vec(la))
        .local_anchor2(vec(lb))
        .build()
        .into();
    if !max_only {
        j.set_limits(JointAxis::LinX, [d, d]);
    }
    j
}
