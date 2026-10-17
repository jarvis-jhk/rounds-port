//! Player prefab defaults (docs/spec/player.md) and the comparison of the
//! sim's hard-coded tuning with the serialized values.
//!
//! The sim has no content slot for player parameters yet; it uses literal
//! constants derived from the prefab (player.md quotes them). [`compare`]
//! lists every such value next to the imported one so drift is visible.

use rounds_import::GameData;
use rounds_import::format::{Object, Prefab};
use rounds_sim::content::{DamageBoxDef, PushableDef, Tuning};
use rounds_sim::geom::layers;
use rounds_sim::math::{Curve, V2, v2};
use rounds_sim::stats::{AmmoStats, BlockStats, GunStats, PlayerStats};
use serde_json::Value;

use crate::fields::{b, f, field_curve, field_ref, i, num};
use crate::maps::script;
use crate::report::{Report, TuningDiff};
use crate::xform::compose_world;

/// Serialized player parameters.
#[derive(Clone, Debug, Default)]
pub struct PlayerDefaults {
    /// Player prefab id.
    pub prefab: String,
    /// Root `CircleCollider2D` radius (local units).
    pub radius: f32,
    /// `ObjectCollider` circle radius (layer 19).
    pub object_collider_radius: f32,
    /// Root local scale in the prefab.
    pub root_scale: f32,
    pub health: f32,
    pub jumps: i32,
    pub since_grounded_mult_wall_grab: f32,
    /// `PlayerMovement`.
    pub movement_force: f32,
    pub air_control: f32,
    pub extra_drag: f32,
    /// `Gravity`.
    pub gravity_force: f32,
    pub gravity_exponent: f32,
    /// `PlayerJump`.
    pub jump_up_force: f32,
    pub jump_side_force: f32,
    /// `LegRaycasters`.
    pub leg_force: f32,
    pub leg_drag: f32,
    pub leg_support_curve: Curve,
    pub leg_bob_curve: Curve,
    pub leg_mask: u32,
    /// Leg cast points relative to the root at prefab scale (world units).
    pub leg_offsets: Vec<V2>,
    /// `PlayerCollision`.
    pub collision_mask: u32,
    pub bounce_threshold: f32,
    /// `Holding` (gun spring).
    pub holding_force: f32,
    pub holding_drag: f32,
    /// `Block` on the player.
    pub block: BlockStats,
    /// The held gun (`WeaponBase`): `Gun` stats and the magazine (`GunAmmo`).
    pub gun: GunStats,
    pub ammo: AmmoStats,
    /// Name of the gun's first projectile prefab (the default bullet).
    pub bullet: Option<String>,
    /// Its prefab id.
    pub bullet_id: Option<String>,
    /// Gun rigidbody (mass, gravity scale, linear/angular drag).
    pub gun_body: Option<[f32; 4]>,
    /// `WeaponHandler` overheat parameters (inert in the original).
    pub heat_per_bullet: f32,
    /// Main scene: `PlayerManager.playerMoveCurve`, `canSeePlayerMask`.
    pub spawn_curve: Curve,
    pub can_see_player_mask: u32,
    /// Main scene: `TimeHandler`.
    pub base_time_scale: f32,
    pub slow_down_curve: Curve,
    pub speed_up_curve: Curve,
    /// Main scene: `MapTransition` slide and gravity curves.
    pub map_slide_curve: Curve,
    pub map_gravity_curve: Curve,
    /// Main scene: `PlayerAssigner.maxPlayers`.
    pub max_players: i64,
}

fn comp<'a>(o: &'a Object, name: &str) -> Option<&'a Value> {
    o.components
        .iter()
        .find(|c| script(c) == name)
        .map(|c| &c.fields)
}

fn find<'a>(p: &'a Prefab, name: &str) -> Option<&'a Value> {
    p.objects.iter().find_map(|o| comp(o, name))
}

fn mask(v: &Value, key: &str) -> u32 {
    v.get(key)
        .and_then(|m| m.get("m_Bits"))
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32
}

pub fn read(data: &GameData, r: &mut Report) -> PlayerDefaults {
    let mut d = PlayerDefaults::default();
    let Some(pid) = data.player.prefabs.first() else {
        r.warn("no player prefab in the import");
        return d;
    };
    let p = match data.prefab(pid) {
        Ok(p) => p,
        Err(e) => {
            r.warn(format!("player prefab: {e}"));
            return d;
        }
    };
    d.prefab = pid.clone();
    let root = &p.objects[0];
    let null = Value::Null;
    d.root_scale = root.local.scale[0];
    d.radius = root
        .colliders
        .iter()
        .find_map(|c| match c.shape {
            rounds_import::format::Shape::Circle { radius } => Some(radius),
            _ => None,
        })
        .unwrap_or(0.0);
    d.object_collider_radius = p
        .objects
        .iter()
        .find(|o| o.name == "ObjectCollider")
        .and_then(|o| {
            o.colliders.iter().find_map(|c| match c.shape {
                rounds_import::format::Shape::Circle { radius } => Some(radius),
                _ => None,
            })
        })
        .unwrap_or(0.0);
    let cd = comp(root, "CharacterData").unwrap_or(&null);
    d.health = f(cd, "health", 0.0);
    d.jumps = i(cd, "jumps", 0) as i32;
    d.since_grounded_mult_wall_grab = f(cd, "sinceGroundedMultiplierWhenWallGrab", 0.0);
    let pm = comp(root, "PlayerMovement").unwrap_or(&null);
    d.movement_force = f(pm, "force", 0.0);
    d.air_control = f(pm, "airControl", 0.0);
    d.extra_drag = f(pm, "extraDrag", 0.0);
    let g = comp(root, "Gravity").unwrap_or(&null);
    d.gravity_force = f(g, "gravityForce", 0.0);
    d.gravity_exponent = f(g, "exponent", 0.0);
    let pj = comp(root, "PlayerJump").unwrap_or(&null);
    d.jump_up_force = f(pj, "upForce", 0.0);
    d.jump_side_force = f(pj, "sideForce", 0.0);
    let pc = comp(root, "PlayerCollision").unwrap_or(&null);
    d.collision_mask = mask(pc, "mask");
    d.bounce_threshold = f(pc, "bounceTreshold", 0.0);
    let h = comp(root, "Holding").unwrap_or(&null);
    d.holding_force = f(h, "force", 0.0);
    d.holding_drag = f(h, "drag", 0.0);
    let wh = comp(root, "WeaponHandler").unwrap_or(&null);
    d.heat_per_bullet = f(wh, "heatPerBullet", 0.0);
    let bl = comp(root, "Block").unwrap_or(&null);
    d.block = BlockStats {
        cooldown: f(bl, "cooldown", 0.0),
        cd_multiplier: f(bl, "cdMultiplier", 1.0),
        cd_add: f(bl, "cdAdd", 0.0),
        force_to_add: f(bl, "forceToAdd", 0.0),
        force_to_add_up: f(bl, "forceToAddUp", 0.0),
        additional_blocks: i(bl, "additionalBlocks", 0) as i32,
        healing: f(bl, "healing", 0.0),
        auto_block: b(bl, "autoBlock", false),
        delay_other_actions: b(bl, "delayOtherActions", false),
    };
    if let Some(lr) = find(&p, "LegRaycasters") {
        d.leg_force = f(lr, "force", 0.0);
        d.leg_drag = f(lr, "drag", 0.0);
        d.leg_support_curve = field_curve(lr, "animationCurve").unwrap_or_default();
        d.leg_bob_curve = field_curve(lr, "forceCurve").unwrap_or_default();
        d.leg_mask = mask(lr, "mask");
        let world = compose_world(&p.objects);
        let rootw = world[0].to_affine();
        if let Some(Value::Array(list)) = lr.get("legCastPositions") {
            for e in list {
                let Some(id) = crate::fields::ref_id(e) else {
                    continue;
                };
                // Transform references resolve through `Object::transform`.
                match p
                    .objects
                    .iter()
                    .position(|o| o.transform.as_deref() == Some(id) || o.id == id)
                {
                    Some(k) => d.leg_offsets.push(world[k].to_affine().t - rootw.t),
                    None => r.warn(format!(
                        "player: legCastPositions entry {id} not found (re-import for Transform ids)"
                    )),
                }
            }
        }
    }
    // The held gun: Holding.holdable -> the WeaponBase prefab.
    let gun_prefab = field_ref(h, "holdable").and_then(|id| {
        let mut files: Vec<_> = std::fs::read_dir(data.dir.join("prefabs"))
            .ok()?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .collect();
        files.sort();
        files.into_iter().find_map(|f| {
            let pf: Prefab = serde_json::from_slice(&std::fs::read(&f).ok()?).ok()?;
            pf.objects
                .iter()
                .any(|o| o.id == id || o.components.iter().any(|c| c.id == id))
                .then_some(pf)
        })
    });
    match gun_prefab {
        Some(gp) => {
            if let Some(gun) = find(&gp, "Gun") {
                d.gun = serde_json::from_value(gun.clone()).unwrap_or_else(|e| {
                    r.warn(format!("player gun: {e}"));
                    GunStats::player_reset()
                });
                d.bullet_id = gun
                    .get("projectiles")
                    .and_then(Value::as_array)
                    .and_then(|a| a.first())
                    .and_then(|e| field_ref(e, "objectToSpawn"))
                    .map(String::from);
                d.bullet = d
                    .bullet_id
                    .as_deref()
                    .and_then(|id| data.prefab(id).ok())
                    .map(|bp| bp.name);
            }
            if let Some(a) = find(&gp, "GunAmmo") {
                d.ammo = serde_json::from_value(a.clone()).unwrap_or_default();
            }
            d.gun_body = gp.objects[0]
                .rigidbody
                .as_ref()
                .map(|rb| [rb.mass, rb.gravity_scale, rb.linear_drag, rb.angular_drag]);
        }
        None => r.warn("player's gun prefab (Holding.holdable) not found"),
    }
    if let Some(main) = &data.main {
        for o in &main.objects {
            if let Some(pmg) = comp(o, "PlayerManager") {
                d.spawn_curve = field_curve(pmg, "playerMoveCurve").unwrap_or_default();
                d.can_see_player_mask = mask(pmg, "canSeePlayerMask");
            }
            if let Some(th) = comp(o, "TimeHandler") {
                d.base_time_scale = f(th, "baseTimeScale", 0.0);
                d.slow_down_curve = field_curve(th, "slowDown").unwrap_or_default();
                d.speed_up_curve = field_curve(th, "speedUp").unwrap_or_default();
            }
            if let Some(mt) = comp(o, "MapTransition") {
                d.map_slide_curve = field_curve(mt, "curve").unwrap_or_default();
                d.map_gravity_curve = field_curve(mt, "gravityCurve").unwrap_or_default();
            }
            if let Some(pa) = comp(o, "PlayerAssigner") {
                d.max_players = i(pa, "maxPlayers", 0);
            }
        }
    } else {
        r.warn("main.json missing: main-scene tuning not read");
    }
    r.count("player prefab", 1);
    d
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() <= 1e-4 * (1.0 + a.abs().max(b.abs()))
}

struct Cmp(Vec<TuningDiff>);

impl Cmp {
    fn num(&mut self, name: &str, imported: f32, sim: f32, src: &str) {
        self.0.push(TuningDiff {
            name: name.into(),
            imported: format!("{imported}"),
            sim: format!("{sim}"),
            sim_source: src.into(),
            matches: close(imported, sim),
        });
    }
    fn bits(&mut self, name: &str, imported: u32, sim: u32, src: &str) {
        self.0.push(TuningDiff {
            name: name.into(),
            imported: format!("{imported:#x}"),
            sim: format!("{sim:#x}"),
            sim_source: src.into(),
            matches: imported == sim,
        });
    }
    fn curve(&mut self, name: &str, imported: &Curve, sim: &[(f32, f32, f32, f32)], src: &str) {
        let ok = imported.keys.len() == sim.len()
            && imported.keys.iter().zip(sim).all(|(k, s)| {
                (k.time - s.0).abs() < 2e-3
                    && (k.value - s.1).abs() < 2e-3 * (1.0 + s.1.abs())
                    && (k.in_slope - s.2).abs() < 2e-3 * (1.0 + s.2.abs())
                    && (k.out_slope - s.3).abs() < 2e-3 * (1.0 + s.3.abs())
            });
        let fmt = |keys: Vec<(f32, f32)>| {
            keys.iter()
                .map(|(t, v)| format!("({t:.3},{v:.3})"))
                .collect::<Vec<_>>()
                .join("")
        };
        self.0.push(TuningDiff {
            name: name.into(),
            imported: fmt(imported.keys.iter().map(|k| (k.time, k.value)).collect()),
            sim: fmt(sim.iter().map(|s| (s.0, s.1)).collect()),
            sim_source: src.into(),
            matches: ok,
        });
    }
}

/// Imported values vs. the sim's current constants. Values the sim keeps in
/// private literals are quoted from its source (`player.rs`, `world.rs`,
/// `map.rs`) and must be updated here if the sim changes them.
#[allow(clippy::approx_constant)] // curve slopes from the data, not 1/pi
pub fn compare(p: &PlayerDefaults, data: &GameData, tuning: &Tuning) -> Vec<TuningDiff> {
    let mut c = Cmp(vec![]);
    let st = PlayerStats::default();
    let gun = GunStats::player_reset();
    let ammo = AmmoStats::default();
    let blk = BlockStats::default();
    c.num(
        "fixed timestep",
        data.index.settings.fixed_timestep,
        rounds_sim::DT,
        "world::DT",
    );
    c.num(
        "Physics2D gravity y",
        data.index.settings.gravity[1],
        rounds_sim::physics::GRAVITY,
        "physics::GRAVITY",
    );
    c.num(
        "TimeHandler.baseTimeScale",
        p.base_time_scale,
        rounds_sim::world::BASE_TIME_SCALE,
        "world::BASE_TIME_SCALE",
    );
    c.num(
        "player CircleCollider2D radius",
        p.radius,
        rounds_sim::player::RADIUS,
        "player::RADIUS",
    );
    c.num(
        "ObjectCollider radius",
        p.object_collider_radius,
        0.15,
        "physics.rs literal",
    );
    c.num(
        "player root scale",
        p.root_scale,
        1.2,
        "Player::new literal",
    );
    c.num(
        "CharacterData.health",
        p.health,
        st.max_health,
        "PlayerStats::default",
    );
    c.num(
        "CharacterData.jumps",
        p.jumps as f32,
        st.jumps as f32,
        "PlayerStats::default",
    );
    c.num(
        "sinceGroundedMultiplierWhenWallGrab",
        p.since_grounded_mult_wall_grab,
        0.5,
        "player.rs literal",
    );
    c.num(
        "PlayerMovement.force*0.01*0.02",
        p.movement_force * 0.01 * 0.02,
        3.4,
        "player.rs literal 3.4",
    );
    c.num(
        "PlayerMovement.airControl",
        p.air_control,
        0.8,
        "player.rs literal",
    );
    c.num(
        "PlayerMovement.extraDrag*0.001",
        p.extra_drag * 0.001,
        0.2,
        "player.rs literal 0.2",
    );
    c.num(
        "Gravity.gravityForce",
        p.gravity_force,
        st.gravity_force,
        "PlayerStats::default",
    );
    c.num(
        "Gravity.exponent",
        p.gravity_exponent,
        1.5,
        "player.rs literal",
    );
    c.num(
        "PlayerJump.upForce",
        p.jump_up_force,
        5500.0,
        "player.rs literal",
    );
    c.num(
        "PlayerJump.sideForce",
        p.jump_side_force,
        1.0,
        "player.rs literal",
    );
    c.num(
        "LegRaycasters.force",
        p.leg_force,
        1000.0,
        "player.rs literal (0.02*1000)",
    );
    c.num(
        "LegRaycasters.drag",
        p.leg_drag,
        100.0,
        "player.rs literal (0.02*100)",
    );
    c.curve(
        "LegRaycasters.animationCurve (support)",
        &p.leg_support_curve,
        &[
            (-0.1837, 0.1584, -0.2704, -0.2704),
            (0.402, 0.0, -0.2704, -0.2704),
        ],
        "player.rs support_curve (linear)",
    );
    c.curve(
        "LegRaycasters.forceCurve (walk bob)",
        &p.leg_bob_curve,
        &[
            (0.0, -99.65, -1.5805, -1.5805),
            (0.5, 70.54, 0.0, 0.0),
            (1.0, -99.65, 1.0155, 1.0155),
        ],
        "player.rs BOB_CURVE",
    );
    for (k, o) in p.leg_offsets.iter().enumerate() {
        let sx = if o.x < 0.0 { -1.0 } else { 1.0 };
        c.num(
            &format!("leg {k} offset x"),
            o.x,
            0.24 * sx,
            "player.rs literal (+-0.24)",
        );
        c.num(
            &format!("leg {k} offset y"),
            o.y,
            -0.36,
            "player.rs literal (-0.36)",
        );
    }
    c.bits(
        "LegRaycasters.mask",
        p.leg_mask,
        layers::LEG_RAY,
        "geom::layers::LEG_RAY",
    );
    c.bits(
        "PlayerCollision.mask",
        p.collision_mask,
        layers::PLAYER_COLLISION,
        "geom::layers::PLAYER_COLLISION",
    );
    c.bits(
        "PlayerManager.canSeePlayerMask",
        p.can_see_player_mask,
        tuning.line_of_sight_mask,
        "Tuning::line_of_sight_mask (from data)",
    );
    c.num(
        "PlayerCollision.bounceTreshold",
        p.bounce_threshold,
        1.0,
        "player.rs (flying bounce)",
    );
    c.num("Holding.force", p.holding_force, 500.0, "player.rs literal");
    c.num("Holding.drag", p.holding_drag, 25.0, "player.rs literal");
    c.num(
        "Block.cooldown",
        p.block.cooldown,
        blk.cooldown,
        "BlockStats::default",
    );
    c.num(
        "Gun.attackSpeed",
        p.gun.attack_speed,
        gun.attack_speed,
        "GunStats::player_reset",
    );
    c.num(
        "Gun.numberOfProjectiles",
        p.gun.number_of_projectiles as f32,
        gun.number_of_projectiles as f32,
        "GunStats::player_reset",
    );
    c.num(
        "Gun.recoil",
        p.gun.recoil,
        gun.recoil,
        "GunStats::player_reset",
    );
    c.num(
        "Gun.shake",
        p.gun.shake,
        gun.shake,
        "GunStats::player_reset",
    );
    c.num(
        "Gun.bodyRecoil",
        p.gun.body_recoil,
        gun.body_recoil,
        "GunStats::player_reset",
    );
    c.num(
        "Gun.damage",
        p.gun.damage,
        gun.damage,
        "GunStats::player_reset",
    );
    c.num(
        "Gun.projectileSpeed",
        p.gun.projectile_speed,
        gun.projectile_speed,
        "GunStats::player_reset",
    );
    c.num(
        "Gun.dragMinSpeed",
        p.gun.drag_min_speed,
        gun.drag_min_speed,
        "GunStats::player_reset",
    );
    c.num(
        "Gun.defaultCooldown",
        p.gun.default_cooldown,
        gun.default_cooldown,
        "GunStats::player_reset",
    );
    c.num(
        "GunAmmo.maxAmmo",
        p.ammo.max_ammo as f32,
        ammo.max_ammo as f32,
        "AmmoStats::default",
    );
    c.num(
        "GunAmmo.reloadTime",
        p.ammo.reload_time,
        ammo.reload_time,
        "AmmoStats::default",
    );
    if let Some([m, gs, ld, ad]) = p.gun_body {
        c.num(
            "WeaponBase Rigidbody2D mass",
            m,
            5.0,
            "player.md 1.2 (gun body)",
        );
        c.num(
            "WeaponBase Rigidbody2D gravityScale",
            gs,
            0.0,
            "player.md 1.2",
        );
        c.num(
            "WeaponBase Rigidbody2D linearDrag",
            ld,
            0.0,
            "player.md 1.2",
        );
        c.num(
            "WeaponBase Rigidbody2D angularDrag",
            ad,
            0.05,
            "player.md 1.2",
        );
    }
    c.curve(
        "PlayerManager.playerMoveCurve",
        &p.spawn_curve,
        &[
            (0.0, 0.0, 0.0, 0.0),
            (0.39, 0.4839, 5.912, 5.912),
            (0.8, 1.0, 0.0, 0.0),
            (0.9, 1.0, 0.0, 0.0),
        ],
        "player.rs SPAWN_CURVE",
    );
    c.curve(
        "TimeHandler.slowDown",
        &p.slow_down_curve,
        &[
            (0.0, 1.0, -0.318, -0.318),
            (0.065, 0.730, -14.30, -14.30),
            (0.1056, 0.264, -4.385, -4.385),
            (0.2, 0.15, 0.0, 0.0),
        ],
        "world.rs SLOW_DOWN",
    );
    c.curve(
        "TimeHandler.speedUp",
        &p.speed_up_curve,
        &[(0.0, 0.025, 0.0, 0.0), (0.1, 1.0, 0.0, 0.0)],
        "world.rs SPEED_UP",
    );
    // Map-level data vs. sim defaults.
    let size = data
        .maps
        .first()
        .and_then(|m| m.map.as_ref())
        .and_then(|v| num(v, "size"))
        .unwrap_or(20.0);
    c.num(
        "Map.size * 16/9 (bounds x)",
        size * 16.0 / 9.0,
        tuning.bounds_half.x,
        "Tuning::bounds_half (from data)",
    );
    c.num(
        "Map.size (bounds y)",
        size,
        tuning.bounds_half.y,
        "Tuning::bounds_half (from data)",
    );
    // Prop and saw scripts: first instance in the maps vs. the defaults.
    let first = |name: &str| {
        data.maps
            .iter()
            .flat_map(|m| &m.objects)
            .find_map(|o| comp(o, name).cloned())
    };
    let pd = PushableDef::default();
    if let Some(n) = first("NetworkPhysicsObject") {
        c.num(
            "NetworkPhysicsObject.playerColThreshold",
            f(&n, "playerColThreshold", 0.0),
            pd.player_col_threshold,
            "PushableDef::default",
        );
        c.num(
            "NetworkPhysicsObject.speed",
            f(&n, "speed", 0.0),
            pd.speed,
            "PushableDef::default",
        );
        c.num(
            "NetworkPhysicsObject.forceAmount",
            f(&n, "forceAmount", 0.0),
            pd.force_amount,
            "PushableDef::default",
        );
        c.num(
            "NetworkPhysicsObject.dmgAmount",
            f(&n, "dmgAmount", 0.0),
            pd.dmg_amount,
            "PushableDef::default",
        );
        c.num(
            "NetworkPhysicsObject.bulletPushMultiplier",
            f(&n, "bulletPushMultiplier", 0.0),
            pd.bullet_push_multiplier,
            "PushableDef::default",
        );
        c.num(
            "NetworkPhysicsObject.collisionThreshold",
            f(&n, "collisionThreshold", 0.0),
            200000.0,
            "maps.md 3.3 (sound threshold)",
        );
    }
    let db = DamageBoxDef::default();
    if let Some(d) = first("DamageBox") {
        c.num(
            "DamageBox.damage",
            f(&d, "damage", 0.0),
            db.damage,
            "DamageBoxDef::default",
        );
        c.num(
            "DamageBox.force",
            f(&d, "force", 0.0),
            db.force,
            "DamageBoxDef::default",
        );
        c.num(
            "DamageBox.cd",
            f(&d, "cd", 0.0),
            db.cd,
            "DamageBoxDef::default",
        );
        c.num(
            "DamageBox.setFlyingFor",
            f(&d, "setFlyingFor", 0.0),
            db.flying_for,
            "DamageBoxDef::default",
        );
    }
    if let Some(ca) = first("CurveAnimation") {
        let sounds: Vec<f32> = ca
            .get("soundPlay")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|s| num(s, "soundDelay")).collect())
            .unwrap_or_default();
        if sounds.len() == 2 {
            c.num(
                "smasher sound 1 (curve time)",
                sounds[0],
                0.15,
                "map.rs literal",
            );
            c.num(
                "smasher sound 2 (curve time)",
                sounds[1],
                0.3377,
                "map.rs literal",
            );
        }
    }
    let _ = v2;
    c.0
}
