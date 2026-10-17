# Simulation design (`rounds-sim`)

The sim is the whole game logic: players, bullets, cards, map props, match
flow. Everything else (rendering, audio, menus outside a match, networking)
reads it or feeds it inputs. Behaviour comes from `docs/spec/*.md`.

## Shape

```rust
pub struct Content { /* Arc-shared, immutable: maps, cards, prefabs, tuning */ }
pub struct World { /* Clone; everything that changes */ }
pub struct PlayerInput { /* bytemuck::Pod, fixed size */ }

impl World {
    pub fn new(content: &Content, setup: &MatchSetup) -> World;
    pub fn step(&mut self, content: &Content, inputs: &[PlayerInput]);
    pub fn events(&self) -> &[Event];      // produced by the last step
    pub fn checksum(&self) -> u64;         // for desync detection
}
```

- `MatchSetup`: slots (count ≤ 12, team per slot, skin/face per slot), mode
  (FFA / teams), rounds-to-win settings, seed, map rotation. Identical on every
  peer (the lobby sends it).
- `Content` holds data imported from the player's copy (see
  `docs/formats/import.md`), converted once into sim-friendly tables (map
  geometry, card definitions with resolved effect parameters). It never changes
  during a match and is not cloned for rollback.
- `World` holds only what changes: frame number, PRNG state, match phase,
  players, projectiles, spawned effect objects, physics props, timers. It must
  be cheap to clone (target: < 100 µs for 12 players and a busy arena) because
  rollback clones it every frame.

## Rules for determinism

- Fixed step `DT = 0.014` s. Any original code that scales by `Time.deltaTime`
  in `Update` is moved to the fixed step (document it in the spec).
- f32 math; transcendental functions only through `libm` (sin, cos, atan2,
  exp, pow, sqrt is IEEE-exact anyway). No `f32::sin` etc. (platform libm
  differs). No fast-math, no SIMD reductions.
- PRNG: one seeded generator in `World` (e.g. PCG32), plus derived streams
  for things that must not perturb each other (card draws vs. bullet spread).
  Never `rand::thread_rng`.
- Containers: `Vec` with stable order; entity ids are generational indices
  allocated deterministically. No `HashMap`/`HashSet` iteration in logic
  (`BTreeMap` or sorted vecs if needed).
- No wall clock, no threads, no I/O inside `step`.
- `checksum()` hashes the gameplay-relevant state (positions, health, ammo,
  phase, PRNG) with a fixed hash (FxHash/xxh3 over the bytes in a defined order).

## Physics

- Map props (Rigidbody2D boxes, ropes, joints, hinges, springs, moving
  platforms) run in `rapier2d` with `enhanced-determinism`, stored inside
  `World` (its sets are `Clone`). Static geometry is one fixed body set.
- Players are not rigid bodies (custom velocity and collision, see
  `docs/spec/player.md`); they interact with props by applying impulses and by
  shape queries against the rapier query pipeline.
- Bullets are ray/shape casts per step, not physics bodies.

## Entities

- `players: [PlayerState; 12]`-like fixed slots (inactive slots skipped).
- `projectiles: Arena<Projectile>`, `objects: Arena<SpawnedObject>` (explosions,
  fields, saws, mines, black holes …), `props` (physics handles + map script
  state).
- Card effects are data, not trait objects (M5, `src/script/`). Every
  prefab is compiled once (in `Content::new`) from its components' imported
  Unity fields into a `Prefab`: a node tree plus typed components (`Kind`:
  Explosion, DelayEvent, SpawnObjects, Saw, Teleport, ShieldCharge,
  PlayerInRangeTrigger, AbyssalCountdown, ...) whose UnityEvents become
  `Call` lists (target component/node + method). A spawned object carries a
  small `ObjState` (node active bits, component enabled bits, one
  `CompState` per stateful component). Card effect objects
  (`AddObjectToPlayer`, "A_..." prefabs) are objects of kind
  `ObjKind::PlayerEffect`; picking the same card again levels the existing
  object up when it has an `AttackLevel`. Hook points (`effects::Hook`:
  block phases, dealt damage, shoot, attack, out of ammo, reload, revive,
  death, player collision) dispatch to their components; per-tick behaviour
  runs in the object pass. Projectile add-ons (homing, drill, poison, ...)
  are compiled per prefab into `AddonDef`s and live on the projectile.

### Content contract for prefabs (adapter)

`ObjectDef` = one prefab: `id` (root GameObject id), `nodes` (every
GameObject, parents first: `id`, `name`, `parent`, `active`, local `pos`,
local `scale`), `effects` (every component of every node, in order:
`script` = script class or engine class name, `params` = raw fields with
`{"$ref": id}` references and UnityEvents as `m_PersistentCalls`, `id` =
component id, `node` = node index, `enabled` = `m_Enabled` if known).
`ProjectileDef.id` = the bullet prefab's root id (effect guns reference it).
Nodes also carry `transform` (the Transform component id), so Transform
references (`Saw.sparkTransform`, `LineEffect.fromPos`) resolve to nodes.
`enabled` comes from the import (revision 1). `ContentDefs::sound_events`
maps SoundEvent ids to names: sound references in `params` become cue
names (`Content::sound_event_name`). `ProjectileDef::wall_mask` /
`ignore_walls_mask` = the prefab's `RayCastTrail` masks.
References resolve by these ids; unknown ones are dropped. The shorthand
fields `explosion` / `remove_after` are for hand-built content.
`crates/sim/tests/fixture.rs` builds this from the import and serves as the
reference conversion.

## Events (sim → presentation)

`Event` values describe what the player sees and hears: shot fired (gun,
projectile ids), hit (point, normal, damage), block (success/fail), damage
number, death, explosion, card picked, phase changes, sound cue names. The
presentation spawns particles and plays sounds from these. Sound cue names
are SoundEvent asset names, resolved by the presentation with the import's
`sounds.json` (`GameData::sound_cue_files`, `SoundBank::event_by_name`). Because rollback
re-runs frames, the presentation keys events by `(frame, sequence)` and only
fires an event once; events of frames that were rolled back and not
re-confirmed can be cancelled for long sounds.

## Match flow inside the sim

Phases (all timers in frames): `Countdown → Playing → RoundOver (slow-mo) →
[CardPick] → MapTransition → Countdown …` until `MatchOver`. Card picks are
driven by the picking player's inputs (left/right/confirm), so they are
deterministic and work for local and online players alike. `PickMode`
(`World::set_pick_mode`; default `Auto` = simultaneous above 3 pickers,
match-flow.md 16): sequential picks expose `World::pick` /
`pick_selected`; simultaneous picks give every picker its own hand in
`World::pick_hands` (`PickHand`: state, own cursor, picked slot, done), with
`pick()` still returning the first unfinished hand. Teams: round
ends when ≤ 1 team has living players.

## Presentation interpolation

The renderer draws `lerp(previous, current, alpha)` for positions it can
match by entity id; snapping is acceptable for spawns/despawns.
