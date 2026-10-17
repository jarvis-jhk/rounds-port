//! Map scenes -> [`MapDef`] (docs/spec/maps.md).
//!
//! Classification of the active objects of a scene (inactive subtrees are
//! dropped, except the hidden `Pieces` of destructible boxes):
//! - an object with a simulated dynamic/kinematic `Rigidbody2D` is a prop;
//!   colliders on it and on descendants without their own body belong to it
//!   (body-local shapes, the body frame has no scale);
//! - colliders below a `CurveAnimation` group that animates (smashers) belong
//!   to an [`AnimatedDef`] (pivot-local), a trigger with `KillBox` becomes its
//!   kill box;
//! - every other collider is static geometry (world space);
//! - `MapObjet_Rope` objects become [`RopeDef`]s (start = the object, end =
//!   its first child), joint components become [`JointDef`]s;
//! - prop scripts (`NetworkPhysicsObject`, `MoveSequence`, `RigLookUp`,
//!   `DamagableEvent` + `DestructibleBoxDestruction`, `DamageBox` + `Rotate`)
//!   fill the prop's optional parts.

use std::collections::BTreeMap;

use rounds_import::format::{Collider, Component, Joint, Map, Object, Rigidbody};
use rounds_sim::content::{
    Affine, AnimatedDef, BodyDef, BodyKind, ColliderDef, DamageBoxDef, DestructibleDef, JointDef,
    JointKind, KillBoxDef, MapDef, MoveSequenceDef, PieceDef, PropDef, PushableDef, RopeDef,
    RopeKind, SpawnPointDef, StaticDef,
};
use rounds_sim::geom::Shape;
use rounds_sim::math::{V2, v2};
use serde_json::Value;

use crate::fields::{b, f, field_curve, i, num};
use crate::report::Report;
use crate::xform::{Approx, affine, angle_of, collider_shapes, pose};

/// Components on map objects that are presentation, audio, networking or
/// editor tooling: skipped on purpose (maps.md 3.3, 5.4, 7.9, 10).
const IGNORED: &[&str] = &[
    "SpriteRenderer",
    "SFPolygon",
    "SpriteMasking.SpriteMask",
    "SpriteMask",
    "SkipMasking",
    "GetColor",
    "ColorBlink",
    "ForceMultiplier",
    "PhotonMapObject",
    "Photon.Pun.PhotonView",
    "MapMirror",
    "LineRenderer",
    "ParticleSystem",
    "ParticleSystemRenderer",
    "ParticleSystemForceField",
    "EnableEvent",
    "SoundImplementation.SoundUnityEventPlayer",
    "SoundImplementation.SoundEnvironmentRigidbody",
    "Grass",
    "PerlinWordScale",
    "ParticleTime",
    "MoveForward",
    "DeathEffect",
    "Screenshaker",
    "SFLight",
];

/// Components the conversion turns into sim data.
const HANDLED: &[&str] = &[
    "Map",
    "SpawnPoint",
    "MapObjet_Rope",
    "MoveSequence",
    "RigLookUp",
    "NetworkPhysicsObject",
    "DamagableEvent",
    "DestructibleBoxDestruction",
    "DamageBox",
    "Rotate",
    "CurveAnimation",
    "KillBox",
    "Rigidbody2D",
    "BoxCollider2D",
    "CircleCollider2D",
    "CapsuleCollider2D",
    "PolygonCollider2D",
    "EdgeCollider2D",
    "FixedJoint2D",
    "HingeJoint2D",
    "SpringJoint2D",
    "DistanceJoint2D",
    "RemoveAfterSeconds",
];

pub fn script(c: &Component) -> &str {
    c.script.as_deref().unwrap_or(&c.kind)
}

fn comp<'a>(o: &'a Object, name: &str) -> Option<&'a Value> {
    o.components
        .iter()
        .find(|c| script(c) == name)
        .map(|c| &c.fields)
}

fn has(o: &Object, name: &str) -> bool {
    o.components.iter().any(|c| script(c) == name)
}

/// Default physics material (friction 0.4, bounciness 0) unless set.
fn collider_def(c: &Collider, shape: Shape, layer: u32) -> ColliderDef {
    let (friction, bounciness) = c
        .material
        .as_ref()
        .map(|m| (m.friction, m.bounciness))
        .unwrap_or((0.4, 0.0));
    ColliderDef {
        shape,
        layer: layer.min(31) as u8,
        trigger: c.is_trigger,
        friction,
        bounciness,
    }
}

fn body_def(
    rb: &Rigidbody,
    colliders: &[ColliderDef],
    density: f32,
    r: &mut Report,
    ctx: &str,
) -> BodyDef {
    let kind = match rb.body_type {
        1 => BodyKind::Kinematic,
        2 => BodyKind::Static,
        _ => BodyKind::Dynamic,
    };
    if rb.constraints & 3 != 0 {
        r.unsupported("Rigidbody2D.constraints(freeze position)");
        r.warn(format!(
            "{ctx}: position constraints {} ignored",
            rb.constraints
        ));
    }
    let mass = if rb.use_auto_mass {
        r.count("bodies with auto mass", 1);
        (colliders.iter().map(|c| c.shape.area()).sum::<f32>() * density).max(1e-3)
    } else {
        rb.mass
    };
    BodyDef {
        kind,
        mass,
        linear_drag: rb.linear_drag,
        angular_drag: rb.angular_drag,
        gravity_scale: rb.gravity_scale,
        freeze_rotation: rb.constraints & 4 != 0,
    }
}

/// Map-level conversion state.
struct Conv<'a> {
    m: &'a Map,
    objs: &'a [Object],
    world: Vec<Affine>,
    /// Object index -> index of the object owning its colliders' body.
    owner: Vec<Option<usize>>,
    /// Object index -> animated group root.
    anim: Vec<Option<usize>>,
    prop_of: BTreeMap<usize, u32>,
}

impl Conv<'_> {
    fn path(&self, mut i: usize) -> String {
        let mut parts = vec![self.objs[i].name.as_str()];
        while let Some(p) = self.objs[i].parent {
            parts.push(self.objs[p].name.as_str());
            i = p;
        }
        parts.reverse();
        parts.join("/")
    }

    fn ctx(&self, i: usize) -> String {
        format!("{} {}", self.m.name, self.path(i))
    }

    /// Indices of `root`'s subtree (preorder, root first).
    fn subtree(&self, root: usize) -> Vec<usize> {
        let mut out = vec![];
        let mut stack = vec![root];
        while let Some(i) = stack.pop() {
            out.push(i);
            for &c in self.objs[i].children.iter().rev() {
                stack.push(c);
            }
        }
        out
    }
}

/// A body that owns colliders: simulated dynamic or kinematic.
fn is_body(o: &Object) -> bool {
    o.rigidbody
        .as_ref()
        .is_some_and(|rb| rb.simulated && rb.body_type != 2)
}

/// A `CurveAnimation` group that plays (on awake or via a `DelayEvent`).
fn anim_start(o: &Object) -> Option<f32> {
    let ca = comp(o, "CurveAnimation")?;
    let a = ca.get("animations")?.as_array()?.first()?;
    if b(a, "playOnAwake", false) {
        return Some(0.0);
    }
    let de = comp(o, "DelayEvent")?;
    let calls_play = de
        .pointer("/delayedEvent/m_PersistentCalls/m_Calls")
        .and_then(Value::as_array)
        .is_some_and(|c| {
            c.iter()
                .any(|c| c.get("m_MethodName").and_then(Value::as_str) == Some("PlayIn"))
        });
    (b(de, "auto", true) && calls_play).then(|| f(de, "time", 0.0))
}

#[allow(clippy::needless_range_loop)]
pub fn convert_map(m: &Map, r: &mut Report) -> MapDef {
    let objs = &m.objects;
    let n = objs.len();
    let world: Vec<Affine> = objs.iter().map(|o| affine(o.world.matrix)).collect();
    let mut owner = vec![None; n];
    let mut anim = vec![None; n];
    for (k, o) in objs.iter().enumerate() {
        let parent_owner = o.parent.and_then(|p| owner[p]);
        owner[k] = if is_body(o) { Some(k) } else { parent_owner };
        let parent_anim = o.parent.and_then(|p| anim[p]);
        anim[k] = if parent_anim.is_none() && has(o, "CurveAnimation") && anim_start(o).is_some() {
            Some(k)
        } else {
            parent_anim
        };
    }
    let mut c = Conv {
        m,
        objs,
        world,
        owner,
        anim,
        prop_of: BTreeMap::new(),
    };
    let mut def = MapDef {
        name: m.name.clone(),
        camera_size: m.map.as_ref().map(|v| f(v, "size", 20.0)).unwrap_or(20.0),
        ..Default::default()
    };

    // Spawn points (hierarchy order, active only).
    for sp in &m.spawn_points {
        if objs.get(sp.object).is_none_or(|o| o.active_in_hierarchy) {
            def.spawn_points.push(SpawnPointDef {
                pos: v2(sp.position[0], sp.position[1]),
                team_id: sp.team as i32,
            });
        }
    }
    if def.spawn_points.is_empty() {
        r.error(format!("{}: no spawn points", m.name));
    }

    let inactive = objs.iter().filter(|o| !o.active_in_hierarchy).count();
    r.count("map objects", n);
    r.count("map objects inactive (skipped)", inactive);

    // Component census of active objects.
    for (k, o) in objs.iter().enumerate() {
        if !o.active_in_hierarchy {
            continue;
        }
        for cp in &o.components {
            let s = script(cp);
            if IGNORED.contains(&s) {
                r.ignored(s);
            } else if s == "DelayEvent" && c.anim[k] == Some(k) {
                // Smasher start delay (consumed by the animated group).
            } else if !HANDLED.contains(&s) {
                r.unsupported(s);
                r.warn(format!("{}: unsupported component {s}", c.ctx(k)));
            }
        }
    }

    // Props first (joints refer to prop indices).
    for k in 0..n {
        let o = &objs[k];
        if !o.active_in_hierarchy || c.owner[k] != Some(k) {
            continue;
        }
        if let Some(p) = convert_prop(&c, k, r) {
            c.prop_of.insert(k, def.props.len() as u32);
            def.props.push(p);
        }
    }
    // Joints.
    for k in 0..n {
        let o = &objs[k];
        if !o.active_in_hierarchy || o.joints.is_empty() {
            continue;
        }
        for j in &o.joints {
            if let Some(jd) = convert_joint(&c, k, j, r) {
                def.joints.push(jd);
            }
        }
    }
    // Animated groups.
    for k in 0..n {
        if objs[k].active_in_hierarchy && c.anim[k] == Some(k) {
            if let Some(a) = convert_anim(&c, k, r) {
                def.animated.push(a);
            }
        } else if objs[k].active_in_hierarchy
            && has(&objs[k], "CurveAnimation")
            && c.anim[k].is_none()
        {
            r.ignored("CurveAnimation (not playing)");
        }
    }
    // Statics and ropes.
    for k in 0..n {
        let o = &objs[k];
        if !o.active_in_hierarchy {
            continue;
        }
        if c.owner[k].is_none() && c.anim[k].is_none() {
            for col in o.colliders.iter().filter(|x| x.enabled) {
                let (shapes, approx) = collider_shapes(col, &c.world[k]);
                note_approx(r, approx, &c.ctx(k));
                let damage_box = comp(o, "DamageBox").map(|d| damage_box(&c, k, d));
                for s in shapes {
                    def.statics.push(StaticDef {
                        id: o.id.clone(),
                        name: o.name.clone(),
                        collider: collider_def(col, s, o.layer),
                        damage_box,
                    });
                }
            }
        }
        if has(o, "MapObjet_Rope") {
            let rope = comp(o, "MapObjet_Rope").unwrap();
            let kind = if i(rope, "jointType", 0) == 1 {
                RopeKind::Distance
            } else {
                RopeKind::Spring
            };
            match o.children.first() {
                Some(&e) => def.ropes.push(RopeDef {
                    start: c.world[k].t,
                    end: c.world[e].t,
                    kind,
                }),
                None => r.warn(format!("{}: rope without end object", c.ctx(k))),
            }
        }
    }

    r.count("maps", 1);
    r.count("map statics", def.statics.len());
    r.count("map props", def.props.len());
    r.count("map joints", def.joints.len());
    r.count("map animated groups", def.animated.len());
    r.count(
        "map kill boxes",
        def.animated.iter().filter(|a| a.kill_box.is_some()).count(),
    );
    r.count(
        "map ropes (spring)",
        def.ropes
            .iter()
            .filter(|x| x.kind == RopeKind::Spring)
            .count(),
    );
    r.count(
        "map ropes (distance)",
        def.ropes
            .iter()
            .filter(|x| x.kind == RopeKind::Distance)
            .count(),
    );
    r.count(
        "map damage boxes",
        def.statics
            .iter()
            .filter(|s| s.damage_box.is_some())
            .count()
            + def.props.iter().filter(|p| p.damage_box.is_some()).count(),
    );
    r.count(
        "map destructibles",
        def.props
            .iter()
            .filter(|p| p.destructible.is_some())
            .count(),
    );
    r.count(
        "map destructible pieces",
        def.props
            .iter()
            .filter_map(|p| p.destructible.as_ref())
            .map(|d| d.pieces.len())
            .sum(),
    );
    r.count(
        "map moving platforms",
        def.props
            .iter()
            .filter(|p| p.move_sequence.is_some())
            .count(),
    );
    r.count("map spawn points", def.spawn_points.len());
    def
}

fn note_approx(r: &mut Report, a: Approx, ctx: &str) {
    match a {
        Approx::Exact => {}
        Approx::RoundedCorners => r.count("colliders with rounded corners squared", 1),
        Approx::Decomposed => r.count("polygon colliders decomposed", 1),
        Approx::Dropped => r.warn(format!("{ctx}: degenerate collider dropped")),
    }
}

fn damage_box(c: &Conv, k: usize, d: &Value) -> DamageBoxDef {
    let dflt = DamageBoxDef::default();
    // Blade rotation: `Rotate` on the object or a descendant ("Sprite").
    let rotate = c
        .subtree(k)
        .into_iter()
        .find_map(|j| comp(&c.objs[j], "Rotate").and_then(|r| num(r, "speed")))
        .unwrap_or(dflt.rotate_speed);
    DamageBoxDef {
        damage: f(d, "damage", dflt.damage),
        force: f(d, "force", dflt.force),
        cd: f(d, "cd", dflt.cd),
        flying_for: f(d, "setFlyingFor", dflt.flying_for),
        shake: f(d, "shake", dflt.shake),
        rotate_speed: rotate,
    }
}

/// Colliders of body `k` in its body frame: on `k` and on active descendants
/// that do not have their own body.
fn body_colliders(
    c: &Conv,
    k: usize,
    inv: &Affine,
    r: &mut Report,
    active_only: bool,
) -> (Vec<ColliderDef>, f32) {
    let mut out = vec![];
    let mut density = 1.0;
    for j in c.subtree(k) {
        let o = &c.objs[j];
        if j != k && o.rigidbody.is_some() {
            continue;
        }
        if active_only && !o.active_in_hierarchy {
            continue;
        }
        // Skip descendants of nested bodies.
        let mut p = if j == k { None } else { o.parent };
        let mut nested = false;
        while let Some(q) = p {
            if q == k {
                break;
            }
            if c.objs[q].rigidbody.is_some() {
                nested = true;
                break;
            }
            p = c.objs[q].parent;
        }
        if nested || (j != k && c.anim[j].is_some() && c.anim[j] != c.anim[k]) {
            continue;
        }
        let local = inv.mul(&c.world[j]);
        for col in o.colliders.iter().filter(|x| x.enabled) {
            density = col.density;
            let (shapes, approx) = collider_shapes(col, &local);
            note_approx(r, approx, &c.ctx(j));
            out.extend(shapes.into_iter().map(|s| collider_def(col, s, o.layer)));
        }
    }
    (out, density)
}

fn convert_prop(c: &Conv, k: usize, r: &mut Report) -> Option<PropDef> {
    let o = &c.objs[k];
    let rb = o.rigidbody.as_ref()?;
    let pos = c.world[k].t;
    let angle = angle_of(o);
    let inv = pose(pos, angle).inverse();
    let (colliders, density) = body_colliders(c, k, &inv, r, true);
    if colliders.is_empty() {
        r.warn(format!("{}: rigidbody without colliders skipped", c.ctx(k)));
        r.count("bodies without colliders (skipped)", 1);
        return None;
    }
    let ctx = c.ctx(k);
    let body = body_def(rb, &colliders, density, r, &ctx);
    let pushable = comp(o, "NetworkPhysicsObject").map(|n| {
        let d = PushableDef::default();
        PushableDef {
            dmg_amount: f(n, "dmgAmount", d.dmg_amount),
            force_amount: f(n, "forceAmount", d.force_amount),
            player_col_threshold: f(n, "playerColThreshold", d.player_col_threshold),
            speed: f(n, "speed", d.speed),
            bullet_push_multiplier: f(n, "bulletPushMultiplier", d.bullet_push_multiplier),
        }
    });
    let parent_lin = o.parent.map(|p| c.world[p]).unwrap_or(Affine::IDENTITY);
    if o.parent.is_some()
        && has(o, "MoveSequence")
        && (parent_lin.ax != v2(1.0, 0.0) || parent_lin.ay != v2(0.0, 1.0))
    {
        r.warn(format!(
            "{ctx}: MoveSequence under a rotated/scaled parent; offsets transformed by the parent"
        ));
    }
    let move_sequence = comp(o, "MoveSequence").map(|ms| MoveSequenceDef {
        positions: ms
            .get("positions")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|p| parent_lin.apply_vec(v2(f(p, "x", 0.0), f(p, "y", 0.0))))
                    .collect()
            })
            .unwrap_or_default(),
        drag: f(ms, "drag", 1.0),
        spring: f(ms, "spring", 0.0),
        cap: f(ms, "cap", 5.0),
        threshold: f(ms, "threshold", 3.0),
        time_at_pos: f(ms, "timeAtPos", 0.0),
    });
    let rig_look_up = comp(o, "RigLookUp").map(|x| f(x, "force", 3.0));
    let destructible = comp(o, "DamagableEvent").map(|d| {
        if !has(o, "DestructibleBoxDestruction") {
            r.warn(format!(
                "{ctx}: DamagableEvent without DestructibleBoxDestruction"
            ));
        }
        DestructibleDef {
            hp: f(d, "maxHP", f(d, "currentHP", 50.0)),
            pieces: pieces(c, k, &inv, angle, r),
        }
    });
    let damage_box = comp(o, "DamageBox").map(|d| damage_box(c, k, d));
    Some(PropDef {
        id: o.id.clone(),
        name: o.name.clone(),
        pos,
        angle,
        colliders,
        body,
        pushable,
        move_sequence,
        rig_look_up,
        destructible,
        damage_box,
    })
}

/// Debris of a destructible box: the bodies below it (inactive `Pieces`).
fn pieces(c: &Conv, k: usize, box_inv: &Affine, box_angle: f32, r: &mut Report) -> Vec<PieceDef> {
    let mut out = vec![];
    for j in c.subtree(k).into_iter().skip(1) {
        let o = &c.objs[j];
        let Some(rb) = o.rigidbody.as_ref() else {
            continue;
        };
        let wp = c.world[j].t;
        let pa = angle_of(o);
        let inv = pose(wp, pa).inverse();
        let (cols, density) = body_colliders(c, j, &inv, r, false);
        let Some(collider) = cols.first().cloned() else {
            continue;
        };
        if cols.len() > 1 {
            r.warn(format!(
                "{}: piece with {} shapes, first kept",
                c.ctx(j),
                cols.len()
            ));
        }
        let ctx = c.ctx(j);
        out.push(PieceDef {
            id: o.id.clone(),
            pos: box_inv.apply(wp),
            angle: pa - box_angle,
            collider,
            body: body_def(rb, &cols, density, r, &ctx),
        });
    }
    out
}

fn convert_joint(c: &Conv, k: usize, j: &Joint, r: &mut Report) -> Option<JointDef> {
    let ctx = c.ctx(k);
    if !j.enabled {
        return None;
    }
    let Some(&body_a) = c.prop_of.get(&k) else {
        r.warn(format!("{ctx}: {} on an object that is not a prop", j.kind));
        return None;
    };
    let fl = &j.fields;
    let kind = match j.kind.as_str() {
        "FixedJoint2D" => JointKind::Fixed,
        "HingeJoint2D" => JointKind::Hinge {
            limits: b(fl, "m_UseLimits", false).then(|| {
                let l = fl.get("m_AngleLimits").cloned().unwrap_or(Value::Null);
                (f(&l, "m_LowerAngle", 0.0), f(&l, "m_UpperAngle", 0.0))
            }),
        },
        "SpringJoint2D" => JointKind::Spring {
            frequency: f(fl, "m_Frequency", 1.0),
            damping_ratio: f(fl, "m_DampingRatio", 0.0),
            distance: (!b(fl, "m_AutoConfigureDistance", true)).then(|| f(fl, "m_Distance", 1.0)),
        },
        "DistanceJoint2D" => JointKind::Distance {
            distance: (!b(fl, "m_AutoConfigureDistance", true)).then(|| f(fl, "m_Distance", 1.0)),
            max_only: b(fl, "m_MaxDistanceOnly", false),
        },
        other => {
            r.unsupported(other);
            r.warn(format!("{ctx}: unsupported joint {other}"));
            return None;
        }
    };
    if j.break_force.is_some() {
        r.unsupported("Joint2D.breakForce");
        r.warn(format!(
            "{ctx}: breakable {} treated as unbreakable",
            j.kind
        ));
    }
    let wa = &c.world[k];
    let pa = pose(wa.t, angle_of(&c.objs[k])).inverse();
    let anchor_w = wa.apply(v2(j.anchor[0], j.anchor[1]));
    let auto_conn = b(fl, "m_AutoConfigureConnectedAnchor", false);
    let conn = v2(j.connected_anchor[0], j.connected_anchor[1]);
    let (body_b, anchor_b) = match j.connected_object {
        Some(bo) => {
            let Some(&bi) = c.prop_of.get(&bo) else {
                r.warn(format!(
                    "{ctx}: joint connected to a non-prop body; attached to the world"
                ));
                return Some(JointDef {
                    body_a,
                    body_b: None,
                    kind,
                    anchor_a: pa.apply(anchor_w),
                    anchor_b: anchor_w,
                    collide_connected: j.enable_collision,
                });
            };
            let wb = &c.world[bo];
            let pb = pose(wb.t, angle_of(&c.objs[bo])).inverse();
            let pt = if auto_conn { anchor_w } else { wb.apply(conn) };
            (Some(bi), pb.apply(pt))
        }
        None => {
            if j.connected_body.is_some() {
                r.warn(format!(
                    "{ctx}: joint's connected body is outside the scene; world used"
                ));
            }
            (None, if auto_conn { anchor_w } else { conn })
        }
    };
    Some(JointDef {
        body_a,
        body_b,
        kind,
        anchor_a: pa.apply(anchor_w),
        anchor_b,
        collide_connected: j.enable_collision,
    })
}

fn convert_anim(c: &Conv, k: usize, r: &mut Report) -> Option<AnimatedDef> {
    let o = &c.objs[k];
    let ctx = c.ctx(k);
    let ca = comp(o, "CurveAnimation")?;
    let anims = ca.get("animations").and_then(Value::as_array)?;
    let a = anims.first()?;
    if anims.len() > 1 {
        r.warn(format!(
            "{ctx}: CurveAnimation with {} animations, first used",
            anims.len()
        ));
    }
    let (ax, ay, az) = (b(ca, "X", false), b(ca, "Y", false), b(ca, "Z", false));
    let pivot = c.world[k].t;
    let angle = angle_of(o);
    let inv = pose(pivot, angle).inverse();
    let mut colliders = vec![];
    let mut kill_box = None;
    for j in c.subtree(k) {
        let oj = &c.objs[j];
        if !oj.active_in_hierarchy || (j != k && c.owner[j].is_some()) {
            continue;
        }
        let local = inv.mul(&c.world[j]);
        for col in oj.colliders.iter().filter(|x| x.enabled) {
            let (shapes, approx) = collider_shapes(col, &local);
            note_approx(r, approx, &c.ctx(j));
            for s in shapes {
                if !matches!(s, Shape::Obb { .. }) {
                    r.unsupported("CurveAnimation collider (non-box)");
                    r.warn(format!("{}: animated non-box collider dropped", c.ctx(j)));
                    continue;
                }
                if col.is_trigger {
                    if let Some(kb) = comp(oj, "KillBox") {
                        if b(kb, "alwaysOn", false) {
                            r.unsupported("KillBox.alwaysOn");
                        }
                        if let Shape::Obb { c: cc, half, .. } = s {
                            kill_box = Some(KillBoxDef {
                                center: cc,
                                half,
                                toggle_time: f(kb, "toggleTime", 0.3),
                                activate_at: f(a, "delay", 0.0),
                                damage: 1000.0,
                            });
                        }
                    } else {
                        r.unsupported("CurveAnimation trigger collider");
                    }
                } else {
                    colliders.push(collider_def(col, s, oj.layer));
                }
            }
        }
    }
    if colliders.is_empty() && kill_box.is_none() {
        r.ignored("CurveAnimation (decoration)");
        return None;
    }
    if ax || az || !ay {
        r.unsupported("CurveAnimation (non-Y axes)");
        r.warn(format!(
            "{ctx}: CurveAnimation axes X={ax} Y={ay} Z={az}; only Y is simulated"
        ));
    }
    if i(a, "animationType", 0) != 0 {
        r.unsupported("CurveAnimation (non-scale type)");
    }
    Some(AnimatedDef {
        id: o.id.clone(),
        name: o.name.clone(),
        pivot,
        angle,
        curve: field_curve(a, "inCurve").unwrap_or_default(),
        speed: f(a, "speed", 1.0),
        start_delay: anim_start(o).unwrap_or(0.0),
        colliders,
        kill_box,
    })
}

/// Arena bounds from a map's camera size and the 16:9 aspect (maps.md 8.1).
pub fn bounds_half(camera_size: f32) -> V2 {
    v2(camera_size * 16.0 / 9.0, camera_size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rounds_import::format::{
        Collider, Component, Joint, Map, Object, PhysicsMaterial, Rigidbody, Shape as UShape,
        SpawnPoint, Trs, WorldTransform,
    };
    use serde_json::json;

    fn trs(pos: [f32; 2], deg: f32, scale: [f32; 2]) -> Trs {
        let h = deg.to_radians() * 0.5;
        Trs {
            position: [pos[0], pos[1], 0.0],
            rotation: [0.0, 0.0, h.sin(), h.cos()],
            scale: [scale[0], scale[1], 1.0],
        }
    }

    /// A synthetic scene builder: objects get world transforms composed
    /// like the importer does.
    struct Scene {
        objs: Vec<Object>,
    }

    impl Scene {
        fn new() -> Scene {
            let mut s = Scene { objs: vec![] };
            s.add(None, "Map", trs([0.0, 0.0], 0.0, [1.0, 1.0]));
            s.objs[0]
                .components
                .push(comp("Map", json!({"size": 20.0})));
            s
        }
        fn add(&mut self, parent: Option<usize>, name: &str, local: Trs) -> usize {
            let k = self.objs.len();
            let active_parent = parent.is_none_or(|p| self.objs[p].active_in_hierarchy);
            self.objs.push(Object {
                id: format!("t:{}", k + 1),
                name: name.into(),
                parent,
                children: vec![],
                active: true,
                active_in_hierarchy: active_parent,
                layer: 0,
                transform: None,
                tag: "Untagged".into(),
                local,
                world: WorldTransform::default(),
                rect: None,
                sprite: None,
                colliders: vec![],
                rigidbody: None,
                joints: vec![],
                particles: None,
                components: vec![],
            });
            if let Some(p) = parent {
                self.objs[p].children.push(k);
            }
            let w = crate::xform::compose_world(&self.objs)[k].to_affine();
            self.objs[k].world.matrix = [w.ax.x, w.ax.y, w.ay.x, w.ay.y, w.t.x, w.t.y];
            self.objs[k].world.rotation_z = w.ax.y.atan2(w.ax.x).to_degrees();
            k
        }
        fn set_inactive(&mut self, k: usize) {
            self.objs[k].active = false;
            let mut stack = vec![k];
            while let Some(i) = stack.pop() {
                self.objs[i].active_in_hierarchy = false;
                stack.extend(self.objs[i].children.iter().copied());
            }
        }
        fn map(self) -> Map {
            Map {
                build_index: 2,
                name: "Synthetic".into(),
                map: Some(json!({"size": 20.0})),
                spawn_points: vec![SpawnPoint {
                    object: 0,
                    id: 0,
                    team: 0,
                    position: [1.0, 2.0],
                }],
                objects: self.objs,
            }
        }
    }

    fn comp(script: &str, fields: Value) -> Component {
        Component {
            id: format!("c:{script}"),
            kind: "MonoBehaviour".into(),
            script: Some(script.into()),
            enabled: Some(true),
            fields,
        }
    }

    fn unit_box(material: bool) -> Collider {
        Collider {
            id: "col".into(),
            enabled: true,
            is_trigger: false,
            offset: [0.0, 0.0],
            density: 1.0,
            material: material.then(|| PhysicsMaterial {
                name: "Box".into(),
                friction: 0.5,
                bounciness: 0.4,
            }),
            shape: UShape::Box {
                size: [1.0, 1.0],
                edge_radius: 0.0,
            },
        }
    }

    fn body(mass: f32) -> Rigidbody {
        Rigidbody {
            id: "rb".into(),
            body_type: 0,
            simulated: true,
            use_auto_mass: false,
            mass,
            linear_drag: 0.2,
            angular_drag: 0.2,
            gravity_scale: 1.0,
            constraints: 4,
            interpolate: 0,
            collision_detection: 0,
            sleeping_mode: 1,
        }
    }

    fn close(a: V2, b: V2) -> bool {
        (a - b).len() < 1e-4
    }

    #[test]
    fn statics_inactive_and_world_space() {
        let mut s = Scene::new();
        let g = s.add(Some(0), "Group", trs([10.0, 0.0], 90.0, [1.0, 1.0]));
        let k = s.add(Some(g), "Ground", trs([2.0, 0.0], 0.0, [4.0, 1.0]));
        s.objs[k].colliders.push(unit_box(false));
        let off = s.add(Some(0), "Off", trs([0.0, 0.0], 0.0, [1.0, 1.0]));
        let hidden = s.add(Some(off), "Hidden", trs([0.0, 0.0], 0.0, [1.0, 1.0]));
        s.objs[hidden].colliders.push(unit_box(false));
        s.set_inactive(off);
        let mut r = Report::default();
        let m = convert_map(&s.map(), &mut r);
        assert_eq!(m.statics.len(), 1, "inactive subtree dropped");
        let st = &m.statics[0];
        assert_eq!(st.id, "t:3");
        assert_eq!((st.collider.friction, st.collider.bounciness), (0.4, 0.0));
        let Shape::Obb { c, axis, half } = &st.collider.shape else {
            panic!()
        };
        // Child at local (2,0) under a parent rotated 90 at (10,0): (10,2).
        assert!(
            close(*c, v2(10.0, 2.0)) && close(*axis, v2(0.0, 1.0)) && close(*half, v2(2.0, 0.5))
        );
        assert_eq!(m.spawn_points.len(), 1);
        assert_eq!(m.camera_size, 20.0);
    }

    #[test]
    fn prop_with_child_collider_destructible_and_saw() {
        let mut s = Scene::new();
        let p = s.add(Some(0), "Box", trs([5.0, 5.0], 90.0, [2.0, 2.0]));
        s.objs[p].layer = 17;
        s.objs[p].rigidbody = Some(body(20000.0));
        s.objs[p].colliders.push(unit_box(true));
        s.objs[p].components.push(comp(
            "NetworkPhysicsObject",
            json!({"speed": 8000.0, "playerColThreshold": 5.0, "dmgAmount": 0.5,
                   "forceAmount": 1500.0, "bulletPushMultiplier": 20.0}),
        ));
        s.objs[p]
            .components
            .push(comp("DamagableEvent", json!({"maxHP": 100.0})));
        s.objs[p]
            .components
            .push(comp("DestructibleBoxDestruction", json!({})));
        // A collider on a child without a body belongs to the prop.
        let child = s.add(Some(p), "Extra", trs([0.5, 0.0], 0.0, [0.5, 0.5]));
        s.objs[child].layer = 17;
        s.objs[child].colliders.push(unit_box(true));
        // Hidden pieces with their own bodies.
        let pieces = s.add(Some(p), "Pieces", trs([0.0, 0.0], 0.0, [1.0, 1.0]));
        let piece = s.add(Some(pieces), "BoxPiece", trs([0.25, 0.0], 0.0, [0.5, 0.25]));
        s.objs[piece].rigidbody = Some(body(1000.0));
        s.objs[piece].colliders.push(unit_box(true));
        s.set_inactive(pieces);
        // A saw welded to the world.
        let saw = s.add(Some(0), "Saw", trs([-3.0, 0.0], 0.0, [1.0, 1.0]));
        s.objs[saw].layer = 21;
        s.objs[saw].rigidbody = Some(body(100000.0));
        let mut c = unit_box(false);
        c.shape = UShape::Circle { radius: 1.15 };
        s.objs[saw].colliders.push(c);
        s.objs[saw].components.push(comp(
            "DamageBox",
            json!({"damage": 55.0, "force": 40000.0, "cd": 0.2, "setFlyingFor": 0.5, "shake": 5.0}),
        ));
        let blade = s.add(Some(saw), "Sprite", trs([0.0, 0.0], 0.0, [1.0, 1.0]));
        s.objs[blade]
            .components
            .push(comp("Rotate", json!({"speed": 400.0})));
        s.objs[saw].joints.push(Joint {
            id: "j".into(),
            kind: "FixedJoint2D".into(),
            enabled: true,
            connected_body: None,
            connected_object: None,
            anchor: [0.0, 0.0],
            connected_anchor: [-3.0, 0.0],
            enable_collision: true,
            break_force: None,
            fields: json!({"m_AutoConfigureConnectedAnchor": true}),
        });
        let mut r = Report::default();
        let m = convert_map(&s.map(), &mut r);
        assert!(m.statics.is_empty());
        assert_eq!(m.props.len(), 2, "pieces are not props");
        let pr = &m.props[0];
        assert!(close(pr.pos, v2(5.0, 5.0)));
        assert!((pr.angle - 90f32.to_radians()).abs() < 1e-5);
        assert_eq!(pr.colliders.len(), 2);
        // Body frame has no scale: the 2x2 box is a 1x1 half-extent box at 0.
        let Shape::Obb { c, half, .. } = &pr.colliders[0].shape else {
            panic!()
        };
        assert!(close(*c, V2::ZERO) && close(*half, v2(1.0, 1.0)));
        // Child at local (0.5, 0) times scale 2 = (1, 0) in the body frame.
        let Shape::Obb { c, half, .. } = &pr.colliders[1].shape else {
            panic!()
        };
        assert!(close(*c, v2(1.0, 0.0)) && close(*half, v2(0.5, 0.5)));
        assert_eq!(pr.colliders[0].layer, 17);
        assert_eq!(pr.colliders[0].friction, 0.5);
        assert!(pr.body.freeze_rotation && pr.body.mass == 20000.0);
        assert!(pr.pushable.is_some());
        let d = pr.destructible.as_ref().unwrap();
        assert_eq!(d.hp, 100.0);
        assert_eq!(d.pieces.len(), 1);
        assert!(
            close(d.pieces[0].pos, v2(0.5, 0.0)),
            "{:?}",
            d.pieces[0].pos
        );
        assert_eq!(d.pieces[0].body.mass, 1000.0);
        let saw = &m.props[1];
        let db = saw.damage_box.unwrap();
        assert_eq!((db.damage, db.rotate_speed), (55.0, 400.0));
        assert_eq!(m.joints.len(), 1);
        assert_eq!(m.joints[0].body_a, 1);
        assert_eq!(m.joints[0].body_b, None);
        assert!(matches!(m.joints[0].kind, JointKind::Fixed));
        assert!(close(m.joints[0].anchor_b, v2(-3.0, 0.0)));
    }

    #[test]
    fn ropes_moving_platforms_and_smashers() {
        let mut s = Scene::new();
        let rope = s.add(Some(0), "Rope_Chain", trs([0.0, 10.0], 0.0, [1.0, 1.0]));
        s.objs[rope]
            .components
            .push(comp("MapObjet_Rope", json!({"jointType": 1})));
        s.add(Some(rope), "Rope_End", trs([0.0, -4.0], 0.0, [1.0, 1.0]));
        let plat = s.add(Some(0), "MovingStone", trs([3.0, 0.0], 0.0, [6.0, 1.0]));
        let mut rb = body(100000.0);
        rb.constraints = 0;
        rb.gravity_scale = 0.0;
        s.objs[plat].rigidbody = Some(rb);
        s.objs[plat].colliders.push(unit_box(true));
        s.objs[plat].components.push(comp(
            "MoveSequence",
            json!({"positions": [{"x": 0.0, "y": 0.0}, {"x": 0.0, "y": 10.0}],
                   "spring": 300.0, "timeAtPos": 2.0, "cap": 5.0, "threshold": 3.0, "drag": 1.0}),
        ));
        s.objs[plat]
            .components
            .push(comp("RigLookUp", json!({"force": 3.0})));
        let smasher = s.add(Some(0), "Anim", trs([0.0, 5.0], 0.0, [1.0, 1.0]));
        s.objs[smasher].components.push(comp(
            "CurveAnimation",
            json!({"X": 0, "Y": 1, "Z": 0, "animations": [{
                "playOnAwake": 1, "speed": 0.2, "delay": 0.338, "animationType": 0,
                "inCurve": {"m_Curve": [
                    {"time": 0.0, "value": 1.0, "inSlope": 0.0, "outSlope": 0.0},
                    {"time": 1.0, "value": 8.0, "inSlope": 0.0, "outSlope": 0.0}]}}]}),
        ));
        let blk = s.add(Some(smasher), "Box", trs([0.0, -0.5], 0.0, [2.0, 1.0]));
        s.objs[blk].colliders.push(unit_box(false));
        let kill = s.add(
            Some(smasher),
            "Kill",
            trs([0.0, -0.859], 0.0, [1.762, 0.307]),
        );
        let mut kc = unit_box(false);
        kc.is_trigger = true;
        s.objs[kill].colliders.push(kc);
        s.objs[kill]
            .components
            .push(comp("KillBox", json!({"toggleTime": 0.3})));
        let mut r = Report::default();
        let m = convert_map(&s.map(), &mut r);
        assert_eq!(m.ropes.len(), 1);
        assert!(close(m.ropes[0].start, v2(0.0, 10.0)) && close(m.ropes[0].end, v2(0.0, 6.0)));
        assert_eq!(m.ropes[0].kind, RopeKind::Distance);
        let ms = m.props[0].move_sequence.as_ref().unwrap();
        assert_eq!(ms.positions, vec![v2(0.0, 0.0), v2(0.0, 10.0)]);
        assert_eq!(m.props[0].rig_look_up, Some(3.0));
        assert!(
            m.statics.is_empty(),
            "smasher colliders are animated, not static"
        );
        assert_eq!(m.animated.len(), 1);
        let a = &m.animated[0];
        assert!(close(a.pivot, v2(0.0, 5.0)));
        assert_eq!(a.colliders.len(), 1);
        let kb = a.kill_box.unwrap();
        assert!(close(kb.center, v2(0.0, -0.859)) && close(kb.half, v2(0.881, 0.1535)));
        assert_eq!((kb.toggle_time, kb.activate_at, a.speed), (0.3, 0.338, 0.2));
        assert_eq!(a.curve.keys.len(), 2);
        assert!(r.unsupported.is_empty(), "{:?}", r.unsupported);
    }
}
