//! The on-disk import format (see `docs/formats/import.md`). These types are
//! written by the importer and read back by [`crate::GameData`].
//!
//! Every imported Unity object is identified by an [`ObjId`] string
//! `"<file>:<path id>"`, e.g. `"sharedassets0.assets:2375"`. In raw field
//! dumps (`fields`), object references are written as `{"$ref": "<id>"}` and
//! null references as `null`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Version of the format; bump on incompatible changes (re-import).
pub const FORMAT_VERSION: u32 = 1;

/// Additive revision within [`FORMAT_VERSION`]: new optional fields and
/// files. An import with an older revision still loads (the new data is
/// empty), and [`crate::import`] re-imports it.
///
/// - 0: the first release of version 1 (+ `main.json`, `materials.json`,
///   `post_profiles.json`);
/// - 1: `Component::enabled`, `Object::transform`, card category names and
///   localisation keys, `sounds.json`, `categories.json`,
///   `localization.json`, `animations.json`, prefab references followed 4
///   levels deep;
/// - 2: typed particle systems (`Object::particles`, every map, prefab
///   and the main scene), the `Skin_Player*` prefabs
///   (`Player::skin_prefabs`), typed font tables (`Font::info`) with
///   repaired whitespace glyph rects, Animator transitions and parameters,
///   `ProjectSettings::gravity_3d`;
/// - 3: `faces.json` (character creator catalog and presets) and the
///   `CharacterItem` prefabs under `prefabs/`.
pub const FORMAT_REVISION: u32 = 3;

/// Global object identifier `"<file>:<path id>"`.
pub type ObjId = String;

/// Raw field values of a component (Unity field names), references as `{"$ref": id}`.
pub type Fields = serde_json::Value;

/// `index.json`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Index {
    /// [`FORMAT_VERSION`] at import time.
    pub format_version: u32,
    /// [`FORMAT_REVISION`] at import time (0 when absent).
    #[serde(default)]
    pub format_revision: u32,
    /// Unity version of the source data.
    pub unity_version: String,
    /// Fingerprint of the source files (sizes and modification times).
    pub source_fingerprint: String,
    /// Scenes in build order.
    pub scenes: Vec<SceneEntry>,
    /// Sorting layers in draw order.
    pub sorting_layers: Vec<SortingLayer>,
    /// Physics/render layer names by index.
    pub layers: Vec<String>,
    /// Tag names (index 0 = "Untagged", ...).
    pub tags: Vec<String>,
    /// Fixed timestep and other project settings of interest.
    pub settings: ProjectSettings,
    /// IDs of the player prefab(s).
    pub player_prefabs: Vec<ObjId>,
    /// Object counts per output kind.
    pub counts: BTreeMap<String, usize>,
}

/// A scene from the build settings.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SceneEntry {
    /// Build index (`levelN`).
    pub build_index: usize,
    /// Scene name, e.g. `Balance_M`.
    pub name: String,
    /// Asset path in the original project.
    pub path: String,
    /// Map file relative to the import dir (maps only).
    pub map_file: Option<String>,
}

/// A sorting layer.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SortingLayer {
    /// Layer name.
    pub name: String,
    /// The ID renderers refer to.
    pub id: u32,
}

/// Project settings relevant to the simulation.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ProjectSettings {
    /// `Time.fixedDeltaTime`.
    pub fixed_timestep: f32,
    /// `Physics2D.gravity`.
    pub gravity: [f32; 2],
    /// Raw `Physics2DSettings` fields.
    pub physics2d: Fields,
    /// 3D `Physics.gravity` (`PhysicsManager.m_Gravity`), which particle
    /// systems use with `gravity_source` 0 (revision 2).
    #[serde(default)]
    pub gravity_3d: [f32; 3],
    /// Raw `TimeManager` fields.
    pub time: Fields,
}

/// One entry of `textures.json`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Texture {
    /// Asset name.
    pub name: String,
    /// PNG file relative to the import dir (RGBA8, top row first).
    pub png: String,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Original Unity format name.
    pub format: String,
    /// 0 point, 1 bilinear, 2 trilinear.
    pub filter_mode: i32,
    /// 0 repeat, 1 clamp.
    pub wrap_mode: i32,
}

/// One entry of `sprites.json`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Sprite {
    /// Asset name.
    pub name: String,
    /// Texture (atlas page when packed).
    pub texture: Option<ObjId>,
    /// Sprite rectangle in its source image, pixels, origin bottom-left.
    pub rect: [f32; 4],
    /// Rectangle inside `texture`, pixels, origin **top-left** (PNG space).
    pub texture_rect: [f32; 4],
    /// Offset of `texture_rect` inside `rect` (trimmed packing), pixels.
    pub texture_rect_offset: [f32; 2],
    /// Pivot, normalised in `rect` (0,0 = bottom-left).
    pub pivot: [f32; 2],
    /// Pixels per world unit.
    pub pixels_per_unit: f32,
    /// 9-slice border `[left, bottom, right, top]`, pixels.
    pub border: [f32; 4],
    /// Packed into an atlas.
    pub packed: bool,
    /// Packing rotation (0 none, 1 flip H, 2 flip V, 3 rot 180, 4 rot 90).
    pub packing_rotation: u32,
    /// Physics shape outlines in world units relative to the pivot.
    pub physics_shape: Vec<Vec<[f32; 2]>>,
}

/// One entry of `audio.json`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AudioClip {
    /// Asset name.
    pub name: String,
    /// Ogg Vorbis file relative to the import dir.
    pub file: String,
    /// Channel count.
    pub channels: u32,
    /// Sample rate.
    pub frequency: u32,
    /// Length in seconds.
    pub length: f32,
    /// Samples per channel.
    pub samples: u64,
}

/// Translation, rotation, scale.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Trs {
    /// Position.
    pub position: [f32; 3],
    /// Rotation quaternion `[x, y, z, w]`.
    pub rotation: [f32; 4],
    /// Scale (for world transforms: lossy scale).
    pub scale: [f32; 3],
}

/// World transform of an object.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct WorldTransform {
    /// Position, rotation, lossy scale.
    #[serde(flatten)]
    pub trs: Trs,
    /// Rotation about Z in degrees (counter-clockwise).
    pub rotation_z: f32,
    /// 2D affine part of the world matrix `[a, b, c, d, tx, ty]`:
    /// `x' = a x + c y + tx`, `y' = b x + d y + ty` (exact for skewed hierarchies).
    pub matrix: [f32; 6],
}

/// `RectTransform` layout fields.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct RectTransform {
    /// Anchor minimum (normalised in the parent rect).
    pub anchor_min: [f32; 2],
    /// Anchor maximum.
    pub anchor_max: [f32; 2],
    /// Pivot position relative to the anchors.
    pub anchored_position: [f32; 2],
    /// Size relative to the anchors (the size when both anchors coincide).
    pub size_delta: [f32; 2],
    /// Pivot (normalised in the own rect).
    pub pivot: [f32; 2],
}

/// A sprite renderer.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SpriteRenderer {
    /// Component enabled.
    pub enabled: bool,
    /// Sprite drawn (may be missing).
    pub sprite: Option<ObjId>,
    /// Tint.
    pub color: [f32; 4],
    /// Mirror X.
    pub flip_x: bool,
    /// Mirror Y.
    pub flip_y: bool,
    /// 0 simple, 1 sliced, 2 tiled.
    pub draw_mode: i32,
    /// Size for sliced/tiled modes, world units.
    pub size: [f32; 2],
    /// Sorting layer name.
    pub sorting_layer: String,
    /// Index of the sorting layer in draw order.
    pub sorting_layer_index: usize,
    /// Order in layer.
    pub sorting_order: i32,
    /// 0 none, 1 visible inside mask, 2 visible outside mask.
    pub mask_interaction: i32,
    /// Material names.
    pub materials: Vec<String>,
}

/// Collider shape.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum Shape {
    /// `BoxCollider2D`.
    Box {
        /// Size, local units.
        size: [f32; 2],
        /// Rounded edge radius.
        edge_radius: f32,
    },
    /// `CircleCollider2D`.
    Circle {
        /// Radius, local units.
        radius: f32,
    },
    /// `CapsuleCollider2D`.
    Capsule {
        /// Size, local units.
        size: [f32; 2],
        /// 0 vertical, 1 horizontal.
        direction: i32,
    },
    /// `PolygonCollider2D`.
    Polygon {
        /// Closed paths, local units.
        paths: Vec<Vec<[f32; 2]>>,
    },
    /// `EdgeCollider2D`.
    Edge {
        /// Open polyline, local units.
        points: Vec<[f32; 2]>,
        /// Edge radius.
        edge_radius: f32,
    },
}

/// A 2D collider.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Collider {
    /// Component ID.
    pub id: ObjId,
    /// Component enabled.
    pub enabled: bool,
    /// Trigger only.
    pub is_trigger: bool,
    /// Offset in local units.
    pub offset: [f32; 2],
    /// Density (auto mass).
    pub density: f32,
    /// Physics material (friction, bounciness), if any.
    pub material: Option<PhysicsMaterial>,
    /// Geometry.
    #[serde(flatten)]
    pub shape: Shape,
}

/// `PhysicsMaterial2D`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PhysicsMaterial {
    /// Asset name.
    pub name: String,
    /// Friction.
    pub friction: f32,
    /// Bounciness.
    pub bounciness: f32,
}

/// `Rigidbody2D`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Rigidbody {
    /// Component ID.
    pub id: ObjId,
    /// 0 dynamic, 1 kinematic, 2 static.
    pub body_type: i32,
    /// Simulated flag.
    pub simulated: bool,
    /// Mass computed from collider density.
    pub use_auto_mass: bool,
    /// Mass.
    pub mass: f32,
    /// Linear drag.
    pub linear_drag: f32,
    /// Angular drag.
    pub angular_drag: f32,
    /// Gravity scale.
    pub gravity_scale: f32,
    /// Constraint bits: 1 freeze X, 2 freeze Y, 4 freeze rotation.
    pub constraints: i32,
    /// 0 none, 1 interpolate, 2 extrapolate.
    pub interpolate: i32,
    /// 0 discrete, 1 continuous.
    pub collision_detection: i32,
    /// 0 never sleep, 1 start awake, 2 start asleep.
    pub sleeping_mode: i32,
}

/// A 2D joint.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Joint {
    /// Component ID.
    pub id: ObjId,
    /// Unity class, e.g. `HingeJoint2D`.
    pub kind: String,
    /// Component enabled.
    pub enabled: bool,
    /// Connected body's Rigidbody2D ID (None = world).
    pub connected_body: Option<ObjId>,
    /// Index of the connected body's object in the same object list, if there.
    pub connected_object: Option<usize>,
    /// Anchor, local units.
    pub anchor: [f32; 2],
    /// Connected anchor (connected body's local units, or world).
    pub connected_anchor: [f32; 2],
    /// Whether the connected bodies collide.
    pub enable_collision: bool,
    /// Break force (Infinity = unbreakable, written as null).
    pub break_force: Option<f32>,
    /// All fields of the joint (limits, motors, distance, frequency, ...).
    pub fields: Fields,
}

/// A component other than Transform, in raw form.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Component {
    /// Component ID.
    pub id: ObjId,
    /// Unity class name (`MonoBehaviour` for scripts).
    #[serde(rename = "type")]
    pub kind: String,
    /// Script class (namespace-qualified) for MonoBehaviours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script: Option<String>,
    /// `m_Enabled` of Behaviours (MonoBehaviours, Animator, ...), Renderers
    /// and Colliders; None for components without the flag (revision 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Field values (omitted for bulky engine classes like ParticleSystem).
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub fields: Fields,
}

/// A GameObject in a flattened hierarchy.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Object {
    /// GameObject ID.
    pub id: ObjId,
    /// Name.
    pub name: String,
    /// Index of the parent in the same list.
    pub parent: Option<usize>,
    /// Indices of the children in order.
    pub children: Vec<usize>,
    /// `activeSelf`.
    pub active: bool,
    /// Active including all ancestors.
    pub active_in_hierarchy: bool,
    /// Layer index.
    pub layer: u32,
    /// Id of the object's `Transform`/`RectTransform` component: fields
    /// referencing a Transform (`{"$ref": id}`) point to this object
    /// (revision 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transform: Option<ObjId>,
    /// Tag name.
    pub tag: String,
    /// Local transform.
    pub local: Trs,
    /// World transform (relative to the prefab root for prefabs).
    pub world: WorldTransform,
    /// `RectTransform` layout (UI objects and rect-sized objects such as
    /// `SFLight`), absent for plain Transforms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rect: Option<RectTransform>,
    /// Sprite renderer, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sprite: Option<SpriteRenderer>,
    /// 2D colliders.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub colliders: Vec<Collider>,
    /// Rigidbody, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rigidbody: Option<Rigidbody>,
    /// 2D joints.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub joints: Vec<Joint>,
    /// Typed particle system and renderer of this object (revision 2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub particles: Option<Box<crate::particles::ParticleSystem>>,
    /// Every component except the Transform, raw.
    pub components: Vec<Component>,
}

/// `maps/<NN>_<Scene>.json`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Map {
    /// Build index.
    pub build_index: usize,
    /// Scene name.
    pub name: String,
    /// Fields of the scene's `Map` script, if present.
    pub map: Option<Fields>,
    /// Spawn points (world positions).
    pub spawn_points: Vec<SpawnPoint>,
    /// Every GameObject of the scene, parents before children.
    pub objects: Vec<Object>,
}

/// `main.json`: the persistent main scene (`level0`) with the cameras,
/// `SFRenderer`, lights, post-process volumes and the background/foreground
/// art particle systems. Unlike maps, its `ParticleSystem` and
/// `ParticleSystemRenderer` components keep their raw `fields`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Scene {
    /// Build index.
    pub build_index: usize,
    /// Scene name.
    pub name: String,
    /// Every GameObject of the scene, parents before children.
    pub objects: Vec<Object>,
}

/// A texture slot of a [`Material`].
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TexEnv {
    /// Texture, if assigned.
    pub texture: Option<ObjId>,
    /// Tiling.
    pub scale: [f32; 2],
    /// Offset.
    pub offset: [f32; 2],
}

/// One entry of `materials.json`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Material {
    /// Asset name.
    pub name: String,
    /// Shader name (`m_ParsedForm.m_Name`), empty if unresolved.
    pub shader: String,
    /// `m_CustomRenderQueue` (-1 = shader default).
    pub render_queue: i32,
    /// Enabled shader keywords.
    pub keywords: Vec<String>,
    /// Texture properties.
    pub textures: BTreeMap<String, TexEnv>,
    /// Float properties (non-finite values are written as `null` and read
    /// back as infinity).
    #[serde(deserialize_with = "lossy::floats")]
    pub floats: BTreeMap<String, f32>,
    /// Colour properties (as authored, sRGB for colours; `null` components
    /// are infinity).
    #[serde(deserialize_with = "lossy::colors")]
    pub colors: BTreeMap<String, [f32; 4]>,
}

/// Deserializers that accept `null` for non-finite floats.
mod lossy {
    use std::collections::BTreeMap;

    use serde::{Deserialize, Deserializer};

    fn f(v: Option<f32>) -> f32 {
        v.unwrap_or(f32::INFINITY)
    }

    pub fn floats<'de, D: Deserializer<'de>>(d: D) -> Result<BTreeMap<String, f32>, D::Error> {
        let m = BTreeMap::<String, Option<f32>>::deserialize(d)?;
        Ok(m.into_iter().map(|(k, v)| (k, f(v))).collect())
    }

    pub fn colors<'de, D: Deserializer<'de>>(d: D) -> Result<BTreeMap<String, [f32; 4]>, D::Error> {
        let m = BTreeMap::<String, [Option<f32>; 4]>::deserialize(d)?;
        Ok(m.into_iter().map(|(k, v)| (k, v.map(f))).collect())
    }
}

/// One entry of `post_profiles.json`: a post-processing stack profile.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PostProfile {
    /// Asset name (e.g. `Gold`, `Default`).
    pub name: String,
    /// Effect settings by script class (`ColorGrading`, `Bloom`, ...), raw
    /// fields; parameters are `{"overrideState": 0|1, "value": ...}`.
    pub effects: BTreeMap<String, Fields>,
}

/// A `SpawnPoint` script instance.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SpawnPoint {
    /// Object index.
    pub object: usize,
    /// Script `ID`.
    pub id: i64,
    /// Script `TEAMID`.
    pub team: i64,
    /// World position.
    pub position: [f32; 2],
}

/// `prefabs/<id>.json`: a prefab hierarchy (root first).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Prefab {
    /// Root GameObject ID.
    pub id: ObjId,
    /// Root name.
    pub name: String,
    /// Objects, root at index 0; world transforms are relative to the root's parent.
    pub objects: Vec<Object>,
}

/// One entry of `cards.json`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Card {
    /// GameObject ID of the card prefab.
    pub id: ObjId,
    /// GameObject name.
    pub object_name: String,
    /// `CardInfo.cardName`.
    pub name: String,
    /// `CardInfo.cardDestription`.
    pub description: String,
    /// `CardInfo.rarity`.
    pub rarity: i64,
    /// `CardInfo.colorTheme`.
    pub color_theme: i64,
    /// `CardInfo.cardArt` prefab.
    pub art: Option<ObjId>,
    /// Prefab file of the card (all components and children).
    pub prefab: String,
    /// Fields of the components by script/class name (first instance on the root).
    pub components: BTreeMap<String, Fields>,
    /// Prefabs referenced from the card's components (spawned objects, effects, art).
    pub referenced_prefabs: Vec<ObjId>,
    /// `CardInfo.categories` as category names (revision 1).
    #[serde(default)]
    pub categories: Vec<String>,
    /// `CardInfo.blacklistedCategories` as names (revision 1).
    #[serde(default)]
    pub blacklisted_categories: Vec<String>,
    /// Localisation key of the name in `StringTableCards` (e.g.
    /// `CARD_BARRAGE`), if the tables were imported (revision 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name_key: Option<String>,
    /// Localisation key of the description (revision 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description_key: Option<String>,
    /// Localisation keys of the `cardStats` lines' stat names, in order
    /// (None where unresolved; revision 1).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stat_keys: Vec<Option<String>>,
}

/// One entry of `fonts.json` (TextMeshPro font assets).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Font {
    /// MonoBehaviour ID.
    pub id: ObjId,
    /// Asset name.
    pub name: String,
    /// SDF atlas textures.
    pub atlases: Vec<ObjId>,
    /// All fields (face info, glyph and character tables, kerning, ...).
    /// Glyph rects outside the atlas (whitespace glyphs of fonts upgraded
    /// from TMP's legacy glyph list, see [`FontInfo`]) are replaced by an
    /// empty rect at 0,0 (revision 2).
    pub fields: Fields,
    /// Typed metrics, glyph, character and kerning tables (revision 2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info: Option<FontInfo>,
}

/// Typed TextMeshPro font metrics (revision 2). Units are atlas pixels at
/// `point_size`; scale by `size / point_size` for a font size.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct FontInfo {
    /// Atlas size in pixels.
    pub atlas_width: u32,
    /// See [`FontInfo::atlas_width`].
    pub atlas_height: u32,
    /// SDF spread around each glyph rect, pixels (`m_AtlasPadding`).
    pub padding: f32,
    /// `m_AtlasRenderMode` (GlyphRenderMode, e.g. 4165 SDFAA).
    pub render_mode: i64,
    /// Sampling point size of the atlas (`m_FaceInfo.m_PointSize`).
    pub point_size: f32,
    /// `m_FaceInfo.m_Scale`.
    pub scale: f32,
    /// Line metrics (`m_FaceInfo`): line height, ascent (> 0), descent
    /// (< 0), baseline, cap and mean line, underline offset/thickness, tab
    /// width.
    pub line_height: f32,
    /// See [`FontInfo::line_height`].
    pub ascent: f32,
    /// See [`FontInfo::line_height`].
    pub descent: f32,
    /// See [`FontInfo::line_height`].
    pub baseline: f32,
    /// See [`FontInfo::line_height`].
    pub cap_line: f32,
    /// See [`FontInfo::line_height`].
    pub mean_line: f32,
    /// See [`FontInfo::line_height`].
    pub underline_offset: f32,
    /// See [`FontInfo::line_height`].
    pub underline_thickness: f32,
    /// See [`FontInfo::line_height`].
    pub tab_width: f32,
    /// Bold style weight (`boldStyle`) and extra bold spacing (`boldSpacing`).
    pub bold_style: f32,
    /// See [`FontInfo::bold_style`].
    pub bold_spacing: f32,
    /// Glyphs by glyph index order of `m_GlyphTable`.
    pub glyphs: Vec<FontGlyph>,
    /// Character map: unicode code point -> glyph index.
    pub characters: BTreeMap<u32, u32>,
    /// Pair adjustments (kerning) by glyph index.
    pub kerning: Vec<FontKerning>,
}

/// One glyph of a [`FontInfo`].
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct FontGlyph {
    /// Glyph index (`m_Index`, the target of [`FontInfo::characters`]).
    pub index: u32,
    /// Atlas texture index (into [`Font::atlases`]).
    pub atlas: u32,
    /// Rect in the atlas, pixels, origin **bottom-left** as in TMP
    /// (`[x, y, w, h]`, padding not included). Always inside the atlas;
    /// `[0, 0, 0, 0]` for glyphs without pixels (whitespace).
    pub rect: [f32; 4],
    /// The same rect in PNG space (origin top-left).
    pub rect_png: [f32; 4],
    /// Glyph metrics: size, bearing from the pen position (x right, y up
    /// from the baseline to the glyph's top) and advance.
    pub width: f32,
    /// See [`FontGlyph::width`].
    pub height: f32,
    /// See [`FontGlyph::width`].
    pub bearing_x: f32,
    /// See [`FontGlyph::width`].
    pub bearing_y: f32,
    /// See [`FontGlyph::width`].
    pub advance: f32,
    /// Glyph scale (`m_Scale`).
    pub scale: f32,
}

/// A glyph pair adjustment (`m_FontFeatureTable.m_GlyphPairAdjustmentRecords`):
/// value records `[x placement, y placement, x advance, y advance]`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct FontKerning {
    /// First glyph index.
    pub first: u32,
    /// Second glyph index.
    pub second: u32,
    /// Adjustment of the first glyph.
    pub first_adjust: [f32; 4],
    /// Adjustment of the second glyph.
    pub second_adjust: [f32; 4],
}

impl FontInfo {
    /// The glyph of a character, if mapped.
    pub fn glyph_for(&self, c: char) -> Option<&FontGlyph> {
        let gi = *self.characters.get(&(c as u32))?;
        self.glyphs.iter().find(|g| g.index == gi)
    }

    /// Kerning: the first glyph's x advance adjustment for a pair.
    pub fn kerning_x(&self, first: u32, second: u32) -> f32 {
        self.kerning
            .iter()
            .find(|k| k.first == first && k.second == second)
            .map_or(0.0, |k| k.first_adjust[2])
    }
}

/// `player.json`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Player {
    /// Player prefab IDs (see `prefabs/`).
    pub prefabs: Vec<ObjId>,
    /// `PlayerSkinBank` and `PlayerSkin` assets: ID → fields.
    pub skins: BTreeMap<ObjId, Fields>,
    /// `PlayerSkin` id -> its GameObject, exported under `prefabs/`: the
    /// `Skin_PlayerOne`..`Four` objects whose particle system fills the
    /// player's body (revision 2).
    #[serde(default)]
    pub skin_prefabs: BTreeMap<ObjId, ObjId>,
}

/// `faces.json` (revision 3): the character creator's catalog, from the main
/// scene's `CharacterCreatorItemLoader` (docs/spec/ui.md 2.6). Each item's
/// GameObject is also exported under `prefabs/` (`prefab`), with its
/// `CharacterItem` component (`id`) and `SpriteRenderer`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Faces {
    /// `CharacterCreatorItemLoader.eyes`, in list order (the item id of a
    /// face is its index).
    pub eyes: Vec<FaceItem>,
    /// `CharacterCreatorItemLoader.mouths`.
    pub mouths: Vec<FaceItem>,
    /// `CharacterCreatorItemLoader.accessories` (both detail slots).
    pub accessories: Vec<FaceItem>,
    /// `CharacterCreatorItemEquipper.defaultEyes` of the player prefab, as
    /// a `CharacterItem` id (an item of `eyes`/`mouths` when it is listed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_eyes: Option<ObjId>,
    /// `CharacterCreatorItemEquipper.defaultMouth`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_mouth: Option<ObjId>,
    /// `CharacterCreatorItemEquipper.scaleM` of the player prefab.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale_m: Option<f32>,
    /// `CharacterCreatorHandler.playerFaces`: the preset slots (the shipped
    /// data has ten, all with every id 0 and zero offsets).
    pub presets: Vec<FacePreset>,
    /// `CharacterCreatorHandler.selectedPlayerFaces`.
    pub selected_presets: Vec<FacePreset>,
    /// `CharacterCreatorHandler.selectedFaceID` and `lockedPortraits`.
    pub selected_face_id: Vec<i64>,
    pub locked_portraits: Vec<i64>,
}

/// One `CharacterItem` of [`Faces`].
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FaceItem {
    /// `CharacterItem` component id (what the loader lists reference).
    pub id: ObjId,
    /// The item's GameObject, exported under `prefabs/`.
    pub prefab: ObjId,
    /// GameObject name (`P_E_Neutral`, ...).
    pub name: String,
    /// `CharacterItem.itemType`: 0 eye, 1 mouth, 2 accessory.
    pub item_type: i32,
    /// `CharacterItem.sprite` (falls back to the `SpriteRenderer`'s).
    pub sprite: Option<ObjId>,
    /// `CharacterItem.offset`, face units.
    pub offset: [f32; 2],
    /// `CharacterItem.scale`.
    pub scale: f32,
    /// `SpriteRenderer.color` (RGBA, sRGB).
    pub color: [f32; 4],
    /// `CharacterItem.moveHealthBarUp`.
    pub move_health_bar_up: f32,
    /// Raw `CharacterItem` fields.
    pub fields: Fields,
}

/// A `CharacterCreatorHandler` face slot (`CharacterCreator.currentPlayerFace`).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct FacePreset {
    pub eye_id: i64,
    pub eye_offset: [f32; 2],
    pub mouth_id: i64,
    pub mouth_offset: [f32; 2],
    pub detail_id: i64,
    pub detail_offset: [f32; 2],
    pub detail2_id: i64,
    pub detail2_offset: [f32; 2],
}

/// `sounds.json`: the Sonigon sound data (docs/spec/audio.md 1). Sim
/// sound cues are SoundEvent names; resolve them with
/// [`SoundBank::event_by_name`] and [`SoundBank::clip_files`].
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SoundBank {
    /// `SoundEvent` assets by id.
    pub events: BTreeMap<ObjId, SoundEvent>,
    /// `SoundContainer` assets by id.
    pub containers: BTreeMap<ObjId, SoundContainer>,
    /// `SoundShotModifier` assets (gun shot sound sets, audio.md 5).
    pub shot_modifiers: BTreeMap<ObjId, SoundModifier>,
    /// `SoundImpactModifier` assets (impact sound sets).
    pub impact_modifiers: BTreeMap<ObjId, SoundModifier>,
    /// `SoundPolyGroup` assets.
    pub poly_groups: BTreeMap<ObjId, SoundPolyGroup>,
    /// `SoundMix` assets.
    pub mixes: BTreeMap<ObjId, SoundMix>,
    /// Every component field of the game data (maps, prefabs, main scene,
    /// shared assets) that references a SoundEvent or a sound modifier:
    /// `"<script>.<field path>"` (array elements as `[]`, e.g.
    /// `PlayerSounds.soundCharacterJump`,
    /// `CurveAnimation.soundPlay[].soundEvent`) -> referenced ids.
    pub references: BTreeMap<String, Vec<ObjId>>,
}

/// One container slot of a [`SoundEvent`] (all play together, layered).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SoundEventLayer {
    /// The `SoundContainer`.
    pub container: ObjId,
    /// Timeline delay in seconds.
    pub delay: f32,
    /// Timeline volume ratio.
    pub volume_ratio: f32,
}

/// Events triggered by a [`SoundEvent`] (on play / stop / tail).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SoundTrigger {
    /// `SoundEvent` ids (null entries dropped).
    pub events: Vec<ObjId>,
    /// `...WhichToPlayEnum` (audio.md 1.1: play one at random / all).
    pub which: i64,
}

/// A `SoundEvent` (name ends in `_SE`).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SoundEvent {
    /// Asset name (the sim's cue name).
    pub name: String,
    /// Containers with their timeline settings.
    pub layers: Vec<SoundEventLayer>,
    /// Minimum seconds between two accepted plays (0 = none).
    pub cooldown: f32,
    /// Chance in percent that a play request plays.
    pub probability: f32,
    /// `disableEnable` flag of the event.
    pub disabled: bool,
    /// Shared polyphony group and this event's priority in it.
    pub poly_group: Option<ObjId>,
    /// `soundPolyGroupPriority`.
    pub poly_group_priority: f32,
    /// `SoundMix`.
    pub mix: Option<ObjId>,
    /// Event modifier (raw `soundEventModifier`: values with `...Use` flags,
    /// e.g. `polyphony`/`polyphonyUse`, `volumeDecibel`/`volumeUse`).
    pub modifier: Fields,
    /// Trigger-on-play events, if enabled.
    pub trigger_on_play: Option<SoundTrigger>,
    /// Trigger-on-stop events, if enabled.
    pub trigger_on_stop: Option<SoundTrigger>,
    /// Trigger-on-tail events, if enabled.
    pub trigger_on_tail: Option<SoundTrigger>,
    /// Seconds before the end of the longest clip at which the tail fires.
    pub tail_length: f32,
    /// All raw `variables` of the event.
    pub fields: Fields,
}

/// A `SoundContainer` (name ends in `_SC`).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SoundContainer {
    /// Asset name.
    pub name: String,
    /// AudioClip ids (keys of `audio.json`).
    pub clips: Vec<ObjId>,
    /// Their Ogg files relative to the import dir (clips that failed to
    /// convert are left out).
    pub files: Vec<String>,
    /// Loops until stopped.
    pub looping: bool,
    /// Base volume in dB and as a ratio.
    pub volume_db: f32,
    /// See [`SoundContainer::volume_db`].
    pub volume_ratio: f32,
    /// Random extra attenuation range in dB when enabled (e.g. -3).
    pub volume_random_db: Option<f32>,
    /// Static pitch in semitones and as a ratio.
    pub pitch_semitone: f32,
    /// See [`SoundContainer::pitch_semitone`].
    pub pitch_ratio: f32,
    /// Random pitch range (+-semitones) when enabled.
    pub pitch_random_semitone: Option<f32>,
    /// 0 = 2D, 1 = 3D.
    pub spatial_blend: f32,
    /// Distance attenuation on.
    pub distance_enabled: bool,
    /// Distance scale.
    pub distance_scale: f32,
    /// Voice priority (higher survives).
    pub priority: f32,
    /// Starts at a random position (loops).
    pub random_start_position: bool,
    /// All raw `setting` fields (intensity curves, crossfades, effects, ...).
    pub settings: Fields,
}

/// A `SoundShotModifier` / `SoundImpactModifier`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SoundModifier {
    /// Asset name (e.g. `Basic_SoundShotModifier`).
    pub name: String,
    /// Selection priority (audio.md 5).
    pub priority: i64,
    /// SoundEvent per slot (`single`, `singleAutoLoop`, `singleAutoTail`,
    /// `shotgun`, `shotgunAutoLoop`, `shotgunAutoTail` /
    /// `impactCharacter`, `impactEnvironment`); unset slots are absent.
    pub events: BTreeMap<String, ObjId>,
}

/// A `SoundPolyGroup`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SoundPolyGroup {
    /// Asset name.
    pub name: String,
    /// Limit, if enabled.
    pub limit: Option<i64>,
    /// `skipLowerPriority`.
    pub skip_lower_priority: bool,
    /// `allowFadeOut`.
    pub allow_fade_out: bool,
}

/// A `SoundMix`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SoundMix {
    /// Asset name.
    pub name: String,
    /// Parent mix.
    pub parent: Option<ObjId>,
    /// Raw `soundEventModifier`.
    pub modifier: Fields,
}

impl SoundBank {
    /// A SoundEvent by asset name (= sim cue name), with its id.
    pub fn event_by_name(&self, name: &str) -> Option<(&ObjId, &SoundEvent)> {
        self.events.iter().find(|(_, e)| e.name == name)
    }

    /// Name -> id of every event (build once for frequent lookups).
    pub fn event_ids_by_name(&self) -> BTreeMap<&str, &ObjId> {
        self.events
            .iter()
            .map(|(id, e)| (e.name.as_str(), id))
            .collect()
    }

    /// The Ogg files of every container of an event, per layer.
    pub fn clip_files(&self, event: &SoundEvent) -> Vec<&[String]> {
        event
            .layers
            .iter()
            .filter_map(|l| self.containers.get(&l.container))
            .map(|c| c.files.as_slice())
            .collect()
    }
}

/// `localization.json`: Unity Localization string tables of every locale
/// (docs/spec/ui.md 7).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Localization {
    /// Locale codes found (e.g. `en-US`, `fr`, `zh`), sorted.
    pub locales: Vec<String>,
    /// Tables by collection name (`StringTableDefault`, `StringTableCards`).
    pub tables: BTreeMap<String, StringTable>,
}

/// One string table collection.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct StringTable {
    /// Collection GUID (32 hex digits, as in `"GUID:<hex>"` references).
    pub guid: String,
    /// Key id -> key name (the shared table data).
    pub keys: BTreeMap<i64, String>,
    /// Locale -> key name -> text (smart-string syntax and TMP tags kept).
    pub strings: BTreeMap<String, BTreeMap<String, String>>,
}

impl StringTable {
    /// Key name for a reference by name (non-empty) or id.
    pub fn key<'a>(&'a self, key: &'a str, id: i64) -> Option<&'a str> {
        if !key.is_empty() {
            return Some(key);
        }
        self.keys.get(&id).map(String::as_str)
    }

    /// Text of `key` in `locale`, falling back to `en-US`.
    pub fn text(&self, locale: &str, key: &str) -> Option<&str> {
        self.strings
            .get(locale)
            .and_then(|t| t.get(key))
            .or_else(|| self.strings.get("en-US").and_then(|t| t.get(key)))
            .map(String::as_str)
    }
}

impl Localization {
    /// A table by collection name or `"GUID:<hex>"` reference.
    pub fn table(&self, reference: &str) -> Option<&StringTable> {
        if let Some(g) = reference.strip_prefix("GUID:") {
            let g = g.replace('-', "").to_ascii_lowercase();
            return self.tables.values().find(|t| t.guid == g);
        }
        self.tables.get(reference)
    }

    /// Text of `key` in table `table` (name or GUID reference), falling back
    /// to `en-US`.
    pub fn text(&self, locale: &str, table: &str, key: &str) -> Option<&str> {
        self.table(table)?.text(locale, key)
    }

    /// Resolves a raw `LocalizedString` field as imported
    /// (`m_TableReference.m_TableCollectionName`,
    /// `m_TableEntryReference.m_Key` / `m_KeyId`).
    pub fn resolve(&self, locale: &str, localized: &Fields) -> Option<&str> {
        let t = self.table(
            localized
                .pointer("/m_TableReference/m_TableCollectionName")?
                .as_str()?,
        )?;
        let e = localized.get("m_TableEntryReference")?;
        let key = t.key(
            e.get("m_Key").and_then(|k| k.as_str()).unwrap_or(""),
            e.get("m_KeyId").and_then(|k| k.as_i64()).unwrap_or(0),
        )?;
        t.text(locale, key)
    }
}

/// `animations.json`: Mecanim animation clips and controllers (revision 1),
/// e.g. the card art animations (`Animator.m_Controller` in prefabs).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Animations {
    /// `AnimationClip` assets by id.
    pub clips: BTreeMap<ObjId, AnimClip>,
    /// `AnimatorController` assets by id.
    pub controllers: BTreeMap<ObjId, AnimController>,
}

/// An animation clip as piecewise cubic curves.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AnimClip {
    /// Asset name.
    pub name: String,
    /// Start and stop time in seconds (the clip length is `stop - start`).
    pub start: f32,
    /// See [`AnimClip::start`].
    pub stop: f32,
    /// Authoring sample rate.
    pub sample_rate: f32,
    /// Loops (`m_LoopTime`).
    pub looping: bool,
    /// Animated values.
    pub curves: Vec<AnimCurve>,
    /// Objects of object-reference curves (sprite swaps): a curve's value
    /// (rounded down) indexes this list.
    pub pptr: Vec<Option<ObjId>>,
    /// Raw animation events (`m_Events`).
    pub events: Fields,
}

/// One animated value.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AnimCurve {
    /// CRC32 of the target's transform path relative to the Animator's
    /// object (names joined by `/`, `""` = the Animator's object = 0); see
    /// [`path_hash`].
    pub path: u32,
    /// Unity class id of the animated component (4 Transform, 224
    /// RectTransform, 1 GameObject, 114 MonoBehaviour, 212 SpriteRenderer,
    /// 225 CanvasGroup, ...).
    pub class_id: i32,
    /// Script class (`Namespace.Class`) for MonoBehaviour targets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script: Option<String>,
    /// Property: for Transforms `position.x|y|z`, `rotation.x|y|z|w`
    /// (quaternion), `euler.x|y|z` (degrees), `scale.x|y|z`; otherwise the
    /// serialized property path (`m_Color.a`, `m_IsActive`,
    /// `m_AnchoredPosition.x`, `m_Sprite`, ...) or `#<crc32>` if unresolved.
    pub property: String,
    /// Object-reference curve (value indexes [`AnimClip::pptr`]).
    #[serde(default)]
    pub pptr: bool,
    /// `[t0, a, b, c, d]` pieces sorted by `t0`: at time `t` use the last
    /// piece with `t0 <= t` and evaluate `((a*dt + b)*dt + c)*dt + d`,
    /// `dt = t - t0` (before the first piece: its `d`).
    pub segments: Vec<[f32; 5]>,
}

impl AnimCurve {
    /// Value at clip time `t`.
    pub fn eval(&self, t: f32) -> f32 {
        let i = self.segments.partition_point(|s| s[0] <= t);
        let Some(s) = self.segments.get(i.saturating_sub(1)) else {
            return 0.0;
        };
        let dt = (t - s[0]).max(0.0);
        ((s[1] * dt + s[2]) * dt + s[3]) * dt + s[4]
    }
}

/// Unity's hash of a transform path / property name (CRC32, zlib
/// polynomial).
pub fn path_hash(s: &str) -> u32 {
    let mut c = !0u32;
    for &b in s.as_bytes() {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                (c >> 1) ^ 0xEDB8_8320
            } else {
                c >> 1
            };
        }
    }
    !c
}

/// An `AnimatorController`: its layers' state machines with transitions
/// and parameters (revision 2; the shipped card art uses one looping state
/// per controller and no transitions).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AnimController {
    /// Asset name.
    pub name: String,
    /// Every clip the controller uses.
    pub clips: Vec<ObjId>,
    /// Layers in order.
    pub layers: Vec<AnimLayer>,
    /// Parameters with their default values (revision 2).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<AnimParameter>,
}

/// An Animator parameter (revision 2).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AnimParameter {
    /// Name (or `#<hash>`).
    pub name: String,
    /// `float`, `int`, `bool` or `trigger`.
    pub kind: String,
    /// Default value (bools and triggers as 0/1).
    pub default: f32,
}

/// A state transition (revision 2). Times are normalised to the source
/// state's length unless `fixed_duration` (then `duration` is seconds).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AnimTransition {
    /// Destination state index in the layer, None for an exit or
    /// sub-state-machine selector (see `destination_raw`).
    pub destination: Option<usize>,
    /// Raw `m_DestinationState` (>= 30000: selector states).
    pub destination_raw: u32,
    /// Cross-fade duration.
    pub duration: f32,
    /// Normalised start time in the destination state.
    pub offset: f32,
    /// Normalised exit time of the source state.
    pub exit_time: f32,
    /// Only fires once the exit time has passed (and conditions hold).
    pub has_exit_time: bool,
    /// `duration` is in seconds (else normalised).
    pub fixed_duration: bool,
    /// 0 none, 1 source, 2 destination, 3 source then destination,
    /// 4 destination then source.
    pub interruption_source: i32,
    /// `m_OrderedInterruption`.
    pub ordered_interruption: bool,
    /// Any-state transitions only: may re-enter the current state.
    pub can_transition_to_self: bool,
    /// All must hold.
    pub conditions: Vec<AnimCondition>,
}

/// A transition condition (revision 2).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AnimCondition {
    /// `if`, `if_not` (bools/triggers), `greater`, `less`, `equals`,
    /// `not_equal` (numbers), `exit_time`, or `#<n>`.
    pub mode: String,
    /// Parameter name (or `#<hash>`).
    pub parameter: String,
    /// Threshold for numeric modes.
    pub threshold: f32,
}

/// A controller layer.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AnimLayer {
    /// Layer name.
    pub name: String,
    /// Weight (layer 0 always counts as 1).
    pub default_weight: f32,
    /// Index of the state entered first.
    pub default_state: usize,
    /// States of the layer's state machine.
    pub states: Vec<AnimState>,
    /// Transitions from any state (revision 2).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub any_state_transitions: Vec<AnimTransition>,
}

/// A state of a layer.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AnimState {
    /// Full path name (`Base Layer.Anim_Bg1`) or `#<hash>`.
    pub name: String,
    /// Playback speed.
    pub speed: f32,
    /// Loops.
    pub looping: bool,
    /// The clip played (first leaf of its blend tree).
    pub clip: Option<ObjId>,
    /// Number of outgoing transitions (see `outgoing`).
    pub transitions: usize,
    /// Outgoing transitions in priority order (revision 2).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outgoing: Vec<AnimTransition>,
    /// Normalised cycle offset (revision 2).
    #[serde(default)]
    pub cycle_offset: f32,
    /// Parameter multiplying `speed`, if any (revision 2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_parameter: Option<String>,
}

/// File name of a prefab, derived from its ID.
pub fn prefab_file(id: &str) -> String {
    format!("prefabs/{}.json", sanitize(id))
}

/// Replaces characters that are awkward in file names.
pub fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names() {
        assert_eq!(
            prefab_file("sharedassets0.assets:12"),
            "prefabs/sharedassets0.assets_12.json"
        );
        assert_eq!(sanitize("a b/c:d"), "a_b_c_d");
    }

    #[test]
    fn hashes_and_curves() {
        assert_eq!(path_hash(""), 0);
        assert_eq!(path_hash("Base Layer"), 756_556_552);
        let c = AnimCurve {
            segments: vec![[0.0, -240.0, 180.0, 0.0, 0.0], [0.5, 0.0, 0.0, 0.0, 15.0]],
            ..Default::default()
        };
        assert_eq!(c.eval(-1.0), 0.0);
        assert_eq!(AnimCurve::default().eval(1.0), 0.0);
        assert!((c.eval(0.5) - 15.0).abs() < 1e-4);
        assert!((c.eval(0.25) - (-240.0 * 0.015625 + 180.0 * 0.0625)).abs() < 1e-4);
    }

    #[test]
    fn shape_json_is_tagged() {
        let c = Collider {
            id: "level2:5".into(),
            enabled: true,
            is_trigger: false,
            offset: [0.0, 0.0],
            density: 1.0,
            material: None,
            shape: Shape::Circle { radius: 0.5 },
        };
        let j = serde_json::to_value(&c).unwrap();
        assert_eq!(j["shape"], "circle");
        assert_eq!(j["radius"], 0.5);
        assert_eq!(serde_json::from_value::<Collider>(j).unwrap(), c);
    }
}
