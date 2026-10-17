//! Test fixture: the real cards and prefabs from the player's import
//! (`cache/import`, or `$ROUNDS_IMPORT_DIR`) converted into `ContentDefs`,
//! plus the hand-built test arenas. Tests using it are skipped when no import
//! exists (the repo ships no game data).
//!
//! This mirrors the M5 adapter contract documented on
//! `rounds_sim::content::EffectDesc`: every component of every prefab node
//! becomes an `EffectDesc` with its raw fields, id and node index.
#![allow(dead_code)]

use rounds_sim::content::test_maps::{flat_arena, physics_arena, platforms_arena};
use rounds_sim::content::{
    CardDef, Content, ContentDefs, EffectDesc, NodeDef, ObjectDef, ObjectsToSpawnDef,
    ProjectileDef, Rarity,
};
use rounds_sim::stats::{BlockCardStats, CharacterStats, GunStats};
use rounds_sim::v2;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::OnceLock;

pub fn import_dir() -> Option<PathBuf> {
    let d = std::env::var_os("ROUNDS_IMPORT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../cache/import"));
    d.join("cards.json").exists().then_some(d)
}

fn read(p: &std::path::Path) -> Value {
    serde_json::from_slice(&std::fs::read(p).expect("read")).expect("json")
}

fn refid(v: &Value) -> Option<&str> {
    v.get("$ref")?.as_str()
}

fn f(v: &Value, k: &str, d: f32) -> f32 {
    v.get(k)
        .and_then(|x| x.as_f64())
        .map(|x| x as f32)
        .unwrap_or(d)
}
fn b(v: &Value, k: &str) -> bool {
    match v.get(k) {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|x| x != 0.0),
        _ => false,
    }
}

/// One prefab file -> ObjectDef (+ ProjectileDef if it is a bullet).
fn prefab(d: &Value) -> (ObjectDef, Option<ProjectileDef>) {
    let objs = d["objects"].as_array().unwrap();
    let mut nodes = Vec::new();
    let mut effects = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        let lp = &o["local"]["position"];
        nodes.push(NodeDef {
            id: o["id"].as_str().unwrap_or("").into(),
            name: o["name"].as_str().unwrap_or("").into(),
            parent: o["parent"].as_u64().map(|p| p as u16),
            active: o["active"].as_bool().unwrap_or(true),
            pos: v2(
                lp[0].as_f64().unwrap_or(0.0) as f32,
                lp[1].as_f64().unwrap_or(0.0) as f32,
            ),
            scale: o["local"]["scale"][0].as_f64().unwrap_or(1.0) as f32,
            transform: o["transform"].as_str().unwrap_or("").into(),
        });
        for c in o["components"].as_array().into_iter().flatten() {
            let script = c
                .get("script")
                .and_then(|s| s.as_str())
                .or_else(|| c.get("type").and_then(|s| s.as_str()))
                .unwrap_or("")
                .to_string();
            effects.push(EffectDesc {
                script,
                path: String::new(),
                params: c.get("fields").cloned().unwrap_or(Value::Null),
                id: c["id"].as_str().unwrap_or("").into(),
                node: Some(i as u16),
                enabled: c.get("enabled").and_then(|e| e.as_bool()),
            });
        }
    }
    let name = d["name"].as_str().unwrap().to_string();
    let id = d["id"].as_str().unwrap().to_string();
    let scale = nodes.first().map(|n| n.scale).unwrap_or(1.0);
    // Bullets: root MoveTransform + ProjectileHit.
    let root = |s: &str| {
        effects
            .iter()
            .find(|e| e.node == Some(0) && e.script == s)
            .map(|e| &e.params)
    };
    let proj = match (root("MoveTransform"), root("ProjectileHit")) {
        (Some(m), Some(h)) => Some(ProjectileDef {
            name: name.clone(),
            id: id.clone(),
            gravity: f(m, "gravity", 100.0),
            speed: m["localForce"]
                .get("z")
                .and_then(|z| z.as_f64())
                .unwrap_or(50.0) as f32,
            world_force: v2(f(&m["worldForce"], "x", 0.0), f(&m["worldForce"], "y", 5.0)),
            damage: f(h, "damage", 55.0),
            force: f(h, "force", 5000.0),
            hold_player_for: f(h, "holdPlayerFor", 0.3),
            lifetime: root("RemoveAfterSeconds")
                .map(|r| f(r, "seconds", 15.0))
                .unwrap_or(15.0),
            deal_damage_multiplier: f(h, "dealDamageMultiplierr", 1.0),
            shake: f(h, "shake", 0.8),
            can_push_box: b(h, "canPushBox"),
            destroy_on_block: b(h, "destroyOnBlock"),
            full_self_damage: b(h, "fullSelfDamage"),
            bullet_immunity: h["bulletImmunity"].as_str().unwrap_or("").into(),
            collision_radius: root("FriendlyFoe.BulletPoolInstancer").map(|_| 0.32),
            effects: vec![],
            wall_mask: root("RayCastTrail")
                .and_then(|r| r["mask"]["m_Bits"].as_u64())
                .map_or(rounds_sim::geom::layers::BULLET_WALLS, |m| m as u32),
            ignore_walls_mask: root("RayCastTrail")
                .and_then(|r| r["ignoreWallsMask"]["m_Bits"].as_u64())
                .map_or(rounds_sim::geom::layers::BULLET_IGNORE_WALLS, |m| m as u32),
        }),
        _ => None,
    };
    (
        ObjectDef {
            name,
            id,
            scale,
            nodes,
            effects,
            ..ObjectDef::default()
        },
        proj,
    )
}

fn spawn_entry(e: &Value, names: &BTreeMap<String, String>) -> ObjectsToSpawnDef {
    let n = |k: &str| e.get(k).and_then(refid).and_then(|r| names.get(r).cloned());
    ObjectsToSpawnDef {
        effect: n("effect"),
        add_to_projectile: n("AddToProjectile"),
        direction: e["direction"].as_u64().unwrap_or(0) as u8,
        spawn_on: e["spawnOn"].as_u64().unwrap_or(0) as u8,
        spawn_as_child: b(e, "spawnAsChild"),
        number_of_spawns: e["numberOfSpawns"].as_i64().unwrap_or(1) as i32,
        normal_offset: f(e, "normalOffset", 0.0),
        stick_to_big_targets: b(e, "stickToBigTargets"),
        stick_to_all_targets: b(e, "stickToAllTargets"),
        zero_z: b(e, "zeroZ"),
        remove_scripts_from_projectile_object: b(e, "removeScriptsFromProjectileObject"),
        scale_stacks: b(e, "scaleStacks"),
        scale_stack_m: f(e, "scaleStackM", 0.5),
        scale_from_damage: f(e, "scaleFromDamage", 0.0),
        stacks: e["stacks"].as_i64().unwrap_or(0) as i32,
    }
}

/// Category names (import revision 1), else the category ids.
fn category_names(card: &Value, info: &Value, key: &str, raw: &str) -> Vec<String> {
    if let Some(a) = card[key].as_array().filter(|a| !a.is_empty()) {
        return a
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();
    }
    info[raw]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| refid(v).map(String::from))
        .collect()
}

/// SoundEvent id -> name from `sounds.json` (empty for older imports).
fn sound_events(dir: &std::path::Path) -> BTreeMap<String, String> {
    let p = dir.join("sounds.json");
    if !p.exists() {
        return BTreeMap::new();
    }
    read(&p)["events"]
        .as_object()
        .map(|m| {
            m.iter()
                .map(|(id, e)| (id.clone(), e["name"].as_str().unwrap_or("").to_string()))
                .collect()
        })
        .unwrap_or_default()
}

pub struct Real {
    pub defs: ContentDefs,
    /// Card object names ("Teleport", "Poison bullets", ...) -> card index.
    pub by_object: BTreeMap<String, u16>,
}

/// The real data as `ContentDefs` (maps: flat, platforms, physics arenas).
pub fn real_defs() -> Option<Real> {
    let dir = import_dir()?;
    let mut objects = Vec::new();
    let mut projectiles = vec![];
    let mut names = BTreeMap::new();
    let mut files: Vec<_> = std::fs::read_dir(dir.join("prefabs"))
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    files.sort();
    for p in files {
        let d = read(&p);
        let (o, pr) = prefab(&d);
        names.insert(o.id.clone(), o.name.clone());
        if let Some(pr) = pr {
            if pr.name == "Bullet_Base" {
                projectiles.insert(0, pr);
            } else {
                projectiles.push(pr);
            }
        }
        objects.push(o);
    }
    let cards_json = read(&dir.join("cards.json"));
    let mut cards = Vec::new();
    let mut pool = Vec::new();
    let mut by_object = BTreeMap::new();
    for c in cards_json.as_array()? {
        let co = &c["components"];
        let info = &co["CardInfo"];
        let mut card = CardDef {
            name: c["name"].as_str().unwrap_or("").into(),
            description: c["description"].as_str().unwrap_or("").into(),
            rarity: Rarity::from(c["rarity"].as_u64().unwrap_or(0) as u8),
            allow_multiple: b(info, "allowMultiple"),
            categories: category_names(c, info, "categories", "categories"),
            blacklisted_categories: category_names(
                c,
                info,
                "blacklisted_categories",
                "blacklistedCategories",
            ),
            sound_disable_block_basic: b(info, "soundDisableBlockBasic"),
            ..Default::default()
        };
        if let Some(g) = co.get("Gun") {
            card.gun = Some(serde_json::from_value::<GunStats>(g.clone()).unwrap());
            card.objects_to_spawn = g["objectsToSpawn"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|e| spawn_entry(e, &names))
                .collect();
        }
        if let Some(ch) = co.get("CharacterStatModifiers") {
            let mut ch = ch.clone();
            let obj = ch
                .get("AddObjectToPlayer")
                .and_then(refid)
                .and_then(|r| names.get(r).cloned());
            ch["AddObjectToPlayer"] = obj.map(Value::String).unwrap_or(Value::Null);
            card.character = Some(serde_json::from_value::<CharacterStats>(ch).unwrap());
        }
        if let Some(bl) = co.get("Block") {
            let mut bl = bl.clone();
            let list: Vec<Value> = bl["objectsToSpawn"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| refid(v).and_then(|r| names.get(r).cloned()))
                .map(Value::String)
                .collect();
            bl["objectsToSpawn"] = Value::Array(list);
            card.block = Some(serde_json::from_value::<BlockCardStats>(bl).unwrap());
        }
        let idx = cards.len() as u16;
        by_object.insert(c["object_name"].as_str().unwrap_or("").to_string(), idx);
        if c["id"].as_str().unwrap_or("").starts_with("sharedassets0") {
            pool.push(idx);
        }
        cards.push(card);
    }
    Some(Real {
        defs: ContentDefs {
            maps: vec![flat_arena(), platforms_arena(), physics_arena()],
            cards,
            pool,
            objects,
            projectiles,
            tuning: Default::default(),
            sound_events: sound_events(&dir),
        },
        by_object,
    })
}

pub struct RealContent {
    pub content: Content,
    pub by_object: BTreeMap<String, u16>,
}

/// Shared, built once per test binary.
pub fn real() -> Option<&'static RealContent> {
    static R: OnceLock<Option<RealContent>> = OnceLock::new();
    R.get_or_init(|| {
        let r = real_defs()?;
        Some(RealContent {
            content: Content::new(r.defs),
            by_object: r.by_object,
        })
    })
    .as_ref()
}

impl RealContent {
    /// Card id by the card prefab's object name (e.g. "Teleport").
    pub fn card(&self, object_name: &str) -> rounds_sim::CardId {
        rounds_sim::CardId(
            *self
                .by_object
                .get(object_name)
                .unwrap_or_else(|| panic!("no card {object_name:?}")),
        )
    }
}
