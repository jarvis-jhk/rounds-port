//! Immutable game data the simulation reads (docs/sim-design.md "Content").
//!
//! Built once from plain Rust structs ([`ContentDefs`]); an adapter from the
//! importer's JSON fills those (field names follow the original's serialized
//! names where a component maps 1:1). [`Content::new`] derives lookup tables
//! (static-geometry grids, prefab ids). Content is never cloned for rollback.

pub mod card;
pub mod map;
pub mod test_maps;

pub use card::*;
pub use map::*;

use crate::geom::{Grid, Shape};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CardId(pub u16);
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ObjectId(pub u16);
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct MapId(pub u16);

/// Tunable switches for behaviour the specs leave to the project owner.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tuning {
    /// Bullet-vs-bullet health exchange (weapons.md 17: recommended on).
    pub bullet_collision: bool,
    /// Reset `gravityForce`, `regeneration`, `jumps` on a rematch (player.md
    /// 22.2: the original keeps them; recommended: reset).
    pub reset_quirk_stats: bool,
    /// Keep the "slot 0 is never filtered" hand-drawing quirk (cards.md 3.4).
    pub unfiltered_first_slot: bool,
    /// Half extents of the logical arena (maps.md 8.1): `Map.size * 16/9`,
    /// `Map.size` (the adapter sets it from the data).
    pub bounds_half: crate::math::V2,
    /// `PlayerManager.canSeePlayerMask`: line-of-sight mask (cards.md 5.4;
    /// players never block, so the Player bit is ignored).
    pub line_of_sight_mask: u32,
}

impl Default for Tuning {
    fn default() -> Self {
        Tuning {
            bullet_collision: true,
            reset_quirk_stats: true,
            unfiltered_first_slot: true,
            bounds_half: crate::math::v2(20.0 * 16.0 / 9.0, 20.0),
            line_of_sight_mask: crate::geom::layers::LINE_OF_SIGHT
                | crate::geom::layers::bit(crate::geom::layers::PLAYER),
        }
    }
}

/// Everything the adapter provides.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ContentDefs {
    pub maps: Vec<MapDef>,
    pub cards: Vec<CardDef>,
    /// The draw pool (indices into `cards`) in `CardChoice.cards` order.
    /// Empty = every card.
    #[serde(default)]
    pub pool: Vec<u16>,
    #[serde(default)]
    pub objects: Vec<ObjectDef>,
    /// Projectile prefabs; index 0 is the player's default bullet. Empty =
    /// the built-in standard bullet.
    #[serde(default)]
    pub projectiles: Vec<ProjectileDef>,
    #[serde(default)]
    pub tuning: Tuning,
    /// SoundEvent asset id -> name (the cue name), for sound references in
    /// effect prefabs (`{"$ref": id}` in `EffectDesc::params`). Unknown ids
    /// give no cue.
    #[serde(default)]
    pub sound_events: BTreeMap<String, String>,
}

/// Runtime form of an `ObjectsToSpawn` entry: prefab names resolved to ids.
/// `Copy`, so players' lists clone cheaply.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpawnEntry {
    pub effect: Option<ObjectId>,
    pub add_to_projectile: Option<ObjectId>,
    pub direction: u8,
    pub spawn_on: u8,
    pub spawn_as_child: bool,
    pub number_of_spawns: i32,
    pub normal_offset: f32,
    pub stick_to_big_targets: bool,
    pub stick_to_all_targets: bool,
    pub remove_scripts: bool,
    pub scale_stacks: bool,
    pub scale_stack_m: f32,
    pub scale_from_damage: f32,
    pub stacks: i32,
}

/// One static collider of a prepared map.
#[derive(Clone, Debug)]
pub struct StaticCollider {
    pub shape: Shape,
    pub layer: u8,
    pub trigger: bool,
    pub damage_box: Option<DamageBoxDef>,
}

/// Derived per-map data.
#[derive(Clone, Debug, Default)]
pub struct PreparedMap {
    pub statics: Vec<StaticCollider>,
    pub grid: Grid,
}

pub struct Content {
    pub maps: Vec<MapDef>,
    pub cards: Vec<CardDef>,
    pub pool: Vec<CardId>,
    pub objects: Vec<ObjectDef>,
    pub projectiles: Vec<ProjectileDef>,
    pub tuning: Tuning,
    /// Problems found while resolving references (unknown prefab names ...).
    pub warnings: Vec<String>,
    prepared: Vec<PreparedMap>,
    card_spawns: Vec<Vec<SpawnEntry>>,
    card_block_spawns: Vec<Vec<ObjectId>>,
    card_player_object: Vec<Option<ObjectId>>,
    object_names: BTreeMap<String, ObjectId>,
    card_names: BTreeMap<String, CardId>,
    /// Compiled prefabs (`crate::script`), one per object.
    prefabs: Vec<crate::script::Prefab>,
    /// Projectile add-on behaviours per object.
    addon_defs: Vec<Vec<crate::script::addons::AddonDef>>,
    /// SoundEvent id -> interned cue name.
    sound_events: BTreeMap<String, &'static str>,
}

impl Content {
    pub fn new(defs: ContentDefs) -> Content {
        let ContentDefs {
            maps,
            cards,
            pool,
            objects,
            mut projectiles,
            tuning,
            sound_events,
        } = defs;
        let sound_events = crate::script::sounds::register(&sound_events);
        if projectiles.is_empty() {
            projectiles.push(ProjectileDef::default());
        }
        let mut warnings = Vec::new();
        let mut object_names = BTreeMap::new();
        for (i, o) in objects.iter().enumerate() {
            object_names
                .entry(o.name.clone())
                .or_insert(ObjectId(i as u16));
        }
        let mut card_names = BTreeMap::new();
        for (i, c) in cards.iter().enumerate() {
            card_names.entry(c.name.clone()).or_insert(CardId(i as u16));
        }
        let mut resolve = |name: &Option<String>, ctx: &str| -> Option<ObjectId> {
            let n = name.as_ref()?;
            match object_names.get(n) {
                Some(id) => Some(*id),
                None => {
                    warnings.push(format!("{ctx}: unknown prefab {n:?}"));
                    None
                }
            }
        };
        let mut card_spawns = Vec::new();
        let mut card_block_spawns = Vec::new();
        let mut card_player_object = Vec::new();
        for c in &cards {
            let entries = c
                .objects_to_spawn
                .iter()
                .map(|e| SpawnEntry {
                    effect: resolve(&e.effect, &c.name),
                    add_to_projectile: resolve(&e.add_to_projectile, &c.name),
                    direction: e.direction,
                    spawn_on: e.spawn_on,
                    spawn_as_child: e.spawn_as_child,
                    number_of_spawns: e.number_of_spawns,
                    normal_offset: e.normal_offset,
                    stick_to_big_targets: e.stick_to_big_targets,
                    stick_to_all_targets: e.stick_to_all_targets,
                    remove_scripts: e.remove_scripts_from_projectile_object,
                    scale_stacks: e.scale_stacks,
                    scale_stack_m: e.scale_stack_m,
                    scale_from_damage: e.scale_from_damage,
                    stacks: e.stacks,
                })
                .collect();
            card_spawns.push(entries);
            let bs = c
                .block
                .as_ref()
                .map(|b| {
                    b.objects_to_spawn
                        .iter()
                        .filter_map(|n| resolve(&Some(n.clone()), &c.name))
                        .collect()
                })
                .unwrap_or_default();
            card_block_spawns.push(bs);
            card_player_object.push(
                c.character
                    .as_ref()
                    .and_then(|ch| resolve(&ch.add_object_to_player, &c.name)),
            );
        }
        let pool = if pool.is_empty() {
            (0..cards.len() as u16).map(CardId).collect()
        } else {
            pool.into_iter().map(CardId).collect()
        };
        let prepared = maps.iter().map(prepare_map).collect();
        // Prefab components (M5 scripts).
        let mut object_ids = BTreeMap::new();
        for (i, o) in objects.iter().enumerate() {
            if !o.id.is_empty() {
                object_ids.entry(o.id.clone()).or_insert(ObjectId(i as u16));
            }
        }
        let mut projectile_ids = BTreeMap::new();
        for (i, p) in projectiles.iter().enumerate() {
            if !p.id.is_empty() {
                projectile_ids.entry(p.id.clone()).or_insert(i as u16);
            }
        }
        let ids = crate::script::compile::Ids {
            objects: &object_ids,
            projectiles: &projectile_ids,
            sounds: &sound_events,
        };
        let prefabs = objects
            .iter()
            .map(|o| crate::script::compile::compile(o, &ids, &mut warnings))
            .collect();
        let addon_defs = objects
            .iter()
            .map(|o| crate::script::compile::compile_addons_with(o, &ids))
            .collect();
        Content {
            maps,
            cards,
            pool,
            objects,
            projectiles,
            tuning,
            warnings,
            prepared,
            card_spawns,
            card_block_spawns,
            card_player_object,
            object_names,
            card_names,
            prefabs,
            addon_defs,
            sound_events,
        }
    }

    /// Cue name (SoundEvent asset name) of a SoundEvent id from the import.
    pub fn sound_event_name(&self, id: &str) -> Option<&'static str> {
        self.sound_events.get(id).copied()
    }

    /// The compiled prefab of an object.
    pub fn prefab(&self, id: ObjectId) -> &crate::script::Prefab {
        &self.prefabs[id.0 as usize]
    }
    /// Projectile add-on behaviours of an object used as `AddToProjectile`.
    pub fn addons(&self, id: ObjectId) -> &[crate::script::addons::AddonDef] {
        &self.addon_defs[id.0 as usize]
    }

    pub fn map(&self, id: MapId) -> &MapDef {
        &self.maps[id.0 as usize]
    }
    pub fn prepared(&self, id: MapId) -> &PreparedMap {
        &self.prepared[id.0 as usize]
    }
    pub fn card(&self, id: CardId) -> &CardDef {
        &self.cards[id.0 as usize]
    }
    pub fn card_spawns(&self, id: CardId) -> &[SpawnEntry] {
        &self.card_spawns[id.0 as usize]
    }
    pub fn card_block_spawns(&self, id: CardId) -> &[ObjectId] {
        &self.card_block_spawns[id.0 as usize]
    }
    pub fn card_player_object(&self, id: CardId) -> Option<ObjectId> {
        self.card_player_object[id.0 as usize]
    }
    pub fn object(&self, id: ObjectId) -> &ObjectDef {
        &self.objects[id.0 as usize]
    }
    pub fn object_by_name(&self, name: &str) -> Option<ObjectId> {
        self.object_names.get(name).copied()
    }
    pub fn card_by_name(&self, name: &str) -> Option<CardId> {
        self.card_names.get(name).copied()
    }
    /// Total pool weight (cards.md 3.3).
    pub fn pool_weight(&self) -> f32 {
        self.pool
            .iter()
            .map(|c| self.card(*c).rarity.weight())
            .sum()
    }
}

fn prepare_map(m: &MapDef) -> PreparedMap {
    let statics: Vec<StaticCollider> = m
        .statics
        .iter()
        .map(|s| StaticCollider {
            shape: s.collider.shape.clone(),
            layer: s.collider.layer,
            trigger: s.collider.trigger,
            damage_box: s.damage_box,
        })
        .collect();
    let grid = Grid::build(statics.iter().map(|s| s.shape.aabb()).collect(), 6.0);
    PreparedMap { statics, grid }
}
