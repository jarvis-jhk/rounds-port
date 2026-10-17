//! Card and spawnable-object definitions (cards.md 2, weapons.md 18-19).

use crate::stats::{BlockCardStats, CharacterStats, GunStats, Rgba, de_bool};
use serde::{Deserialize, Serialize};

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(from = "u8", into = "u8")]
pub enum Rarity {
    #[default]
    Common,
    Uncommon,
    Rare,
}

impl From<u8> for Rarity {
    fn from(v: u8) -> Self {
        match v {
            0 => Rarity::Common,
            1 => Rarity::Uncommon,
            _ => Rarity::Rare,
        }
    }
}
impl From<Rarity> for u8 {
    fn from(r: Rarity) -> u8 {
        r as u8
    }
}

impl Rarity {
    /// Draw weight (cards.md 3.2).
    pub fn weight(self) -> f32 {
        match self {
            Rarity::Common => 10.0,
            Rarity::Uncommon => 4.0,
            Rarity::Rare => 1.0,
        }
    }
}

/// One display line of the card face (`cardStats[]`; UI only).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CardStatLine {
    #[serde(deserialize_with = "de_bool")]
    pub positive: bool,
    pub amount: String,
    #[serde(rename = "simepleAmount")]
    pub simple_amount: u8,
    pub stat: String,
}

/// `ObjectsToSpawn` entry (weapons.md 18). Prefab references by name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ObjectsToSpawnDef {
    /// Prefab spawned at the impact point.
    pub effect: Option<String>,
    /// Prefab attached to every projectile.
    #[serde(rename = "AddToProjectile")]
    pub add_to_projectile: Option<String>,
    /// 0 forward, 1 normal, 2 identity.
    pub direction: u8,
    /// 0 all, 1 player, 2 notPlayer.
    #[serde(rename = "spawnOn")]
    pub spawn_on: u8,
    #[serde(rename = "spawnAsChild", deserialize_with = "de_bool")]
    pub spawn_as_child: bool,
    #[serde(rename = "numberOfSpawns")]
    pub number_of_spawns: i32,
    #[serde(rename = "normalOffset")]
    pub normal_offset: f32,
    #[serde(rename = "stickToBigTargets", deserialize_with = "de_bool")]
    pub stick_to_big_targets: bool,
    #[serde(rename = "stickToAllTargets", deserialize_with = "de_bool")]
    pub stick_to_all_targets: bool,
    #[serde(rename = "zeroZ", deserialize_with = "de_bool")]
    pub zero_z: bool,
    #[serde(
        rename = "removeScriptsFromProjectileObject",
        deserialize_with = "de_bool"
    )]
    pub remove_scripts_from_projectile_object: bool,
    #[serde(rename = "scaleStacks", deserialize_with = "de_bool")]
    pub scale_stacks: bool,
    #[serde(rename = "scaleStackM")]
    pub scale_stack_m: f32,
    #[serde(rename = "scaleFromDamage")]
    pub scale_from_damage: f32,
    pub stacks: i32,
}

impl Default for ObjectsToSpawnDef {
    fn default() -> Self {
        ObjectsToSpawnDef {
            effect: None,
            add_to_projectile: None,
            direction: 0,
            spawn_on: 0,
            spawn_as_child: false,
            number_of_spawns: 1,
            normal_offset: 0.0,
            stick_to_big_targets: false,
            stick_to_all_targets: false,
            zero_z: false,
            remove_scripts_from_projectile_object: false,
            scale_stacks: false,
            scale_stack_m: 0.5,
            scale_from_damage: 0.0,
            stacks: 0,
        }
    }
}

/// A behaviour component of a prefab (one Unity `MonoBehaviour`): the script
/// class name plus its serialized fields as the importer read them. The sim
/// compiles these into its own component kinds (`crate::script`), resolving
/// `{"$ref": id}` references against [`EffectDesc::id`], [`NodeDef::id`],
/// [`ObjectDef::id`] and [`ProjectileDef::id`]. Unknown scripts are ignored.
///
/// Adapter contract (M5): emit **every** component of every node of the
/// prefab (including `Explosion`, `RemoveAfterSeconds`, `Gun`, sound players,
/// UnityEvent senders), in component order, with `params` = the raw Unity
/// fields (UnityEvents as `{"m_PersistentCalls": {"m_Calls": [...]}}` with
/// `m_Target: {"$ref": component-or-gameobject id}`), `id` = the component's
/// object id, and `node` = index into [`ObjectDef::nodes`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EffectDesc {
    /// Script class name, e.g. `"Saw"`, `"Homing"`, `"DelayEvent"`.
    pub script: String,
    /// Path of the owning object inside the prefab ("" = root, "Saw/Blade").
    /// Used only when `node` is None.
    #[serde(default)]
    pub path: String,
    /// Serialized fields as the importer read them.
    #[serde(default)]
    pub params: serde_json::Value,
    /// The component's object id (`"<file>:<path id>"`), target of UnityEvent
    /// calls and component references. Empty = not referenceable.
    #[serde(default)]
    pub id: String,
    /// Index of the owning node in [`ObjectDef::nodes`] (None: use `path`).
    #[serde(default)]
    pub node: Option<u16>,
    /// `Behaviour.enabled` (`m_Enabled`; the adapter passes the imported
    /// flag). None = unknown (hand-built content): the sim then treats a
    /// component as initially disabled iff some UnityEvent of the same
    /// prefab enables it (`set_enabled(true)`), see cards.md 5.2.
    #[serde(default)]
    pub enabled: Option<bool>,
}

/// One GameObject of a prefab hierarchy (index 0 = the root).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NodeDef {
    /// GameObject id (`"<file>:<path id>"`), target of `SetActive` calls and
    /// GameObject references (e.g. `AbyssalCountdown.abyssalObjects`).
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// Parent node index (None for the root).
    #[serde(default)]
    pub parent: Option<u16>,
    /// `activeSelf`.
    #[serde(default = "yes")]
    pub active: bool,
    /// Local position (x, y).
    #[serde(default)]
    pub pos: crate::math::V2,
    /// Local scale x.
    #[serde(default = "one")]
    pub scale: f32,
    /// Id of the node's `Transform` component: Transform references in
    /// `params` (`Saw.sparkTransform`, `LineEffect.fromPos`, ...) resolve
    /// to this node. Empty = unknown.
    #[serde(default)]
    pub transform: String,
}

impl Default for NodeDef {
    fn default() -> Self {
        NodeDef {
            id: String::new(),
            name: String::new(),
            parent: None,
            active: true,
            pos: crate::math::V2::ZERO,
            scale: 1.0,
            transform: String::new(),
        }
    }
}

/// A card (`CardInfo` + its stat containers).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CardDef {
    pub name: String,
    /// Source card prefab id in the import (card art lookup); empty for
    /// hand-built content.
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub description: String,
    pub rarity: Rarity,
    #[serde(default)]
    pub color_theme: u8,
    #[serde(default)]
    pub card_color: Rgba,
    #[serde(default = "yes")]
    pub allow_multiple: bool,
    /// Category names.
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub blacklisted_categories: Vec<String>,
    #[serde(default)]
    pub sound_disable_block_basic: bool,
    #[serde(default)]
    pub affects_block: bool,
    #[serde(default)]
    pub stats: Vec<CardStatLine>,
    /// The card's `Gun` component (also carries the ammo/reload adders).
    #[serde(default)]
    pub gun: Option<GunStats>,
    /// The card gun's `objectsToSpawn` list.
    #[serde(default)]
    pub objects_to_spawn: Vec<ObjectsToSpawnDef>,
    /// First `CharacterStatModifiers` on the card object.
    #[serde(default)]
    pub character: Option<CharacterStats>,
    /// First `Block` in the card's children.
    #[serde(default)]
    pub block: Option<BlockCardStats>,
}

fn yes() -> bool {
    true
}

/// Parameters of an `Explosion` component (weapons.md 19).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExplosionDef {
    pub damage: f32,
    pub force: f32,
    pub range: f32,
    pub slow: f32,
    #[serde(rename = "fastSlow", deserialize_with = "de_bool")]
    pub fast_slow: bool,
    pub silence: f32,
    pub stun: f32,
    #[serde(rename = "objectForceMultiplier")]
    pub object_force_multiplier: f32,
    #[serde(rename = "forceIgnoreMass", deserialize_with = "de_bool")]
    pub force_ignore_mass: bool,
    #[serde(rename = "flyingFor")]
    pub flying_for: f32,
    #[serde(rename = "ignoreTeam", deserialize_with = "de_bool")]
    pub ignore_team: bool,
    #[serde(rename = "ignoreWalls", deserialize_with = "de_bool")]
    pub ignore_walls: bool,
    #[serde(rename = "staticRangeMultiplier", deserialize_with = "de_bool")]
    pub static_range_multiplier: bool,
    #[serde(rename = "locallySimulated", deserialize_with = "de_bool")]
    pub locally_simulated: bool,
    pub immunity: f32,
    #[serde(deserialize_with = "de_bool")]
    pub auto: bool,
    #[serde(rename = "scaleDmg", deserialize_with = "de_bool")]
    pub scale_dmg: bool,
    #[serde(rename = "scaleForce", deserialize_with = "de_bool")]
    pub scale_force: bool,
    #[serde(rename = "scaleSlow", deserialize_with = "de_bool")]
    pub scale_slow: bool,
    #[serde(rename = "scaleSilence", deserialize_with = "de_bool")]
    pub scale_silence: bool,
    #[serde(rename = "scaleStun", deserialize_with = "de_bool")]
    pub scale_stun: bool,
    #[serde(rename = "scaleRadius", deserialize_with = "de_bool")]
    pub scale_radius: bool,
}

impl Default for ExplosionDef {
    fn default() -> Self {
        ExplosionDef {
            damage: 25.0,
            force: 2000.0,
            range: 2.0,
            slow: 0.0,
            fast_slow: false,
            silence: 0.0,
            stun: 0.0,
            object_force_multiplier: 1.0,
            force_ignore_mass: false,
            flying_for: 0.0,
            ignore_team: false,
            ignore_walls: false,
            static_range_multiplier: false,
            locally_simulated: false,
            immunity: 0.0,
            auto: true,
            scale_dmg: true,
            scale_force: true,
            scale_slow: true,
            scale_silence: true,
            scale_stun: true,
            scale_radius: true,
        }
    }
}

/// A spawnable prefab: hit-spawned objects, block-spawned objects, effect
/// objects added to players, projectile add-ons.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ObjectDef {
    pub name: String,
    /// Root local scale x of the prefab.
    #[serde(default = "one")]
    pub scale: f32,
    /// Explosion on the prefab root. Shorthand for hand-built content: the
    /// adapter should pass the `Explosion` component in `effects` instead
    /// (an `Explosion` effect on the root replaces this field).
    #[serde(default)]
    pub explosion: Option<ExplosionDef>,
    /// `RemoveAfterSeconds.seconds` (game time) on the root. Shorthand like
    /// `explosion` (a `RemoveAfterSeconds` effect on the root replaces it).
    #[serde(default)]
    pub remove_after: Option<f32>,
    /// Behaviour components (see [`EffectDesc`] for the adapter contract).
    #[serde(default)]
    pub effects: Vec<EffectDesc>,
    /// Root GameObject id (`"<file>:<path id>"`); prefab references inside
    /// `params` (`SpawnObjects.objectToSpawn`, ...) resolve through it.
    #[serde(default)]
    pub id: String,
    /// The prefab's GameObjects, parents before children, `nodes[0]` = root.
    /// Empty = a single root node of scale `scale`.
    #[serde(default)]
    pub nodes: Vec<NodeDef>,
}

impl Default for ObjectDef {
    fn default() -> Self {
        ObjectDef {
            name: String::new(),
            scale: 1.0,
            explosion: None,
            remove_after: None,
            effects: vec![],
            id: String::new(),
            nodes: vec![],
        }
    }
}

fn one() -> f32 {
    1.0
}

impl ObjectDef {
    pub fn explosion(name: &str, e: ExplosionDef) -> ObjectDef {
        ObjectDef {
            name: name.into(),
            scale: 1.0,
            explosion: Some(e),
            remove_after: Some(1.0),
            ..ObjectDef::default()
        }
    }
}

/// A projectile prefab (`Bullet_Base` and friends; weapons.md 2, 8).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectileDef {
    pub name: String,
    /// Root GameObject id (`"<file>:<path id>"`): effect guns
    /// (`Gun.projectiles[].objectToSpawn`) reference projectiles by it.
    #[serde(default)]
    pub id: String,
    /// `MoveTransform.gravity`.
    pub gravity: f32,
    /// Initial local force z = start speed.
    pub speed: f32,
    /// Initial world force (y component; x usually 0).
    pub world_force: crate::math::V2,
    pub damage: f32,
    pub force: f32,
    pub hold_player_for: f32,
    pub lifetime: f32,
    pub deal_damage_multiplier: f32,
    pub shake: f32,
    pub can_push_box: bool,
    pub destroy_on_block: bool,
    pub full_self_damage: bool,
    /// `bulletImmunity` name (EMP), empty = none.
    pub bullet_immunity: String,
    /// Bullet-vs-bullet collision circle (radius before damage scaling); None
    /// if the prefab has no collision child.
    pub collision_radius: Option<f32>,
    /// Behaviour components of the prefab beyond the standard set (M5).
    pub effects: Vec<EffectDesc>,
    /// `RayCastTrail.mask`: layers the bullet collides with (weapons.md 2).
    #[serde(default = "bullet_walls")]
    pub wall_mask: u32,
    /// `RayCastTrail.ignoreWallsMask`: the mask used with `ignoreWalls`.
    #[serde(default = "bullet_ignore_walls")]
    pub ignore_walls_mask: u32,
}

fn bullet_walls() -> u32 {
    crate::geom::layers::BULLET_WALLS
}
fn bullet_ignore_walls() -> u32 {
    crate::geom::layers::BULLET_IGNORE_WALLS
}

impl Default for ProjectileDef {
    /// The standard bullet (asset values quoted in weapons.md 2).
    fn default() -> Self {
        ProjectileDef {
            name: "Bullet_Base".into(),
            id: String::new(),
            gravity: 100.0,
            speed: 50.0,
            world_force: crate::math::v2(0.0, 5.0),
            damage: 55.0,
            force: 5000.0,
            hold_player_for: 0.3,
            lifetime: 15.0,
            deal_damage_multiplier: 1.0,
            shake: 0.8,
            can_push_box: true,
            destroy_on_block: false,
            full_self_damage: false,
            bullet_immunity: String::new(),
            collision_radius: Some(0.32),
            effects: vec![],
            wall_mask: bullet_walls(),
            ignore_walls_mask: bullet_ignore_walls(),
        }
    }
}
