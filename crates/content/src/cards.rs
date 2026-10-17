//! Cards, spawnable prefabs and projectiles (docs/spec/cards.md,
//! weapons.md 2, 18-21).
//!
//! - [`CardDef`]: `CardInfo` metadata, the `Gun` / `CharacterStatModifiers`
//!   / `Block` stat blocks (deserialised straight from the raw fields, prefab
//!   references replaced by prefab names), the gun's `objectsToSpawn`.
//! - Every prefab reachable from the cards (`AddObjectToPlayer`,
//!   `objectsToSpawn` effects and add-ons, block spawns, and transitively
//!   everything those prefabs reference: `SpawnObjects`, inner guns, ...)
//!   becomes an [`ObjectDef`], or a [`ProjectileDef`] when its root is a
//!   projectile (`ProjectileHit` + `MoveTransform`).
//! - Effects follow the sim's adapter contract ([`EffectDesc`] docs): every
//!   component of every node of an object prefab, in component order, with
//!   the raw Unity fields as `params` (engine classes whose fields the import
//!   types, i.e. colliders, `Rigidbody2D`, joints and `SpriteRenderer`, get
//!   that typed data), `id` = the component id, `node` = the index into
//!   [`ObjectDef::nodes`] (= the prefab's object list), `enabled` = the
//!   imported `m_Enabled` (None for components without the flag or imports
//!   before revision 1). Nodes carry their Transform id, so Transform
//!   references resolve. Projectiles carry the components beyond the
//!   standard projectile set, and their `RayCastTrail` masks.
//! - The draw pool is `CardChoice.cards` of the main scene, in order.

use std::collections::{BTreeMap, BTreeSet};

use rounds_import::GameData;
use rounds_import::format::{Card, Object, Prefab};
use rounds_sim::content::{
    CardDef, CardStatLine, EffectDesc, ExplosionDef, NodeDef, ObjectDef, ObjectsToSpawnDef,
    ProjectileDef, Rarity,
};
use rounds_sim::math::v2;
use rounds_sim::stats::{BlockCardStats, CharacterStats, GunStats, Rgba};
use serde_json::{Value, json};

use crate::fields::{b, f, field_ref, num, ref_id, refs_in, refs_to_names, vec_z, vec2};
use crate::maps::script;
use crate::report::Report;

/// Components that are rendering, audio, networking, UI or editor-only on
/// effect prefabs and projectiles (cards.md 5.2 list plus engine classes).
pub const PRESENTATION: &[&str] = &[
    "SpriteRenderer",
    "ParticleSystem",
    "ParticleSystemRenderer",
    "ParticleSystemForceField",
    "TrailRenderer",
    "LineRenderer",
    "MeshFilter",
    "MeshRenderer",
    "CanvasRenderer",
    "Canvas",
    "CanvasGroup",
    "SortingGroup",
    "SpriteMask",
    "Animator",
    "UnityEngine.UI.Image",
    "UnityEngine.UI.CanvasScaler",
    "UnityEngine.UI.GraphicRaycaster",
    "UnityEngine.UI.HorizontalLayoutGroup",
    "UnityEngine.UI.VerticalLayoutGroup",
    "UnityEngine.UI.GridLayoutGroup",
    "UnityEngine.UI.LayoutElement",
    "UnityEngine.UI.Mask",
    "UnityEngine.UI.ProceduralImage.ProceduralImage",
    "TMPro.TextMeshProUGUI",
    "TMPro.TextMeshPro",
    "UILocalizedString",
    "Photon.Pun.PhotonView",
    "Photon.Pun.SyncProjectile",
    "ChildRPC",
    "SFPolygon",
    "SkipMasking",
    "PositionNoise",
    "SetTeamColor",
    "SetTeamColorFromParentPlayer",
    "SetTeamColorFromSpawnedAttack",
    "SetTeamColorSpecific",
    "SetSpriteColor",
    "SetSpawnedParticleColor",
    "ChangeColor",
    "AddShake",
    "ParticleExplosionModifier",
    "LineEffectExplosionModifier",
    "PlayLineAnimation",
    "UnparentOnHit",
    "ObjectShake",
    "MoveForward",
    "ApplyRootScale",
    "SoundImplementation.SoundUnityEventPlayer",
    "PlayerWobblePosition",
    "ScaleEvent",
    "FlickerEvent",
    "RayHitBulletSound",
    "ScaleTrailFromDamage",
    "ParticleTime",
    "FreeModifier",
    "RoundModifier",
    "CodeAnimation",
    "ScaleShake",
    "FriendlyFoe.BulletPoolInstancer",
    "CardAudioModifier",
    "RootMotion.FinalIK.LimbIK",
];

/// The standard projectile components ([`ProjectileDef`] fields).
const PROJECTILE_STANDARD: &[&str] = &[
    "MoveTransform",
    "RayCastTrail",
    "ProjectileHit",
    "ProjectileInit",
    "BulletBase",
    "RemoveAfterSeconds",
];

/// Components on a card prefab that `CardDef` covers.
const CARD_STANDARD: &[&str] = &[
    "CardInfo",
    "Gun",
    "ApplyCardStats",
    "CharacterStatModifiers",
    "Block",
];

pub struct Cards {
    pub cards: Vec<CardDef>,
    pub pool: Vec<u16>,
    pub objects: Vec<ObjectDef>,
    pub projectiles: Vec<ProjectileDef>,
    /// Prefab id -> def name of every converted prefab.
    pub names: BTreeMap<String, String>,
}

/// All prefab files of the import, indexed by object and component id.
pub struct Prefabs {
    pub list: BTreeMap<String, Prefab>,
    /// Object/component id -> prefab id.
    index: BTreeMap<String, String>,
}

impl Prefabs {
    pub fn load(data: &GameData, r: &mut Report) -> Prefabs {
        let mut list = BTreeMap::new();
        let dir = data.dir.join("prefabs");
        let mut files: Vec<_> = std::fs::read_dir(&dir)
            .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).collect())
            .unwrap_or_default();
        files.sort();
        for p in files {
            if p.extension().is_none_or(|e| e != "json") {
                continue;
            }
            match std::fs::read(&p)
                .map_err(anyhow::Error::from)
                .and_then(|s| serde_json::from_slice::<Prefab>(&s).map_err(Into::into))
            {
                Ok(pf) => {
                    list.insert(pf.id.clone(), pf);
                }
                Err(e) => r.warn(format!("{}: {e}", p.display())),
            }
        }
        let mut index = BTreeMap::new();
        for (pid, pf) in &list {
            for o in &pf.objects {
                index.insert(o.id.clone(), pid.clone());
                for c in &o.components {
                    index.insert(c.id.clone(), pid.clone());
                }
                for c in &o.colliders {
                    index.entry(c.id.clone()).or_insert_with(|| pid.clone());
                }
            }
        }
        r.count("prefab files", list.len());
        Prefabs { list, index }
    }

    /// The prefab a reference points into (root or inner object/component).
    fn prefab_of(&self, id: &str) -> Option<&str> {
        self.index.get(id).map(String::as_str)
    }
}

/// Path of object `k` inside a hierarchy ("" = root).
pub fn path_in(objs: &[Object], mut k: usize) -> String {
    let mut parts = vec![];
    while let Some(p) = objs[k].parent {
        parts.push(objs[k].name.as_str());
        k = p;
    }
    parts.reverse();
    parts.join("/")
}

fn comp<'a>(o: &'a Object, name: &str) -> Option<&'a Value> {
    o.components
        .iter()
        .find(|c| script(c) == name)
        .map(|c| &c.fields)
}

fn is_projectile(p: &Prefab) -> bool {
    p.objects.first().is_some_and(|o| {
        o.components.iter().any(|c| script(c) == "ProjectileHit")
            && o.components.iter().any(|c| script(c) == "MoveTransform")
    })
}

/// Prefab references a card's stat components spawn or attach.
fn card_seeds(card: &Card, prefab: Option<&Prefab>) -> Vec<String> {
    let mut out = vec![];
    if let Some(g) = card.components.get("Gun") {
        if let Some(Value::Array(list)) = g.get("objectsToSpawn") {
            for e in list {
                out.extend(field_ref(e, "effect").map(String::from));
                out.extend(field_ref(e, "AddToProjectile").map(String::from));
            }
        }
        if let Some(Value::Array(list)) = g.get("projectiles") {
            for e in list {
                out.extend(field_ref(e, "objectToSpawn").map(String::from));
            }
        }
    }
    if let Some(c) = card.components.get("CharacterStatModifiers") {
        out.extend(field_ref(c, "AddObjectToPlayer").map(String::from));
    }
    if let Some(bl) = prefab
        .and_then(first_block)
        .or(card.components.get("Block"))
        && let Some(Value::Array(list)) = bl.get("objectsToSpawn")
    {
        out.extend(list.iter().filter_map(ref_id).map(String::from));
    }
    out
}

fn first_block(p: &Prefab) -> Option<&Value> {
    p.objects.iter().find_map(|o| comp(o, "Block"))
}

/// `extra_seeds`: further prefab references to convert (the player's
/// default bullet).
pub fn convert(data: &GameData, extra_seeds: &[String], r: &mut Report) -> Cards {
    let prefabs = Prefabs::load(data, r);
    let player_prefabs: BTreeSet<&str> = data.player.prefabs.iter().map(String::as_str).collect();
    let card_ids: BTreeSet<&str> = data.cards.iter().map(|c| c.id.as_str()).collect();

    // Reachable spawnable prefabs, breadth first in a stable order.
    let mut order: Vec<String> = vec![];
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut queue: std::collections::VecDeque<String> = Default::default();
    let mut missing: BTreeSet<String> = BTreeSet::new();
    let push = |id: String,
                queue: &mut std::collections::VecDeque<String>,
                seen: &mut BTreeSet<String>,
                missing: &mut BTreeSet<String>| {
        match prefabs.prefab_of(&id) {
            Some(pid) => {
                if !player_prefabs.contains(pid)
                    && !card_ids.contains(pid)
                    && seen.insert(pid.to_string())
                {
                    queue.push_back(pid.to_string());
                }
            }
            None => {
                missing.insert(id);
            }
        }
    };
    for s in extra_seeds {
        push(s.clone(), &mut queue, &mut seen, &mut missing);
    }
    for card in &data.cards {
        for s in card_seeds(card, prefabs.list.get(&card.id)) {
            push(s, &mut queue, &mut seen, &mut missing);
        }
    }
    while let Some(pid) = queue.pop_front() {
        order.push(pid.clone());
        let pf = &prefabs.list[&pid];
        let mut refs = vec![];
        for o in &pf.objects {
            for c in &o.components {
                refs_in(&c.fields, &mut refs);
            }
        }
        // Pooled visuals of a projectile (trail, sparks, the collision
        // circle read by `convert_projectile`) are not spawnable objects.
        let pooled: BTreeSet<String> = pf
            .objects
            .iter()
            .flat_map(|o| &o.components)
            .filter(|c| script(c) == "FriendlyFoe.BulletPoolInstancer")
            .flat_map(|c| {
                let mut v = vec![];
                refs_in(&c.fields, &mut v);
                v
            })
            .collect();
        for id in refs {
            if pooled.contains(&id) {
                continue;
            }
            if prefabs.prefab_of(&id).is_some_and(|p| p != pid) {
                push(id, &mut queue, &mut seen, &mut missing);
            } else if prefabs.prefab_of(&id).is_none()
                && !data.audio.contains_key(&id)
                && !data.sprites.contains_key(&id)
                && !data.materials.contains_key(&id)
                && !data.textures.contains_key(&id)
            {
                // Assets (animation clips, meshes, ...) are not imported; only
                // count GameObject-looking misses below.
                missing.insert(id);
            }
        }
    }
    // Refs from cards that are not prefab files: report.
    let mut card_missing = 0;
    for card in &data.cards {
        for s in card_seeds(card, prefabs.list.get(&card.id)) {
            if prefabs.prefab_of(&s).is_none() {
                card_missing += 1;
                r.warn(format!(
                    "card {}: referenced prefab {s} not in the import",
                    card.name
                ));
            }
        }
    }
    r.count("card prefab references unresolved", card_missing);
    r.count(
        "unresolved asset references in effect prefabs",
        missing.len(),
    );

    // Names (prefab names are unique in the shipped data; disambiguate anyway).
    let mut names: BTreeMap<String, String> = BTreeMap::new();
    let mut used: BTreeSet<String> = BTreeSet::new();
    for pid in &order {
        let base = prefabs.list[pid].name.clone();
        let name = if used.contains(&base) {
            format!("{base}#{pid}")
        } else {
            base
        };
        used.insert(name.clone());
        names.insert(pid.clone(), name);
    }

    let mut objects = vec![];
    let mut projectiles = vec![];
    for pid in &order {
        let pf = &prefabs.list[pid];
        if is_projectile(pf) {
            projectiles.push(convert_projectile(pf, &names[pid], &prefabs, r));
        } else {
            objects.push(convert_object(pf, &names[pid], r));
        }
    }
    r.count("objects (spawnable prefabs)", objects.len());
    r.count("projectiles", projectiles.len());

    // Cards.
    let mut cards = vec![];
    for card in &data.cards {
        cards.push(convert_card(card, prefabs.list.get(&card.id), &names, r));
    }
    r.count("cards", cards.len());
    let pool = pool(data, &prefabs, r);
    r.count(
        "card pool",
        if pool.is_empty() {
            cards.len()
        } else {
            pool.len()
        },
    );
    Cards {
        cards,
        pool,
        objects,
        projectiles,
        names,
    }
}

/// `CardChoice.cards` (main scene) as indices into `data.cards`.
fn pool(data: &GameData, prefabs: &Prefabs, r: &mut Report) -> Vec<u16> {
    let Some(main) = data.main.as_ref() else {
        r.warn("main.json missing: card pool = every card");
        return vec![];
    };
    let Some(list) = main
        .objects
        .iter()
        .find_map(|o| comp(o, "CardChoice"))
        .and_then(|c| c.get("cards"))
        .and_then(Value::as_array)
    else {
        r.warn("CardChoice.cards not found: card pool = every card");
        return vec![];
    };
    let mut out = vec![];
    for e in list {
        let Some(id) = ref_id(e) else { continue };
        // The entries reference the CardInfo components; find their card.
        let card = prefabs
            .prefab_of(id)
            .and_then(|pid| data.cards.iter().position(|c| c.id == pid))
            .or_else(|| data.cards.iter().position(|c| c.id == id));
        match card {
            Some(i) => out.push(i as u16),
            None => r.warn(format!("card pool entry {id} is not a card")),
        }
    }
    out
}

fn rgba(v: Option<&Value>) -> Rgba {
    v.and_then(|c| serde_json::from_value(c.clone()).ok())
        .unwrap_or_default()
}

fn convert_card(
    card: &Card,
    prefab: Option<&Prefab>,
    names: &BTreeMap<String, String>,
    r: &mut Report,
) -> CardDef {
    let null = Value::Null;
    let ci = card.components.get("CardInfo").unwrap_or(&null);
    let name = if card.name.is_empty() {
        r.warn(format!(
            "card {} has no name; using the object name",
            card.id
        ));
        card.object_name.clone()
    } else {
        card.name.clone()
    };
    let mut cats = |names: &[String], key: &str| -> Vec<String> {
        let ids: Vec<String> = ci
            .get(key)
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(ref_id).map(String::from).collect())
            .unwrap_or_default();
        if names.len() == ids.len() {
            return names.to_vec();
        }
        // Import before revision 1: no category names; the ids still work
        // as keys (only equality matters for the blacklist).
        r.warn(format!(
            "card {}: category names not in the import (re-import); using ids",
            card.object_name
        ));
        ids
    };
    let categories = cats(&card.categories, "categories");
    let blacklisted_categories = cats(&card.blacklisted_categories, "blacklistedCategories");
    let stats = ci
        .get("cardStats")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|s| serde_json::from_value::<CardStatLine>(s.clone()).ok())
                .collect()
        })
        .unwrap_or_default();
    let gun = card.components.get("Gun").map(|g| {
        serde_json::from_value::<GunStats>(g.clone()).unwrap_or_else(|e| {
            r.warn(format!("card {name}: Gun: {e}"));
            GunStats::default()
        })
    });
    let objects_to_spawn = card
        .components
        .get("Gun")
        .and_then(|g| g.get("objectsToSpawn"))
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|e| {
                    serde_json::from_value::<ObjectsToSpawnDef>(refs_to_names(e, names))
                        .map_err(|err| r.warn(format!("card {name}: objectsToSpawn: {err}")))
                        .ok()
                })
                .collect()
        })
        .unwrap_or_default();
    let character = card.components.get("CharacterStatModifiers").map(|c| {
        serde_json::from_value::<CharacterStats>(refs_to_names(c, names)).unwrap_or_else(|e| {
            r.warn(format!("card {name}: CharacterStatModifiers: {e}"));
            CharacterStats::default()
        })
    });
    let block = prefab
        .and_then(first_block)
        .or(card.components.get("Block"))
        .map(|bl| {
            let mut v = bl.clone();
            let spawns: Vec<Value> = bl
                .get("objectsToSpawn")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(ref_id)
                        .filter_map(|id| names.get(id))
                        .map(|n| Value::String(n.clone()))
                        .collect()
                })
                .unwrap_or_default();
            v["objectsToSpawn"] = Value::Array(spawns);
            serde_json::from_value::<BlockCardStats>(v).unwrap_or_else(|e| {
                r.warn(format!("card {name}: Block: {e}"));
                BlockCardStats::default()
            })
        });
    // Anything else on the card prefab has no CardDef slot.
    if let Some(p) = prefab {
        for o in &p.objects {
            for c in &o.components {
                let s = script(c);
                if CARD_STANDARD.contains(&s) {
                    continue;
                }
                if PRESENTATION.contains(&s) {
                    r.ignored(s);
                } else {
                    r.unsupported(&format!("card component {s}"));
                }
            }
        }
    }
    CardDef {
        name,
        id: card.id.clone(),
        description: card.description.clone(),
        rarity: Rarity::from(card.rarity.clamp(0, 2) as u8),
        color_theme: card.color_theme.clamp(0, 255) as u8,
        card_color: rgba(ci.get("cardColor")),
        allow_multiple: b(ci, "allowMultiple", true),
        categories,
        blacklisted_categories,
        sound_disable_block_basic: b(ci, "soundDisableBlockBasic", false),
        affects_block: b(ci, "m_affectsBlock", false),
        stats,
        gun,
        objects_to_spawn,
        character,
        block,
    }
}

/// Effect descriptions of the components of a prefab (see the module docs).
/// `skip_root`: root components represented by typed fields instead.
fn effects_of(
    pf: &Prefab,
    skip_root: &[&str],
    with_nodes: bool,
    r: &mut Report,
) -> Vec<EffectDesc> {
    let mut out = vec![];
    for (k, o) in pf.objects.iter().enumerate() {
        let path = path_in(&pf.objects, k);
        for c in &o.components {
            let s = script(c);
            if k == 0 && skip_root.contains(&s) {
                continue;
            }
            // Projectiles (no nodes) only carry gameplay components.
            if !with_nodes && PRESENTATION.contains(&s) {
                r.ignored(s);
                continue;
            }
            let params = match c.kind.as_str() {
                "MonoBehaviour" => c.fields.clone(),
                "BoxCollider2D" | "CircleCollider2D" | "CapsuleCollider2D"
                | "PolygonCollider2D" | "EdgeCollider2D" => {
                    let mut v = o
                        .colliders
                        .iter()
                        .find(|x| x.id == c.id)
                        .and_then(|x| serde_json::to_value(x).ok())
                        .unwrap_or(json!({}));
                    v["layer"] = json!(o.layer);
                    v
                }
                "Rigidbody2D" => serde_json::to_value(&o.rigidbody).unwrap_or(Value::Null),
                "SpriteRenderer" => serde_json::to_value(&o.sprite).unwrap_or(Value::Null),
                k if k.ends_with("Joint2D") => o
                    .joints
                    .iter()
                    .find(|x| x.id == c.id)
                    .and_then(|x| serde_json::to_value(x).ok())
                    .unwrap_or(Value::Null),
                _ => c.fields.clone(),
            };
            if PRESENTATION.contains(&s) {
                r.ignored(s);
            } else {
                r.effect(s);
            }
            match c.enabled {
                Some(false) => r.count("effect components initially disabled (m_Enabled)", 1),
                None if c.kind == "MonoBehaviour" => {
                    r.count("effect scripts without m_Enabled (old import)", 1)
                }
                _ => {}
            }
            out.push(EffectDesc {
                script: s.to_string(),
                path: path.clone(),
                params,
                id: c.id.clone(),
                node: with_nodes.then_some(k as u16),
                enabled: c.enabled,
            });
        }
    }
    out
}

/// The prefab's GameObjects as nodes (index = object index).
fn nodes_of(pf: &Prefab) -> Vec<NodeDef> {
    pf.objects
        .iter()
        .map(|o| NodeDef {
            id: o.id.clone(),
            name: o.name.clone(),
            parent: o.parent.map(|p| p as u16),
            active: o.active,
            pos: v2(o.local.position[0], o.local.position[1]),
            scale: o.local.scale[0],
            transform: o.transform.clone().unwrap_or_default(),
        })
        .collect()
}

fn convert_object(pf: &Prefab, name: &str, r: &mut Report) -> ObjectDef {
    let root = &pf.objects[0];
    // The root `Explosion` / `RemoveAfterSeconds` travel as effects (the
    // contract); the typed shorthands are only validated here.
    if let Some(e) = comp(root, "Explosion")
        && let Err(err) = serde_json::from_value::<ExplosionDef>(e.clone())
    {
        r.warn(format!("{name}: Explosion: {err}"));
    }
    ObjectDef {
        name: name.to_string(),
        scale: root.local.scale[0],
        explosion: None,
        remove_after: None,
        effects: effects_of(pf, &[], true, r),
        id: pf.id.clone(),
        nodes: nodes_of(pf),
    }
}

fn convert_projectile(pf: &Prefab, name: &str, prefabs: &Prefabs, r: &mut Report) -> ProjectileDef {
    let root = &pf.objects[0];
    let d = ProjectileDef::default();
    let null = Value::Null;
    let mt = comp(root, "MoveTransform").unwrap_or(&null);
    let ph = comp(root, "ProjectileHit").unwrap_or(&null);
    let rc = comp(root, "RayCastTrail").unwrap_or(&null);
    let bits = |k: &str| {
        rc.get(k)
            .and_then(|m| m.get("m_Bits"))
            .and_then(Value::as_u64)
            .map(|b| b as u32)
    };
    let wall_mask = bits("mask").unwrap_or_else(|| {
        r.warn(format!(
            "projectile {name}: no RayCastTrail.mask; using the standard bullet's"
        ));
        d.wall_mask
    });
    let ignore_walls_mask = bits("ignoreWallsMask").unwrap_or(d.ignore_walls_mask);
    r.count("projectile wall masks from data", 1);
    // Bullet-vs-bullet circle: the pooled collision prefab of the standard
    // bullet (weapons.md 2: radius 3.2 at local scale 0.1).
    let collision_radius = comp(root, "FriendlyFoe.BulletPoolInstancer").map(|bp| {
        field_ref(bp, "m_collisionPrefab")
            .and_then(|id| prefabs.list.get(id))
            .and_then(|cp| {
                cp.objects.iter().enumerate().find_map(|(k, o)| {
                    o.colliders.iter().find_map(|c| match c.shape {
                        rounds_import::format::Shape::Circle { radius } => {
                            let w = crate::xform::compose_world(&cp.objects)[k].to_affine();
                            Some(radius * w.ax.len().max(w.ay.len()))
                        }
                        _ => None,
                    })
                })
            })
            .unwrap_or_else(|| {
                r.warn(format!(
                    "projectile {name}: collision prefab not imported; radius {} assumed",
                    d.collision_radius.unwrap_or(0.32)
                ));
                d.collision_radius.unwrap_or(0.32)
            })
    });
    let world_force = vec2(mt, "worldForce").unwrap_or(d.world_force);
    let stun = f(ph, "stun", 0.5);
    let _ = stun;
    ProjectileDef {
        name: name.to_string(),
        id: pf.id.clone(),
        gravity: f(mt, "gravity", d.gravity),
        speed: vec_z(mt, "localForce").unwrap_or(d.speed),
        world_force: v2(world_force.x, world_force.y),
        damage: f(ph, "damage", d.damage),
        force: f(ph, "force", d.force),
        hold_player_for: f(ph, "holdPlayerFor", d.hold_player_for),
        lifetime: comp(root, "RemoveAfterSeconds")
            .and_then(|x| num(x, "seconds"))
            .unwrap_or(d.lifetime),
        deal_damage_multiplier: f(ph, "dealDamageMultiplierr", d.deal_damage_multiplier),
        shake: f(ph, "shake", d.shake),
        can_push_box: b(ph, "canPushBox", d.can_push_box),
        destroy_on_block: b(ph, "destroyOnBlock", d.destroy_on_block),
        full_self_damage: b(ph, "fullSelfDamage", d.full_self_damage),
        bullet_immunity: ph
            .get("bulletImmunity")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        collision_radius,
        effects: effects_of(pf, PROJECTILE_STANDARD, false, r),
        wall_mask,
        ignore_walls_mask,
    }
}
