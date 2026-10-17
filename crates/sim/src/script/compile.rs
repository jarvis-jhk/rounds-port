//! Compiling an [`ObjectDef`]'s components (`EffectDesc`: script name + raw
//! Unity fields) into a [`Prefab`] and projectile add-on lists. Runs once in
//! `Content::new`; unknown or purely visual scripts become
//! [`Kind::Cosmetic`].

use super::addons::AddonDef;
use super::*;
use crate::content::{EffectDesc, ObjectDef, ObjectId, SpawnEntry};
use crate::math::{Key, key};
use serde_json::Value;
use std::collections::BTreeMap;

/// Id lookups shared by all prefabs.
pub struct Ids<'a> {
    pub objects: &'a BTreeMap<String, ObjectId>,
    pub projectiles: &'a BTreeMap<String, u16>,
    /// SoundEvent id -> cue name.
    pub sounds: &'a BTreeMap<String, &'static str>,
}

// ---- raw field helpers ----

pub(crate) fn fnum(p: &Value, k: &str, d: f32) -> f32 {
    p.get(k)
        .and_then(|v| v.as_f64())
        .map(|x| x as f32)
        .unwrap_or(d)
}
pub(crate) fn fbool(p: &Value, k: &str, d: bool) -> bool {
    match p.get(k) {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|x| x != 0.0),
        _ => d,
    }
}
pub(crate) fn fint(p: &Value, k: &str, d: i64) -> i64 {
    match p.get(k) {
        Some(Value::Number(n)) => n.as_i64().or(n.as_f64().map(|x| x as i64)).unwrap_or(d),
        Some(Value::Bool(b)) => *b as i64,
        _ => d,
    }
}
fn refid(v: &Value) -> Option<&str> {
    v.get("$ref")?.as_str()
}
/// An `AnimationCurve`: raw (`{"m_Curve": [{time, value, inSlope,
/// outSlope}]}`) or already converted (`{"keys": [{time, value, in_slope,
/// out_slope}]}`).
pub(crate) fn fcurve(p: &Value, k: &str) -> Curve {
    let Some(c) = p.get(k) else {
        return Curve::default();
    };
    let (arr, si, so) = match (c.get("m_Curve"), c.get("keys")) {
        (Some(a), _) => (a.as_array(), "inSlope", "outSlope"),
        (None, Some(a)) => (a.as_array(), "in_slope", "out_slope"),
        _ => (None, "", ""),
    };
    let keys = arr
        .map(|a| {
            a.iter()
                .map(|kk| {
                    key(
                        fnum(kk, "time", 0.0),
                        fnum(kk, "value", 0.0),
                        fnum(kk, si, 0.0),
                        fnum(kk, so, 0.0),
                    )
                })
                .collect::<Vec<Key>>()
        })
        .unwrap_or_default();
    Curve { keys }
}
fn fmask(p: &Value, k: &str, d: u32) -> u32 {
    p.get(k)
        .and_then(|m| m.get("m_Bits"))
        .and_then(|b| b.as_u64())
        .map(|b| b as u32)
        .unwrap_or(d)
}

/// Persistent calls of a UnityEvent field: (target id, method, bool arg).
/// Accepts the raw form (`{"m_PersistentCalls": {"m_Calls": [{m_Target,
/// m_MethodName, m_Arguments}]}}`) and the adapter's converted form
/// (`{"calls": [{target, method, arg}]}`).
fn raw_calls<'a>(p: &'a Value, k: &str) -> Vec<(&'a str, &'a str, bool)> {
    let Some(ev) = p.get(k) else {
        return vec![];
    };
    if let Some(calls) = ev
        .get("m_PersistentCalls")
        .and_then(|e| e.get("m_Calls"))
        .and_then(|e| e.as_array())
    {
        return calls
            .iter()
            .filter_map(|c| {
                let t = refid(c.get("m_Target")?)?;
                let m = c.get("m_MethodName")?.as_str()?;
                let arg = c
                    .get("m_Arguments")
                    .map(|a| fbool(a, "m_BoolArgument", false))
                    .unwrap_or(false);
                Some((t, m, arg))
            })
            .collect();
    }
    let Some(calls) = ev.get("calls").and_then(|e| e.as_array()) else {
        return vec![];
    };
    calls
        .iter()
        .filter_map(|c| {
            let t = refid(c.get("target")?)?;
            let m = c.get("method")?.as_str()?;
            let arg = match c.get("arg") {
                Some(Value::Bool(b)) => *b,
                Some(Value::Number(n)) => n.as_f64().is_some_and(|x| x != 0.0),
                _ => false,
            };
            Some((t, m, arg))
        })
        .collect()
}

struct Ctx<'a> {
    comp_ids: BTreeMap<&'a str, CompIdx>,
    node_ids: BTreeMap<&'a str, NodeIdx>,
    scripts: Vec<&'a str>,
    ids: &'a Ids<'a>,
}

impl Ctx<'_> {
    fn ev(&self, p: &Value, k: &str) -> Ev {
        raw_calls(p, k)
            .into_iter()
            .filter_map(|(t, m, arg)| self.call(t, m, arg))
            .collect()
    }
    fn call(&self, t: &str, m: &str, arg: bool) -> Option<Call> {
        let target = match self.comp_ids.get(t) {
            Some(c) => Target::Comp(*c),
            None => Target::Node(*self.node_ids.get(t)?),
        };
        let script = match target {
            Target::Comp(c) => self.scripts[c as usize],
            Target::Node(_) => "GameObject",
        };
        let method = match (script, m) {
            (_, "set_enabled") => Method::SetEnabled(arg),
            (_, "SetActive") => Method::SetActive(arg),
            ("ParticleSystem", "Play") => Method::ParticlePlay,
            ("ParticleSystem", _) => Method::ParticleStop,
            ("LineEffect", _) | ("PlayLineAnimation", _) => Method::LinePlay,
            (_, "Explode") => Method::Explode,
            (_, "Spawn") => Method::Spawn,
            (_, "Go") => Method::Go,
            (_, "DoJump") => Method::DoJump,
            (_, "DoReload") => Method::DoReload,
            (_, "DoBlock") => Method::DoBlock,
            (_, "TurnOn") => Method::TurnOn,
            (_, "TurnOff") => Method::TurnOff,
            (_, "Add") => Method::Add,
            (_, "Reset") => Method::Reset,
            (_, "TryEvent") => Method::TryEvent,
            (_, "Interupt") => Method::Interrupt,
            (_, "ResetCounter") => Method::ResetCounter,
            (_, "OnDisable") => Method::OnDisable,
            (_, "Attack") => Method::Attack,
            (_, "PlayStart") => Method::PlayStart,
            (_, "PlayEnd") => Method::PlayEnd,
            (_, "DoShake") => Method::Shake,
            _ => Method::Other,
        };
        Some(Call { target, method })
    }
    fn obj(&self, v: &Value) -> Option<ObjectId> {
        self.ids.objects.get(refid(v)?).copied()
    }
    fn objs(&self, p: &Value, k: &str) -> Box<[ObjectId]> {
        p.get(k)
            .and_then(|a| a.as_array())
            .map(|a| a.iter().filter_map(|v| self.obj(v)).collect())
            .unwrap_or_default()
    }
    fn comp(&self, v: &Value) -> Option<CompIdx> {
        self.comp_ids.get(refid(v)?).copied()
    }
    fn nodes(&self, p: &Value, k: &str) -> Box<[NodeIdx]> {
        p.get(k)
            .and_then(|a| a.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| self.node_ids.get(refid(v)?).copied())
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Resolve an `objectsToSpawn` list given as raw Unity fields (effect guns).
pub(crate) fn raw_spawns(p: &Value, ids: &Ids) -> Vec<SpawnEntry> {
    let Some(a) = p.get("objectsToSpawn").and_then(|a| a.as_array()) else {
        return vec![];
    };
    let r = |v: Option<&Value>| v.and_then(refid).and_then(|i| ids.objects.get(i).copied());
    a.iter()
        .map(|e| SpawnEntry {
            effect: r(e.get("effect")),
            add_to_projectile: r(e.get("AddToProjectile")),
            direction: fint(e, "direction", 0) as u8,
            spawn_on: fint(e, "spawnOn", 0) as u8,
            spawn_as_child: fbool(e, "spawnAsChild", false),
            number_of_spawns: fint(e, "numberOfSpawns", 1) as i32,
            normal_offset: fnum(e, "normalOffset", 0.0),
            stick_to_big_targets: fbool(e, "stickToBigTargets", false),
            stick_to_all_targets: fbool(e, "stickToAllTargets", false),
            remove_scripts: fbool(e, "removeScriptsFromProjectileObject", false),
            scale_stacks: fbool(e, "scaleStacks", false),
            scale_stack_m: fnum(e, "scaleStackM", 0.5),
            scale_from_damage: fnum(e, "scaleFromDamage", 0.0),
            stacks: fint(e, "stacks", 0) as i32,
        })
        .collect()
}

/// Sound reference field -> cue name (presentation).
fn sound(ids: &Ids, p: &Value, k: &str) -> Option<&'static str> {
    ids.sounds.get(refid(p.get(k)?)?).copied()
}

/// Path of every node ("" = root, "A/B").
fn node_paths(def: &ObjectDef) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(def.nodes.len());
    for (i, n) in def.nodes.iter().enumerate() {
        let p = match n.parent {
            None => String::new(),
            Some(pi) if (pi as usize) < i => {
                let pp = &out[pi as usize];
                if pp.is_empty() {
                    n.name.clone()
                } else {
                    format!("{pp}/{}", n.name)
                }
            }
            Some(_) => n.name.clone(),
        };
        out.push(p);
    }
    out
}

/// Compile one prefab. `warnings` collects unresolved data.
pub fn compile(def: &ObjectDef, ids: &Ids, warnings: &mut Vec<String>) -> Prefab {
    // Nodes.
    let mut nodes: Vec<Node> = if def.nodes.is_empty() {
        vec![Node {
            name: def.name.clone(),
            parent: None,
            active: true,
            scale: def.scale,
        }]
    } else {
        def.nodes
            .iter()
            .map(|n| Node {
                name: n.name.clone(),
                parent: n.parent,
                active: n.active,
                scale: n.scale,
            })
            .collect()
    };
    nodes[0].parent = None;
    let paths = node_paths(def);
    let node_of = |d: &EffectDesc| -> NodeIdx {
        if let Some(n) = d.node
            && (n as usize) < nodes.len()
        {
            return n;
        }
        if d.path.is_empty() {
            return 0;
        }
        paths
            .iter()
            .position(|p| *p == d.path)
            .map(|i| i as NodeIdx)
            .unwrap_or(0)
    };
    // Component list: legacy shorthand fields first (only if not given as
    // components on the root).
    let mut descs: Vec<(EffectDesc, NodeIdx)> = def
        .effects
        .iter()
        .map(|d| (d.clone(), node_of(d)))
        .collect();
    let has_root = |s: &str| descs.iter().any(|(d, n)| *n == 0 && d.script == s);
    let mut legacy = Vec::new();
    if let Some(e) = def.explosion
        && !has_root("Explosion")
    {
        legacy.push((
            EffectDesc {
                script: "Explosion".into(),
                params: serde_json::to_value(e).unwrap_or(Value::Null),
                ..Default::default()
            },
            0,
        ));
    }
    if let Some(s) = def.remove_after
        && !has_root("RemoveAfterSeconds")
    {
        legacy.push((
            EffectDesc {
                script: "RemoveAfterSeconds".into(),
                params: serde_json::json!({ "seconds": s }),
                ..Default::default()
            },
            0,
        ));
    }
    legacy.append(&mut descs);
    let descs = legacy;

    let mut ctx = Ctx {
        comp_ids: BTreeMap::new(),
        node_ids: BTreeMap::new(),
        scripts: descs.iter().map(|(d, _)| d.script.as_str()).collect(),
        ids,
    };
    for (i, n) in def.nodes.iter().enumerate() {
        if !n.id.is_empty() {
            ctx.node_ids.entry(n.id.as_str()).or_insert(i as NodeIdx);
        }
        if !n.transform.is_empty() {
            ctx.node_ids
                .entry(n.transform.as_str())
                .or_insert(i as NodeIdx);
        }
    }
    for (i, (d, _)) in descs.iter().enumerate() {
        if !d.id.is_empty() {
            ctx.comp_ids.entry(d.id.as_str()).or_insert(i as CompIdx);
        }
    }
    // The rope LineEffect of an Ensnare prefab (cards.md 6.15).
    let line = descs
        .iter()
        .find(|(d, _)| d.script == "LineEffect")
        .map(|(d, _)| {
            let speed = fnum(&d.params, "globalTimeSpeed", 1.0);
            let w = fcurve(&d.params, "lineWidthOverTimeCurve");
            let end = w
                .keys
                .iter()
                .skip(1)
                .find(|k| k.value <= 0.0)
                .map(|k| k.time)
                .unwrap_or_else(|| w.end_time().max(1.0));
            (speed, end)
        })
        .unwrap_or((1.0, 1.0));

    let mut comps: Vec<Comp> = Vec::with_capacity(descs.len());
    let mut slots = 0u16;
    let mut attack_level = None;
    let mut follows = false;
    for (ci, (d, node)) in descs.iter().enumerate() {
        let p = &d.params;
        let kind = match d.script.as_str() {
            "Explosion" => match serde_json::from_value::<ExplosionDef>(p.clone()) {
                Ok(e) => Kind::Explosion(e),
                Err(err) => {
                    warnings.push(format!("{}: Explosion params: {err}", def.name));
                    Kind::Explosion(ExplosionDef::default())
                }
            },
            "Explosion_Overpower" => Kind::ExplosionOverpower {
                dmg_per_100hp: fnum(p, "dmgPer100Hp", 0.0),
            },
            "Implosion" => Kind::Implosion {
                force: fnum(p, "force", 0.0),
                clamp_dist: fnum(p, "clampDist", 0.0),
                drag: fnum(p, "drag", 0.0),
                time: fnum(p, "time", 0.0),
            },
            "RemoveAfterSeconds" => Kind::RemoveAfterSeconds {
                seconds: fnum(p, "seconds", 1.0),
                scale_with_scale: fbool(p, "scaleWithScale", false),
            },
            "DelayEvent" => Kind::DelayEvent {
                time: fnum(p, "time", 1.0),
                auto: fbool(p, "auto", false),
                repeating: fbool(p, "repeating", false),
                ev: ctx.ev(p, "delayedEvent"),
            },
            "StartEvent" => Kind::StartEvent(ctx.ev(p, "startEvent")),
            "EnableEvent" => Kind::EnableEvent(ctx.ev(p, "enableEvent")),
            "DisableEvent" => Kind::DisableEvent(ctx.ev(p, "disableEvent")),
            "RespawnEvent" => Kind::RespawnEvent(ctx.ev(p, "reviveEvent")),
            "DestroyEvent" => Kind::DestroyEvent(ctx.ev(p, "deathEvent")),
            "SpawnObjects" => Kind::SpawnObjects {
                objects: ctx.objs(p, "objectToSpawn"),
                transform_rotation: fint(p, "spawnRot", 0) == 1,
                inherit_scale: fbool(p, "inheritScale", false),
                destroy_object: fbool(p, "destroyObject", false),
                destroy_root: fbool(p, "destroyRoot", false),
            },
            "FollowPlayer" => {
                let other = fint(p, "target", 0) == 1;
                if *node == 0 && !other {
                    follows = true;
                }
                Kind::FollowPlayer {
                    other,
                    inherit_scale: fbool(p, "inheritScale", true),
                }
            }
            "AttackLevel" => {
                if *node == 0 && attack_level.is_none() {
                    attack_level = Some(ci as CompIdx);
                }
                Kind::AttackLevel {
                    level: fint(p, "attackLevel", 1) as i32,
                    level_scale_m: fnum(p, "levelScaleM", 0.7),
                }
            }
            "LevelScale" => Kind::LevelScale {
                on_start: fbool(p, "onStart", false),
                on_level_up: fbool(p, "onLevelUp", true),
            },
            "BlockTrigger" => Kind::BlockTrigger {
                blacklisted: fint(p, "blackListedType", 1) as u8,
                delay_other_actions: fbool(p, "delayOtherActions", false),
                super_first: ctx.ev(p, "triggerSuperFirstBlock"),
                first_delays: ctx.ev(p, "triggerFirstBlockThatDelaysOthers"),
                early: ctx.ev(p, "triggerEventEarly"),
                block: ctx.ev(p, "triggerEvent"),
                success: ctx.ev(p, "successfulBlockEvent"),
                recharge: ctx.ev(p, "blockRechargeEvent"),
            },
            "EmpowerStopBlockObjectFollow" => Kind::EmpowerStopFollow,
            "Saw" => Kind::Saw {
                range: fnum(p, "range", 0.0),
                damage: fnum(p, "damage", 0.0),
                force: fnum(p, "force", 0.0),
                shake: fnum(p, "shake", 0.0),
            },
            "Teleport" => Kind::Teleport {
                distance: fnum(p, "distance", 0.0),
                mask: fmask(p, "mask", 1),
            },
            "ShieldCharge" => Kind::ShieldCharge(Box::new(ShieldChargeDef {
                time: fnum(p, "time", 0.2),
                force: fnum(p, "force", 0.0),
                drag: fnum(p, "drag", 0.0),
                force_curve: fcurve(p, "forceCurve"),
                damage: fnum(p, "damage", 0.0),
                knockback: fnum(p, "knockBack", 0.0),
                stop_force: fnum(p, "stopForce", 0.0),
                shake: fnum(p, "shake", 0.0),
                damage_per_level: fnum(p, "damagePerLevel", 0.0),
                knockback_per_level: fnum(p, "knockBackPerLevel", 0.0),
                force_per_level: fnum(p, "forcePerLevel", 0.0),
                time_per_level: fnum(p, "timePerLevel", 0.0),
            })),
            "Empower" => Kind::Empower {
                dmg_multiplier: fnum(p, "dmgMultiplier", 1.0),
                speed_multiplier: fnum(p, "speedMultiplier", 1.0),
                add_object: p.get("addObjectToBullet").and_then(|v| ctx.obj(v)),
            },
            "RotatingShooter" => Kind::RotatingShooter {
                bullets_to_fire: fint(p, "bulletsToFire", 1) as i32,
                destroy_after_attack: fbool(p, "destroyAfterAttack", false),
            },
            "Gun" => {
                let stats: GunStats = serde_json::from_value(p.clone()).unwrap_or_default();
                let projectile = p
                    .get("projectiles")
                    .and_then(|a| a.as_array())
                    .and_then(|a| a.first())
                    .and_then(|e| e.get("objectToSpawn"))
                    .and_then(refid)
                    .and_then(|i| ids.projectiles.get(i).copied())
                    .unwrap_or(0);
                Kind::Gun(Box::new(GunDef {
                    stats,
                    spawns: raw_spawns(p, ids).into(),
                    projectile,
                }))
            }
            "GunLevel" => Kind::GunLevel {
                copy_to: p.get("copyTo").and_then(|v| ctx.comp(v)),
            },
            "PlayerDoJump" => Kind::PlayerDoJump {
                multiplier: fnum(p, "multiplier", 1.0),
            },
            "RadarShot" => Kind::RadarShot {
                range: fnum(p, "range", 0.0),
            },
            "AbyssalCountdown" => Kind::AbyssalCountdown {
                time_to_fill: fnum(p, "timeToFill", 7.0),
                time_to_empty: fnum(p, "timeToEmpty", 3.0),
                duration: fnum(p, "duration", 5.0),
                hp_multiplier: fnum(p, "hpMultiplier", 1.0),
                objects: ctx.nodes(p, "abyssalObjects"),
            },
            "BlinkStep" => Kind::BlinkStep {
                interval: fnum(p, "interval", 0.29),
            },
            "StatsAfterDealingDamage" => Kind::StatsAfterDealingDamage {
                duration: fnum(p, "duration", 3.0),
                movement_speed: fnum(p, "movementSpeedMultiplier", 1.0),
                jump: fnum(p, "jumpMultiplier", 1.0),
                hp: fnum(p, "hpMultiplier", 1.0),
                start: ctx.ev(p, "startEvent"),
                end: ctx.ev(p, "endEvent"),
            },
            "StatsWhenFullHP" => Kind::StatsWhenFullHp {
                health_multiplier: fnum(p, "healthMultiplier", 1.0),
                size_multiplier: fnum(p, "sizeMultiplier", 1.0),
                threshold: fnum(p, "healthThreshold", 1.0),
            },
            "Chase" => Kind::Chase {
                on: ctx.ev(p, "turnOnEvent"),
                off: ctx.ev(p, "turnOffEvent"),
            },
            "ToggleStats" => Kind::ToggleStats {
                hp: fnum(p, "hpMultiplier", 1.0),
                movement_speed: fnum(p, "movementSpeedMultiplier", 1.0),
            },
            "PlayerInRangeTrigger" => Kind::PlayerInRangeTrigger {
                any: fint(p, "targetType", 1) == 0,
                range: fnum(p, "range", 5.0),
                cooldown: fnum(p, "cooldown", 0.0),
                repeating: fbool(p, "repeating", false),
                scale_with_range: fbool(p, "scaleWithRange", false),
                ev: ctx.ev(p, "triggerEvent"),
            },
            "PlayerInRangeSlow" => Kind::PlayerInRangeSlow {
                amount: fnum(p, "slowAmount", 0.0),
                max: fnum(p, "maxSlow", 1.0),
            },
            "DealDamageToPlayer" => Kind::DealDamageToPlayer {
                damage: fnum(p, "damage", 25.0),
                lethal: fbool(p, "lethal", true),
                other: fint(p, "targetPlayer", 0) == 1,
            },
            "RegenerateAfterStandStill" => Kind::RegenerateAfterStandStill {
                heal: fnum(p, "heal", 2.0),
            },
            "DealtDamageTrigger" => Kind::DealtDamageTrigger(ctx.ev(p, "triggerEvent")),
            "DoPlayerReload" => Kind::DoPlayerReload,
            "ReloadTigger" | "ReloadTrigger" => Kind::ReloadTrigger {
                out_of_ammo: ctx.ev(p, "outOfAmmoEvent"),
                reload_done: ctx.ev(p, "reloadDoneEvent"),
            },
            "PlayerDoBlock" => Kind::PlayerDoBlock,
            "SpawnObjectOnDealDamage" => Kind::SpawnObjectOnDealDamage {
                damage_needed: fnum(p, "damageNeeded", 0.0),
                cd: fnum(p, "cd", 0.0),
                allow_self: fbool(p, "allowSelfDmg", false),
                ev: ctx.ev(p, "triggerEvent"),
            },
            "ResetBlock" => Kind::ResetBlock,
            "AttackTrigger" => Kind::AttackTrigger {
                every_shot: fbool(p, "triggerOnEveryShot", false),
                ev: ctx.ev(p, "triggerEvent"),
            },
            "DuringReloadTrigger" => Kind::DuringReloadTrigger {
                ev: ctx.ev(p, "triggerEvent"),
                start: ctx.ev(p, "triggerStartEvent"),
                end: ctx.ev(p, "triggerEndEvent"),
            },
            "CooldownCondition" => Kind::CooldownCondition {
                cooldown: fnum(p, "cooldown", 0.0),
                ev: ctx.ev(p, "triggerEvent"),
            },
            "CooldownWindUp" => Kind::CooldownWindUp {
                increase_per_shot: fnum(p, "increasePerShot", 0.0),
                curve: fcurve(p, "multiplierCurve"),
            },
            "LineRangeEffect" => Kind::LineRangeEffect {
                dmg: fnum(p, "dmg", 0.0),
                knockback: fnum(p, "knockback", 0.0),
            },
            "LineEffect" => Kind::LineEffect {
                radius: fnum(p, "radius", 5.0),
                radius_over_time: fcurve(p, "radiusOverTime"),
                speed: fnum(p, "globalTimeSpeed", 1.0),
            },
            "SpawnStaticRemnant" => Kind::SpawnStaticRemnant {
                source: p.get("remnantSource").and_then(|v| ctx.obj(v)),
            },
            "CounterUI" => Kind::CounterUi {
                time_to_fill: fnum(p, "timeToFill", 10.0),
                done: ctx.ev(p, "doneEvent"),
            },
            "StunPlayer" => Kind::StunPlayer {
                own: fint(p, "targetPlayer", 0) == 1,
                time: fnum(p, "time", 0.5),
            },
            "ThereCanOnlyBeOne" => Kind::ThereCanOnlyBeOne(ctx.ev(p, "PokeEvent")),
            "CapScale" => Kind::CapScale {
                min: fnum(p, "min", 0.0),
                max: fnum(p, "max", f32::INFINITY),
            },
            "Thruster" => Kind::Thruster {
                force: fnum(p, "force", 0.0),
                pow: fnum(p, "pow", 1.0),
                physics_object_m: fnum(p, "physicsObjectM", 1.0),
                drag: fnum(p, "drag", 0.0),
            },
            "EnsnareEffect" => Kind::Ensnare(Box::new(EnsnareDef {
                range: fnum(p, "range", 1.0),
                force: fnum(p, "force", 2000.0),
                break_distance: fnum(p, "breakDistance", 2.0),
                drag: fnum(p, "drag", 0.98),
                snap_cap_line_time: fnum(p, "snapCapLineTime", 0.5),
                start_pos_is_my_pos: fbool(p, "startPosIsMyPos", false),
                line_speed: line.0,
                line_end: line.1,
            })),
            "AddShake" => Kind::AddShake {
                shake: crate::math::v2(
                    p.get("shake").map(|s| fnum(s, "x", 0.0)).unwrap_or(0.0),
                    p.get("shake").map(|s| fnum(s, "y", 0.0)).unwrap_or(0.0),
                ),
                auto: fbool(p, "auto", true),
                inherit_scale: fbool(p, "inheritScale", false),
                max: p.get("max").and_then(|v| v.as_f64()).map(|x| x as f32),
            },
            "SoundImplementation.SoundUnityEventPlayer" | "SoundUnityEventPlayer" => {
                Kind::SoundPlayer {
                    start: sound(ids, p, "soundStart"),
                    start_loop: sound(ids, p, "soundStartLoop"),
                    end: sound(ids, p, "soundEnd"),
                    on_start: fbool(p, "triggerStartOnStart", false),
                    on_enable: fbool(p, "triggerStartOnEnable", false),
                    end_on_disable: fbool(p, "triggerEndOnDisable", false),
                    end_on_destroy: fbool(p, "triggerEndOnDestroy", false),
                }
            }
            "ParticleSystem" => Kind::Particles,
            _ => Kind::Cosmetic,
        };
        let stateful = matches!(
            kind,
            Kind::DelayEvent { .. }
                | Kind::RemoveAfterSeconds { .. }
                | Kind::AttackLevel { .. }
                | Kind::LevelScale { .. }
                | Kind::FollowPlayer { .. }
                | Kind::CapScale { .. }
                | Kind::PlayerInRangeTrigger { .. }
                | Kind::CounterUi { .. }
                | Kind::BlinkStep { .. }
                | Kind::LineEffect { .. }
                | Kind::StatsAfterDealingDamage { .. }
                | Kind::StatsWhenFullHp { .. }
                | Kind::ToggleStats { .. }
                | Kind::Chase { .. }
                | Kind::AbyssalCountdown { .. }
                | Kind::ShieldCharge(_)
                | Kind::Empower { .. }
                | Kind::Teleport { .. }
                | Kind::RadarShot { .. }
                | Kind::SpawnObjectOnDealDamage { .. }
                | Kind::CooldownCondition { .. }
                | Kind::CooldownWindUp { .. }
                | Kind::DuringReloadTrigger { .. }
                | Kind::Ensnare(_)
                | Kind::Thruster { .. }
                | Kind::RotatingShooter { .. }
                | Kind::LineRangeEffect { .. }
        );
        let slot = if stateful {
            slots += 1;
            Some(slots - 1)
        } else {
            None
        };
        // Port decision (cards.md 5.2): a disabled `RemoveAfterSeconds`
        // (E_BlackHole) still removes its object, instead of leaving an
        // inert object alive for the rest of the round.
        let enabled = d.enabled.unwrap_or(true) || matches!(kind, Kind::RemoveAfterSeconds { .. });
        comps.push(Comp {
            node: *node,
            enabled,
            kind,
            slot,
        });
    }
    let gameplay = |c: &Comp| !matches!(c.kind, Kind::Cosmetic | Kind::Particles);
    if comps.iter().skip(64).any(gameplay) || comps.iter().any(|c| c.node >= 64 && gameplay(c)) {
        warnings.push(format!(
            "{}: {} components / {} nodes; only the first 64 can be toggled",
            def.name,
            comps.len(),
            nodes.len()
        ));
    }
    // Initial enabled state when the data does not say (EffectDesc docs).
    for (i, (d, _)) in descs.iter().enumerate() {
        if d.enabled.is_none() && enabled_by_event(&comps, i as CompIdx) {
            comps[i].enabled = false;
        }
    }
    let trivial = comps.iter().all(|c| {
        matches!(
            c.kind,
            Kind::Explosion(_)
                | Kind::RemoveAfterSeconds { .. }
                | Kind::Cosmetic
                | Kind::Particles
                | Kind::AddShake { .. }
                | Kind::SoundPlayer { .. }
        )
    }) && comps
        .iter()
        .all(|c| c.node == 0 || !matches!(c.kind, Kind::Explosion(_)));
    if nodes.is_empty() {
        nodes.push(Node {
            name: def.name.clone(),
            parent: None,
            active: true,
            scale: 1.0,
        });
    }
    Prefab {
        nodes,
        comps,
        slots,
        attack_level,
        follows,
        trivial,
    }
}

/// Does some UnityEvent of the prefab call `set_enabled(true)` on comp `c`?
fn enabled_by_event(comps: &[Comp], c: CompIdx) -> bool {
    let hit = |ev: &Ev| {
        ev.iter()
            .any(|k| k.target == Target::Comp(c) && k.method == Method::SetEnabled(true))
    };
    comps.iter().any(|x| match &x.kind {
        Kind::DelayEvent { ev, .. }
        | Kind::StartEvent(ev)
        | Kind::EnableEvent(ev)
        | Kind::DisableEvent(ev)
        | Kind::RespawnEvent(ev)
        | Kind::DestroyEvent(ev)
        | Kind::DealtDamageTrigger(ev)
        | Kind::ThereCanOnlyBeOne(ev) => hit(ev),
        Kind::BlockTrigger {
            super_first,
            first_delays,
            early,
            block,
            ..
        } => hit(super_first) || hit(first_delays) || hit(early) || hit(block),
        Kind::PlayerInRangeTrigger { ev, .. }
        | Kind::AttackTrigger { ev, .. }
        | Kind::CooldownCondition { ev, .. }
        | Kind::SpawnObjectOnDealDamage { ev, .. } => hit(ev),
        Kind::StatsAfterDealingDamage { start, end, .. } => hit(start) || hit(end),
        Kind::Chase { on, off } => hit(on) || hit(off),
        Kind::CounterUi { done, .. } => hit(done),
        _ => false,
    })
}

/// Projectile add-on behaviours of a prefab used as `AddToProjectile`
/// (weapons.md 16, 20). Root components only; children are visuals.
/// Without the content's sound table (cues fall back to default names).
pub fn compile_addons(def: &ObjectDef) -> Vec<AddonDef> {
    let none = BTreeMap::new();
    compile_addons_with(
        def,
        &Ids {
            objects: &BTreeMap::new(),
            projectiles: &BTreeMap::new(),
            sounds: &none,
        },
    )
}

/// [`compile_addons`] resolving sound references through `ids`.
pub fn compile_addons_with(def: &ObjectDef, ids: &Ids) -> Vec<AddonDef> {
    let mut out = Vec::new();
    for d in &def.effects {
        let on_root = d.node.unwrap_or(0) == 0 && (d.node.is_some() || d.path.is_empty());
        if !on_root {
            continue;
        }
        let p = &d.params;
        let a = match d.script.as_str() {
            "Homing" => AddonDef::Homing {
                amount: fnum(p, "amount", 1.0),
                scaling_drag: fnum(p, "scalingDrag", 0.0),
                drag: fnum(p, "drag", 0.0),
                spread: fnum(p, "spread", 0.0),
                sound: sound(ids, p, "soundHomingFound"),
            },
            "RemoteControl" => AddonDef::Remote {
                rotate_speed: fnum(p, "rotateSpeed", 1.0),
            },
            "RayHitDrill" => AddonDef::Drill {
                meters: fnum(p, "metersOfDrilling", 1.0),
                speed_mod: fnum(p, "speedMod", 0.05),
                speed_mod_flat: fnum(p, "speedModFlat", 0.8),
            },
            "RayHitPoison" => AddonDef::Poison {
                sound: sound(ids, p, "soundEventDamageOverTime"),
                time: fnum(p, "time", 1.0),
                interval: fnum(p, "interval", 0.3),
            },
            "RayHitBash" => AddonDef::Bash(super::addons::BashDef {
                base_trigger_chance: fnum(p, "baseTriggerChance", 0.2),
                trigger_chance_per_ten_damage: fnum(p, "triggerChancePerTenDamage", 0.1),
                base_stun_time: fnum(p, "baseStunTime", 0.0),
                stun_time_per_ten_damage: fnum(p, "stunTimePerTenDamage", 0.0),
                stun_multiplier: fnum(p, "stunMultiplier", 1.0),
                multiplier_per_ten_meter: fnum(p, "multiplierPerTenMeterTravelled", 0.0),
                stun_time_exponent: fnum(p, "stunTimeExponent", 1.0),
                cannot_perma_stun: fbool(p, "cannotPermaStun", false),
                stun_time_threshold: fnum(p, "stunTimeThreshold", 0.2),
            }),
            "TrickShot" => AddonDef::TrickShot {
                multiplier: fnum(p, "muiltiplier", 1.0),
                remove_at: fnum(p, "removeAt", 30.0),
            },
            "DodgeGround" => AddonDef::DodgeGround {
                force: fnum(p, "force", 0.0),
                ray_length: fnum(p, "rayLength", 3.0),
                ray_up: fnum(p, "rayUp", 0.3),
                mask: fmask(p, "mask", 1),
            },
            "BounceEffectRetarget" => AddonDef::TargetBounce {
                sound: sound(ids, p, "soundTargetBounceTargetPlayer"),
            },
            "ScreenEdgeBounce" => AddonDef::ScreenEdgeBounce,
            "MoveTransformGravity" => AddonDef::ExtraGravity {
                amount: fnum(p, "amount", 1.0),
                pow: fnum(p, "pow", 1.0),
            },
            "ProjectileHitEmpower" => AddonDef::EmpowerHit,
            _ => continue,
        };
        out.push(a);
    }
    out
}
