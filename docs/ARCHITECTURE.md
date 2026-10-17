# Architecture

## Goals

1. Play ROUNDS natively on Linux and Windows (macOS later) from the player's
   own Steam copy. The original's look, feel, cards and maps.
2. Up to 12 players: any mix of local players on one screen (controllers and
   keyboard) and online players. Free-for-all and teams.
3. Online play over [Warpzone](https://github.com/jarvis-jhk/warpzone) only
   (rooms, codes, NAT traversal, relay). No Photon, no Steam networking.
4. Public open source: the repo holds no game data and no decompiled code.

## Data flow

```
Steam install (read-only)
  └─ unity-assets ── parses SerializedFiles (2022.3, format 22, no type trees)
       └─ rounds-import ── writes <data dir>/import-v<N>/:
            textures/*.png, sprites.json, audio/*.ogg, maps/*.json,
            cards.json, prefabs/*.json, fonts/…
                 └─ loaded at start by rounds-port (sim gets plain Rust data)
```

- The import runs at first start (or `rounds-port --import <path>`), finds the
  install through Steam's library folders, and is versioned, so a format change
  re-imports.
- Engine classes are read with hand-written readers for exactly the Unity
  2022.3.34f1 layouts we need. Game scripts (MonoBehaviour fields such as
  `CardInfo`, `Gun`, `CharacterStatModifiers`, map objects) are read with layouts
  described in Rust (field lists), derived from Unity's serialization rules.
- Textures: DXT5/DXT1, RGBA32, RGB24, Alpha8 → RGBA8. Audio: FSB5 (Vorbis) →
  Ogg Vorbis files.

## Simulation (`rounds-sim`)

- Fixed step of **0.014 s** (the original's `Fixed Timestep`, ≈71.4 Hz).
- Pure function of `(state, inputs for every player) → state`. `State: Clone`;
  cloning is how rollback saves frames. No I/O, no randomness except a seeded
  PRNG in the state, no wall-clock time, no `HashMap` iteration in logic,
  `libm` for sin/cos/atan2/exp/pow, so Linux and Windows x86_64 builds agree
  bit for bit.
- Physics for map props (boxes, ropes, hinges, springs): `rapier2d` with
  `enhanced-determinism`. Players are not rigid bodies in the original; their
  custom movement and collision are reimplemented as specified.
- Emits *events* (shot fired, hit, block, death, sound cues) that the
  presentation layer turns into particles and sounds. Events from rolled-back
  frames are deduplicated by (frame, kind, id).
- Game flow (rounds, points, card picks, map changes) lives in the sim too, so
  it is identical on every peer, and card picks are just inputs.

## Players and input

- A *player slot* (up to 12) is bound to a *device* on some machine: keyboard
  and mouse, keyboard only, or a controller (SDL2 GameController, any number).
- Each frame, each machine produces inputs for its local slots only. Locally
  that goes straight into the sim; online, through GGRS.
- Input per player per frame: move x/y (quantised), aim vector (quantised
  angle), buttons (jump, fire, block, …). Small and fixed-size.

## Networking (`rounds-net`)

- GGRS (GGPO-style rollback), P2P session: each peer is a *machine*, carrying
  1–N local player slots. GGRS's non-blocking socket is implemented on
  Warpzone's peer channels (`warpzone-core` used directly from Rust).
- Lobby, hosting, room browser, codes and invite links through Warpzone
  (tier 2), `--warpzone-join/--warpzone-host` (tier 1). A descriptor for
  `games/` is kept in `packaging/warpzone/rounds-port.toml`.
- Desync detection: a state checksum every N frames, compared between peers.
- Local development uses our own `warpzone-server` from `ext/warpzone`
  (`dev/server.toml`, http://127.0.0.1:2998).

## Presentation (`rounds-render`, `rounds-port`)

- SDL2 for window, input (controllers), audio. Linked against the system's
  `libSDL2-2.0.so.0` on Linux (dev: `tools/lib/libSDL2.so`) and `SDL2.dll` on
  Windows (shipped with the release).
- wgpu for rendering: sprites, coloured shapes, 2D soft shadows (the original's
  SFPolygon/SFLight), particles, text (TextMeshPro SDF atlases from the import),
  screen shake, post effects. Same renderer runs offscreen (`--headless`) for
  screenshots and tests.
- Camera: like the original for ≤ 4 players; for more players the arena view
  stays one shared screen with a camera framing all living players.
- Interpolation between sim frames is visual only.

## Builds

- Linux x86_64: `cargo build --release`.
- Windows x86_64: cross-compiled from Linux (`x86_64-pc-windows-gnu`) with a
  toolchain in `tools/`, tested under Proton/Wine (`tools/wineprefix`).
- macOS: later (wgpu → Metal, SDL2 framework).
