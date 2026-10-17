# Import format (version 1, revision 3)

`rounds-import` converts the player's ROUNDS install into this directory
layout. The game loads it with `rounds_import::GameData::load(dir)`; the serde
types live in `crates/import/src/format.rs` (the source of truth for field
names). In the user data dir the directory is called `import-v<N>`
(`rounds_import::versioned_dir_name()`); for development it is `cache/import`.

```
cargo run --release -p rounds-import -- [--game DIR] [--out DIR] [--force]
cargo run --release -p rounds-import --bin map-sheet -- [--data DIR] [--out FILE] [--map N] [--colliders]
```

The import is deterministic and idempotent. It is written to `<out>.tmp` and
then renamed over `<out>`. If `index.json` already has the current format
version, at least the current revision and the same source fingerprint (data
file names, sizes, mtimes), the import is skipped unless you pass `--force`.
A non-empty directory without `index.json` is never replaced.

**Versions and revisions**: `FORMAT_VERSION` (`index.json`
`format_version`, also the `import-v<N>` directory name) changes only on
incompatible changes. `FORMAT_REVISION` (`format_revision`, 0 when absent)
counts additive changes within a version: new optional fields and files.
`GameData::load` accepts any revision of its version (missing data loads as
empty/`None`); `rounds_import::import` re-imports anything older than the
current revision (and anything missing one of `load::LATER_FILES`), so a
player's old import is refreshed automatically at the next start.

| Revision | Added |
|---|---|
| 0 | first release; later `main.json`, `materials.json`, `post_profiles.json` |
| 1 | `Component.enabled`, `Object.transform`, `Card.categories` / `blacklisted_categories` / `name_key` / `description_key` / `stat_keys`, `sounds.json`, `categories.json`, `localization.json`, `animations.json`; prefab references followed 4 levels deep (the default bullet's pooled collision prefab) |
| 2 | `Object.particles` (typed particle systems in every map, prefab and `main.json`), `Player.skin_prefabs` (the `Skin_Player*` prefabs), `Font.info` (typed glyph tables; out-of-atlas whitespace rects repaired), Animator `parameters` / transitions (`AnimState.outgoing`, `AnimLayer.any_state_transitions`, `cycle_offset`, `speed_parameter`), `ProjectSettings.gravity_3d` |
| 3 | `faces.json` (character creator catalog: eyes, mouths, accessories in loader order, default face, preset slots) and the 197 `CharacterItem` GameObjects as `prefabs/` files |

## Conventions

- **Object IDs**: `"<file>:<path id>"`, e.g. `"sharedassets0.assets:2375"`,
  `"level5:94"`. `<file>` is the SerializedFile name in `ROUNDS_Data`.
- **Raw fields** (`fields`): a component's serialized fields under their Unity
  names, converted generically:
  - Object references are written as `{"$ref": "<id>"}`, and null references
    as `null`.
  - Floats use the shortest form that round-trips the `f32`. Non-finite
    floats become `null`.
  - `UInt8`/`bool` script flags stay numbers (`0`/`1`), as Unity stores them.
  - Keys are sorted alphabetically.
  - MonoBehaviour fields omit the header (`m_GameObject`, `m_Enabled`,
    `m_Script`, `m_Name`); the enabled flag is in `Component.enabled`.
- **Coordinates**: Unity world units, y up. Sprite pixel rects in
  `sprites.json` use bottom-left origin, except `texture_rect`, which is in
  PNG space (top-left origin).
- **Transforms**: `local` holds the serialized local TRS (quaternion
  `[x,y,z,w]`). `world` holds the composed world position, rotation, lossy
  scale, `rotation_z` (degrees, counter-clockwise) and `matrix`
  `[a,b,c,d,tx,ty]`, which maps local x/y to world exactly, including skew
  from non-uniform parent scale: `x' = a·x + c·y + tx`, `y' = b·x + d·y + ty`.

## Files

| File | Content |
|---|---|
| `index.json` | `Index`: format version, Unity version, source fingerprint, scenes (build order, `map_file` for maps), sorting layers in draw order (`name`, `id`), layer and tag names, project settings (`fixed_timestep`, `gravity` (2D), `gravity_3d` (3D `Physics.gravity`, used by particle systems; revision 2), raw `physics2d` and `time`), `player_prefabs`, counts. Pretty-printed. |
| `textures.json` | id → `{name, png, width, height, format, filter_mode, wrap_mode}` |
| `textures/<file>_<pid>.png` | RGBA8, top row first (decoded from DXT1/DXT5/RGBA32/RGB24/Alpha8/...; Alpha8 becomes white with alpha) |
| `sprites.json` | id → `{name, texture, rect, texture_rect, texture_rect_offset, pivot, pixels_per_unit, border, packed, packing_rotation, physics_shape}`. `physics_shape` is in world units, relative to the pivot. |
| `audio.json` | id → `{name, file, channels, frequency, length, samples}` |
| `audio/<name>.ogg` | Ogg Vorbis, remuxed losslessly from the FSB5 data (no re-encoding). Name clashes get a `_2`, `_3`, ... suffix. |
| `maps/<NN>_<Scene>.json` | `Map` for build indices 2–71 (see below) |
| `cards.json` | list of `Card`, sorted by id |
| `prefabs/<file>_<pid>.json` | `Prefab`: hierarchies of cards, the player and everything they reference |
| `player.json` | `{prefabs, skins, skin_prefabs}`: player prefab ids, `PlayerSkinBank`/`PlayerSkin` fields (with `$name`, `$script`), and (revision 2) `PlayerSkin` id -> its GameObject `Skin_PlayerOne`..`Four` under `prefabs/`, whose particle system (`particles`) fills the player's body in the skin colours |
| `main.json` | `Scene`: the main scene `level0` (cameras, `SFRenderer`, `SFLight`, post-process volumes, `ArtHandler`, the art particle systems) as an object list like a map's; here `ParticleSystem` and `ParticleSystemRenderer` components also keep their raw `fields` (every object with a particle system has the typed `particles` too). |
| `materials.json` | id → `Material {name, shader, render_queue, keywords, textures: {prop: {texture, scale, offset}}, floats, colors}` for every Material in the data. |
| `post_profiles.json` | id → `PostProfile {name, effects}`: post-processing stack v2 profiles; `effects` maps the effect class (`ColorGrading`, `Bloom`, `Vignette`, ...) to its raw fields (each parameter is `{overrideState, value}`). |
| `fonts.json` | TextMeshPro font assets: `{id, name, atlases, fields, info}`. The atlases are SDF textures in `textures/`, `fields` holds the raw face info and glyph, character and kerning tables, `info` (revision 2) the typed tables, see "Fonts" below. |
| `sounds.json` | `SoundBank` (revision 1): Sonigon sound data, see below. |
| `categories.json` | `CardCategory` assets: id -> name (revision 1). Pretty-printed. |
| `localization.json` | `Localization` (revision 1): the Unity Localization string tables of every locale, see below. |
| `faces.json` | `Faces` (revision 3): the character creator catalog, see "Faces" below. |
| `animations.json` | `Animations` (revision 1): every `AnimationClip` and `AnimatorController`, see below. |

### Objects (maps and prefabs)

A map or prefab is a flat list of `Object`s in which parents come before their
children (`parent` and `children` are indices into the list). Each object
has:

- `id`, `name`, `active` (self), `active_in_hierarchy`, `layer`, `tag`, and the
  `local` and `world` transforms.
- `transform` (revision 1): the id of the object's `Transform` /
  `RectTransform` component. Component fields that reference a Transform
  (`Saw.sparkTransform`, `LineEffect.fromPos`, `LegRaycasters.legCastPositions`,
  IK targets, ...) hold this id.
- `rect` (optional, added later in version 1): `RectTransform` layout
  (`anchor_min`, `anchor_max`, `anchored_position`, `size_delta`, `pivot`).
- `sprite` (from `SpriteRenderer`): `enabled`, `sprite` id, `color` (tint),
  `flip_x`/`flip_y`, `draw_mode` (0 simple, 1 sliced, 2 tiled) with `size`,
  `sorting_layer` (name), `sorting_layer_index` (draw order), `sorting_order`,
  `mask_interaction` and material names. Draw order follows Unity: sorting
  layer index, then order in layer, then larger z first.
- `colliders`: `id`, `enabled`, `is_trigger`, `offset`, `density`, physics
  `material` (`friction`, `bounciness`), and a shape tagged by `shape`:
  - `box`: `size`, `edge_radius`
  - `circle`: `radius`
  - `capsule`: `size`, `direction`
  - `polygon`: `paths`
  - `edge`: `points`, `edge_radius`

  All values are in the object's local units.
- `rigidbody`: `body_type` (0 dynamic, 1 kinematic, 2 static), `simulated`,
  `use_auto_mass`, `mass`, drags, `gravity_scale`, `constraints`,
  `interpolate`, `collision_detection`, `sleeping_mode`.
- `joints`: `kind` (`HingeJoint2D`, `SpringJoint2D`, `FixedJoint2D`, ...),
  `connected_body` (Rigidbody2D id, `null` = world), `connected_object` (index
  of that body's object in the same list), `anchor`, `connected_anchor`,
  `enable_collision`, `break_force` (`null` = unbreakable), and all raw
  `fields` (limits, motor, distance, frequency, damping, ...).
- `particles` (revision 2): the object's `ParticleSystem` with its
  `ParticleSystemRenderer`, typed (see "Particle systems" below).
- `components`: every component except the Transform, in order, as `{id,
  type, script, enabled, fields}`. `enabled` (revision 1) is `m_Enabled`
  for components that have it (Behaviours incl. every MonoBehaviour,
  Renderers, Colliders, Animator, ...) and absent otherwise. `script` is the namespace-qualified script class
  for MonoBehaviours. Raw `fields` are omitted for `ParticleSystem` and
  `ParticleSystemRenderer` outside `main.json` (bulky; use `particles`), and
  for classes already extracted into typed fields above (SpriteRenderer,
  the colliders, Rigidbody2D).

In maps, world transforms are scene world space. In prefabs, they are
relative to the root's parent, so the root's world equals its local
transform.

### Map

`{build_index, name, map, spawn_points, objects}`:

- `map` holds the fields of the scene's `Map` script (`size` is the camera's
  orthographic half-height).
- `spawn_points` lists `{object, id, team, position}` from the `SpawnPoint`
  scripts, with `position` in world space.
- `objects` holds every GameObject of the scene, with the roots in file order.

### Card

- `id` and `object_name`: the card prefab's root GameObject.
- `name`, `description`, `rarity`, `color_theme`: from `CardInfo`.
- `art`: the `cardArt` prefab id.
- `prefab`: the path of the card's prefab file.
- `components`: the raw fields of the root's components keyed by script or
  class name (`CardInfo`, `ApplyCardStats`, `Gun`, `CharacterStatModifiers`,
  `Block`, ...).
- `referenced_prefabs`: the GameObjects outside the card that its
  components reference (spawned objects, effects, art), each exported under
  `prefabs/`.
- `categories`, `blacklisted_categories` (revision 1): `CardInfo.categories`
  / `blacklistedCategories` as category names (`categories.json`).
- `name_key`, `description_key`, `stat_keys` (revision 1): the localisation
  keys (in `StringTableCards`) of `CardInfo.m_localizedCardName`,
  `m_localizedCardDescription` and each `cardStats[].m_localizedStat`;
  absent where the card has no localised reference (12 cards have no name
  key, 29 no description key: use `name` / `description`). Card art:
  `CardInfo.cardArt` (`art`) and the frame `CardInfo.cardBase` are prefabs
  under `prefabs/`; their `UnityEngine.UI.Image` components reference
  sprites in `sprites.json` (`m_Sprite`), their `Animator`s reference
  controllers (`m_Controller`) in `animations.json`.

Prefab references are followed transitively up to 4 levels deep from the
cards and the player prefab.

### Faces (`faces.json`, revision 3)

`Faces` (`format.rs`) comes from the main scene's `CharacterCreatorItemLoader`
(`eyes` 85, `mouths` 58, `accessories` 54 references to `CharacterItem`
components, in list order; a face's item id is the index in its list):

- `eyes` / `mouths` / `accessories`: `[FaceItem {id, prefab, name,
  item_type, sprite, offset, scale, color, move_health_bar_up, fields}]`.
  `id` is the `CharacterItem` component id (what the loader lists hold),
  `prefab` its GameObject (`P_E_Neutral`, ...), `item_type` 0 eye / 1 mouth
  / 2 accessory, `sprite` a `sprites.json` id, `offset` / `scale` the
  `CharacterItem` fields (face units), `color` the item's `SpriteRenderer`
  tint (RGBA), `move_health_bar_up` the health bar raise for tall
  accessories; `fields` the raw `CharacterItem` fields.
- `default_eyes`, `default_mouth` (`CharacterItem` ids) and `scale_m`
  (1.05): the player prefab's `CharacterCreatorItemEquipper` defaults.
- `presets`: `CharacterCreatorHandler.playerFaces` as `[{eye_id,
  eye_offset, mouth_id, mouth_offset, detail_id, detail_offset, detail2_id,
  detail2_offset}]`, plus `selected_presets`, `selected_face_id`,
  `locked_portraits`. The shipped data has ten preset slots, all zero
  (item 0, no offset): the original has no authored preset faces beyond the
  neutral default, and a port may define its own presets.

Every item's GameObject is also written as a prefab file (`prefabs/`,
`FaceItem.prefab`) with the `CharacterItem` component (`script:
"CharacterItem"`, `fields` as above) and the `SpriteRenderer` (`sprite`);
`crates/game/src/faces.rs` reads those files and resolves the loader lists
in `main.json`, so the two sources agree. `GameData::faces` loads
`faces.json` (empty before revision 3).

### Sounds (`sounds.json`)

`SoundBank` (`format.rs`; behaviour in `docs/spec/audio.md`):

- `events`: SoundEvent id -> `{name, layers: [{container, delay,
  volume_ratio}], cooldown, probability, disabled, poly_group,
  poly_group_priority, mix, modifier, trigger_on_play / trigger_on_stop /
  trigger_on_tail: {events, which} | null, tail_length, fields}`.
  `modifier` is the raw `soundEventModifier` (values with `...Use` flags,
  e.g. polyphony), `fields` all raw `variables`.
- `containers`: SoundContainer id -> `{name, clips (AudioClip ids), files
  (their Ogg files), looping, volume_db, volume_ratio, volume_random_db
  (null = off), pitch_semitone, pitch_ratio, pitch_random_semitone (null =
  off), spatial_blend, distance_enabled, distance_scale, priority,
  random_start_position, settings}` (`settings` = every raw field: intensity
  curves, crossfades, effects).
- `shot_modifiers`, `impact_modifiers`: `{name, priority, events: {slot ->
  SoundEvent id}}` (slots `single`, `singleAutoLoop`, ..., `impactCharacter`,
  `impactEnvironment`).
- `poly_groups`: `{name, limit (null = off), skip_lower_priority,
  allow_fade_out}`; `mixes`: `{name, parent, modifier}`.
- `references`: every component field in maps, prefabs, cards and the main
  scene that references a SoundEvent or sound modifier, as
  `"<script>.<field path>"` -> ids (array elements as `[]`), e.g.
  `PlayerSounds.soundCharacterJump`, `CurveAnimation.soundPlay[].soundEvent`,
  `SoundImplementation.SoundUnityEventPlayer.soundStart`.

The sim's sound cues are SoundEvent names. To play one:
`GameData::sound_cue_files(cue)` (absolute Ogg paths per layer), or
`sounds.event_by_name(cue)` -> `SoundEvent` -> `layers[].container` ->
`containers[id].files` and its settings.

### Animations (`animations.json`)

`Animations {clips, controllers}`, keyed by asset id:

- `AnimClip {name, start, stop, sample_rate, looping, curves, pptr, events}`.
  Each `AnimCurve {path, class_id, script, property, pptr, segments}`
  animates one value of the object at transform path `path` below the
  Animator's object (`path_hash("Child/Grandchild")`, CRC32; 0 = the
  Animator's own object): `property` is `position.x`, `rotation.w`
  (quaternion), `euler.z` (degrees), `scale.y` for Transforms (class 4),
  otherwise the serialized property path of component class `class_id` /
  script `script` (`m_Color.a` of an `Image`, `m_AnchoredPosition.x` of a
  RectTransform, `m_IsActive` of the GameObject, `m_Alpha` of a
  CanvasGroup, ...). `segments` are cubic pieces `[t0, a, b, c, d]`
  (`AnimCurve::eval`); object-reference curves (`pptr`, sprite swaps) give
  an index into the clip's `pptr` list. Paths that match no object are
  ignored, as in Unity (about 4% of the card art curves).
- `AnimController {name, clips, parameters, layers: [{name,
  default_weight, default_state, any_state_transitions, states: [{name,
  speed, speed_parameter, cycle_offset, looping, clip, transitions (count),
  outgoing}]}]}` (parameters and transitions: revision 2).
  `parameters`: `{name, kind (float|int|bool|trigger), default}`.
  Transitions (`outgoing` in priority order, `any_state_transitions`):
  `{destination (state index, null for exits/selectors), destination_raw,
  duration (normalised to the source state's length, seconds if
  fixed_duration), offset (normalised start in the destination),
  exit_time, has_exit_time, fixed_duration, interruption_source,
  ordered_interruption, can_transition_to_self, conditions: [{mode
  (if|if_not|greater|less|equals|not_equal|exit_time), parameter,
  threshold}]}`. The shipped controllers have one layer, one looping state
  (one has two, its default state has no clip) and no transitions or
  parameters. `rounds_import::animator::AnimatorPlayer` plays a layer:
  `new(ctrl, layer)`, `set`/`trigger`, `update(ctrl, anims, dt)`,
  `sample(ctrl, anims)` -> `[{clip, time, weight}]` (looping, exit-time and
  condition transitions, trigger consumption, cross-fades).

### Particle systems (`Object.particles`, revision 2)

`rounds_import::particles::ParticleSystem`, converted from the raw
serialized modules (`particles.rs`). Units are Unity's: seconds, world
units, **radians** for start rotation and rotation over lifetime, degrees
for the shape's angle/arc/rotation; colours as authored (sRGB). Disabled
modules are absent (`null`), except `emission`, which carries `enabled`.

- `component` (the ParticleSystem id; sub-emitters refer to it).
- `main`: `duration`, `looping`, `prewarm`, `play_on_awake`, `start_delay`,
  `start_lifetime`, `start_speed`, `start_size` (+ `size_3d`,
  `start_size_y/z`), `start_rotation` (+ `rotation_3d`,
  `start_rotation_x/y`), `flip_rotation`, `start_color`,
  `gravity_modifier`, `gravity_source` (0 = 3D gravity
  `index.settings.gravity_3d` (-9.81), 1 = 2D gravity), `simulation_space`
  (0 local, 1 world, 2 custom + `custom_simulation_space` Transform id),
  `simulation_speed`, `use_unscaled_time`, `scaling_mode` (0 hierarchy,
  1 local, 2 shape), `max_particles`, `emitter_velocity_mode`,
  `stop_action`, `auto_random_seed`, `random_seed`, `culling_mode`.
  "Over the system" curves (start values, rate) are evaluated at the
  system's normalised time (`time / duration`).
- `emission`: `enabled`, `rate_over_time`, `rate_over_distance`, `bursts:
  [{time, count, cycles (0 = infinite), interval, probability}]`.
- `shape`: `kind` + `kind_name` (`sphere`, `hemisphere`, `cone`, `box`,
  `circle`, `edge`, `donut`, `rectangle`, `sprite`, ...), `angle`, `radius`
  / `arc` as `{value, mode (0 random, 1 loop, 2 ping-pong, 3 burst
  spread), spread, speed}`, `radius_thickness`, `length`,
  `box_thickness`, `donut_radius`, `position`, `rotation`, `scale` (the box
  size), `align_to_direction`, `random_direction`, `spherical_direction`,
  `random_position`, `sprite`.
- `velocity_over_lifetime` (`x/y/z`, `orbital_x/y/z`, `orbital_offset`,
  `radial`, `speed_modifier`, `world_space`), `limit_velocity_over_lifetime`
  (`separate_axes`, `limit`, `x/y/z`, `dampen`, `drag`, `drag_by_size`,
  `drag_by_velocity`, `world_space`), `force_over_lifetime` (`x/y/z`,
  `world_space`, `randomize_per_frame`), `color_over_lifetime` (a
  MinMaxGradient), `size_over_lifetime` (`separate_axes`, `x/y/z`),
  `rotation_over_lifetime` (`separate_axes`, `x/y`, `z`), `noise`
  (`strength(_y/_z)`, `separate_axes`, `frequency`, `scroll_speed`,
  `damping`, `octaves`, `octave_multiplier`, `octave_scale`, `quality`,
  `position/rotation/size_amount`, `remap`), `texture_sheet` (`mode` 0 grid
  / 1 sprites, `tiles`, `animation_type`, `row_mode`, `row_index`,
  `frame_over_time`, `start_frame`, `cycles`, `time_mode`, `fps`,
  `speed_range`, `flip_u/v`, `sprites`), `trails` (`mode`, `ratio`,
  `lifetime`, `min_vertex_distance`, `world_space`, `die_with_particles`,
  `texture_mode`, `size_affects_width/lifetime`, `inherit_particle_color`,
  `color_over_lifetime`, `color_over_trail`, `width_over_trail`,
  `ribbon_count`), `collision` (`kind`, `mode` 0 3D / 1 2D, `bounce`,
  `dampen`, `lifetime_loss`, `min_kill_speed`, `radius_scale`,
  `collides_with`, `send_messages`).
- `sub_emitters`: `[{emitter (component id), object (index in the same
  object list, if there), kind (0 birth, 1 collision, 2 death, 3 trigger,
  4 manual), properties (inherit bits), probability}]`.
- `renderer`: `enabled`, `render_mode` (0 billboard, 1 stretched, 2
  horizontal, 3 vertical, 4 mesh, 5 none), `material` (`materials.json`
  key), `material_name`, `shader`, `texture` (the material's `_MainTex`),
  `trail_material` / `trail_texture`, `sorting_layer`,
  `sorting_layer_index`, `sorting_order`, `sort_mode`, `sorting_fudge`,
  `camera_velocity_scale`, `velocity_scale`, `length_scale`,
  `rotate_with_stretch`, `min/max_particle_size`, `render_alignment`,
  `pivot`, `flip`, `mask_interaction`.

Curves and gradients:

- `MinMaxCurve`, tagged by `mode`: `{mode: "constant", value}`, `{mode:
  "curve", multiplier, curve}`, `{mode: "two_curves", multiplier, min,
  max}`, `{mode: "two_constants", min, max}`; `eval(t, random)`.
- `Curve {keys: [{time, value, in_slope, out_slope, weighted_mode?,
  weights?}], pre_wrap, post_wrap}` (wrap 0 ping-pong, 1 loop, 2 clamp):
  Unity's `AnimationCurve` keys with tangents; infinite (stepped) tangents
  are written as `null`. `Curve::eval` is Hermite (weights ignored).
- `MinMaxGradient`, tagged by `mode`: `color {color}`, `gradient
  {gradient}`, `two_colors {min, max}`, `two_gradients {min, max}`,
  `random_color {gradient}`; `eval(t, random)`.
- `Gradient {mode (0 blend, 1 fixed, 2 perceptual), colors: [{time,
  color: [r, g, b]}], alphas: [{time, alpha}]}`; `eval(t)`.

### Fonts (`fonts.json` `info`, revision 2)

`FontInfo {atlas_width, atlas_height, padding, render_mode, point_size,
scale, line_height, ascent, descent, baseline, cap_line, mean_line,
underline_offset, underline_thickness, tab_width, bold_style,
bold_spacing, glyphs, characters, kerning}`: metrics in atlas pixels at
`point_size`. `glyphs: [{index, atlas, rect, rect_png, width, height,
bearing_x, bearing_y, advance, scale}]` with `rect` in TMP's bottom-left
origin and `rect_png` in PNG space (both without the SDF `padding`, which
the quad and UVs extend by on every side); `characters`: code point ->
glyph index; `kerning: [{first, second, first_adjust, second_adjust}]`
(value records `[x placement, y placement, x advance, y advance]`).
Helpers: `glyph_for(char)`, `kerning_x(first, second)`.

Clarified (from the data): `LiberationSans SDF` was made with TMP 1.x
(its legacy `m_glyphInfoList` is still serialized) and upgraded by TMP
1.4+, which converted the legacy top-left `y` to the bottom-left
`m_GlyphRect.m_Y = atlasHeight - (y + height)`. Its 15 whitespace glyphs
(U+0020, U+00A0, U+2000-U+200B, U+202F) had placeholder positions below
the 1024 atlas (`y = 1033`), giving `m_Y = -105` (space, no-break space)
or `-9` (zero-size). TMP never draws whitespace, so only their advances
matter. The import replaces every rect outside the atlas, in `fields` and
`info`, by `[0, 0, 0, 0]` with zero width/height (bearings and advances
kept) and warns if a non-whitespace glyph is affected (none is). The five
Gravity fonts were authored with the current table and are unchanged.
`cargo run --release -p rounds-import --example font_sheet` rasterises a
test string per font to `scratch/fonts-*.png`.

### Localisation (`localization.json`)

Read from the Addressables bundles in
`<Game>_Data/StreamingAssets/aa/<platform>/` (`localization-string-tables-*`
and `localization-assets-shared`; `UnityFS` with LZ4 blocks, SerializedFiles
with embedded type trees: `unity_assets::bundle`). `Localization`:
`{locales: ["de", "en-US", ...], tables: {name -> {guid, keys: {key id ->
key name}, strings: {locale -> {key name -> text}}}}}`. Tables:
`StringTableDefault` (UI, 95 keys), `StringTableCards` (147 keys);
`AssetTableDefault` has keys only (fonts per locale). Texts keep smart-string
placeholders (`{ROOMCODE}`) and TMP tags. Lookups: `Localization::text(locale,
table, key)` (table by name or `"GUID:<hex>"`, falls back to en-US),
`Localization::resolve(locale, raw LocalizedString field)` for the
`m_localizedString` / `m_localizedCardName` fields found in components, and
`GameData::card_text(card, locale)`. Without bundles the file has no tables
(a warning is reported).

`main.json`, `materials.json` and `post_profiles.json` were added to version
1 later (backwards compatible: `GameData` loads them as optional, and the
importer treats an import without them as outdated and re-imports it).

## Known gaps (version 1)

- Sliced/tiled sprites carry `draw_mode` and `size`, but the 9-slice
  `border` must be applied by the renderer.
- `SerializeReference` payloads are read only for the localization types
  listed in `scripts/dev/gen_typetrees.py` (bundles carry their own type
  trees, so their payloads are read fully).
- One MonoBehaviour is skipped:
  `Sirenix.Serialization.GlobalSerializationConfig`, an editor config whose
  DLL is not shipped.
- Of the Addressables bundles only the string tables are imported (not the
  per-locale font assets).
- Animator blend-tree weights are not exported (only each state's first
  clip); sub-state-machine selector destinations are left unresolved
  (`destination: null`).
- Particle systems: mesh shapes/renderers, custom data, lights, external
  forces, triggers, inherit velocity, colour/size/rotation by speed and
  lifetime-by-emitter-speed are not typed (`main.json` keeps the raw
  fields).
