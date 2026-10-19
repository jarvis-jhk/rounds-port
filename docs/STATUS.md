# Status

Living plan. Update when a step finishes.

## Milestones

- [x] M0 Repo, toolchains in `tools/`, decompilation and JSON dump in `ref/`,
      backup wired (`.rsync-filter`, `.rsync-prescript`), quota-loop armed.
- [x] M1 Asset import: textures, sprites, audio, maps, cards readable by
      `unity-assets`; `rounds-import` writes the data dir; contact sheet of
      every map rendered from imported data.
- [x] M2 Renderer + platform: SDL2/wgpu app shows a map; headless
      screenshots; controllers enumerated; audio plays.
- [x] M3 One player in a sandbox: movement, jump, wall interaction, aim,
      shoot, reload, block, health, death; map props are physical.
- [x] M4 Local 2–4 players, full match flow: rounds, points, card choice,
      map transitions, win screen, menus.
- [x] M5 All cards implemented (data from import, effects in Rust).
      Sim side done: prefab component runtime (`crates/sim/src/script`), all
      79 cards tested against the real import (`tests/cards.rs`, skipped
      without data), 12-player stress test (`tests/stress.rs`). Open: the
      adapter (`crates/content`) must emit prefabs per the contract in
      `docs/sim-design.md`; presentation of the new effect events.
- [x] M6 12 local players on one screen: join screen, colours, spawns,
      camera, HUD.
- [x] M7 Teams (any split of 12) and free-for-all.
- [x] M8 Online over Warpzone with rollback; mixed local+online; tested
      against our own local warpzone-server with several instances.
- [x] M9 Audio, options, controller remapping, polish, original feel pass.
      Done: windowed path verified on a private Xvfb (keyboard, mouse, 4+
      simulated pads with hot-plug; bugs fixed, docs/platform.md "Testing on
      a private X server"), imported particle effects everywhere (psys),
      card art animations, LiberationSans card texts, ropes/debris/feet from
      the sim views, main-camera post profile (grading + bloom) actually
      applied, intro, online VS screen, menu music, glyph prompts, control
      remapping, invite-link copy, system CJK fonts, simultaneous pick HUD.
      Open: an on-screen comparison with the original (needs an OK to run
      ROUNDS.exe), chromatic aberration, particle trails.
- [x] M10 Windows build cross-compiled, runs under Proton/Wine.
      Done: GUI subsystem + AttachConsole, procedural icon + VERSIONINFO
      (windres), Proton runs on the private Xvfb, identical bot-match
      checksums Linux vs Windows, online test with a Windows machine mixed
      in (`ONLINE_TEST_WINDOWS`, case `win`). Open: a real Windows machine.
- [x] M11 Packaging: release archives, Warpzone descriptor, README, docs.
      Published as github.com/jarvis-jhk/rounds-port (v0.1.0 prerelease); the
      specs live in a private repository (`docs/spec/`, see CLAUDE.md).

## Workstreams (current)

| Stream | State |
|---|---|
| Asset reader + import (`crates/unity`, `crates/import`) | done (M1) |
| Platform + renderer (`crates/render`, `crates/game`) | done; Windows build under Proton |
| Real maps in renderer (M2) | done. Missing: bloom, chromatic aberration |
| Rollback netcode (`crates/net`) | done standalone (netsim 12 machines, drops, relay). Next: hook up the sim (M8) |
| Specs (`docs/spec/*.md`) | done (v1), clarified by implementers |
| Simulation core + card effects (`crates/sim`) | done (M5 sim side): 110 tests, 12×16-card stress determinism |
| Import → sim adapter (`crates/content`) | done: all maps/cards; smoke test every map 4p/12p |
| Data gaps (m_Enabled, transform ids, categories, sound events, localisation) + Tunnels push-out bug | in progress |
| Presentation data (import revision 2): typed particle systems everywhere + `Skin_Player*` prefabs, font glyph tables (LiberationSans whitespace rects repaired), Animator transitions/parameters + `animator::AnimatorPlayer`; sim views `World::ropes`, `debris()[i].prop/piece`, `foot_plants`, `raycast` | done |
| Import revision 3: `faces.json` (character creator catalog, 197 `CharacterItem` prefabs) + sim forfeit (`World::set_disconnected`, `BTN_FORFEIT`, `PlayerDisconnected`; match-flow.md 16) | done; net/game must call `set_disconnected` for `PlayersDropped` slots |
| Playable match presentation (`crates/game` play module) M3/M4 | done (headless + windowed on Xvfb) |
| Menus, local lobby (12 devices, teams), online driver + UI over rounds-net (M6–M8) | done (headless + windowed on Xvfb; online test matrix passes, forfeit on drop wired) |
| Sim review fixes (`docs/spec/review/sim-vs-spec.md`, private: M1-M4, L1-L12 fixed with regression tests, specs corrected; L13 declined) + simultaneous card picks (`PickMode`, `World::set_pick_mode`, `World::pick_hands`; default Auto = simultaneous above 3 pickers) | done (sim side); presentation must draw `pick_hands()` when `simultaneous_pick()` |
| Packaging + CI (M11 groundwork) | done: dist tarball/zip, SHA256SUMS, workflows; fmt + clippy clean workspace-wide |
| Built-in mods (`docs/MODS.md`, ROADMAP "Popular mods, built in") | slices 1-2 done: survey of the top 20; `rounds_sim::Mods` in `MatchSetup` (serde default = all off, sent to peers inside the lobby's `MatchRules`), lobby MODS page, `--pick-n`; first mod Pick N Cards. `tests/mods.rs` pins vanilla checksums recorded before the change. Next: slice 3 (rule mods) |

## Open questions (for the owner)

- **Reference screenshots of the original**: running ROUNDS.exe under Proton on
  the private Xvfb (`scripts/dev/xvfb.sh`) would connect to the running Steam
  client (playtime, possibly cloud sync) — outside this folder, so not done
  without an OK. Needed to verify bloom strength, menu brightness, feel.
- **Real hardware**: windowed play on the real GPU, physical controllers, a
  real Windows machine — all verified only on Xvfb/lavapipe/Proton so far.

## Next steps

- M11: published 2026-10-17 as github.com/jarvis-jhk/rounds-port (v0.1.0
  prerelease, built on the dev laptop) with the Warpzone descriptor
  (`games/rounds-port.toml` in jarvis-jhk/warpzone). CI green and v0.1.0
  rebuilt by `release.yml` (b9b2791). Priorities: `ROADMAP.md`.
- Crash reports: `crates/game/src/crash.rs` (panic hook → public ntfy topic)
  and `.github/workflows/crash-reports.yml` (topic → issues labelled `crash`).
- Feel pass against the original (needs reference footage or the OK above).
- macOS port (wgpu Metal + SDL2 framework).
- Gaps: chromatic aberration, particle trails/collision, right-keyboard aim
  keys fixed, glyph prompts outside the lobby.

## How to run (so far)

- Import: `cargo run --release -p rounds-import --bin rounds-import` → `cache/import` (0.8 s warm).
- Font check: `cargo run --release -p rounds-import --example font_sheet` → `scratch/fonts-*.png`.
- Map contact sheet: `cargo run --release -p rounds-import --bin map-sheet` → `scratch/maps-sheet.png`.
- Sim content report: `cargo run --release -p rounds-content --bin content-check -- --quiet [--json FILE]`.
- Sim collision sheet (4-player bot match, 300 ticks into the first battle): `cargo run --release -p rounds-content --bin sim-sheet` → `scratch/sim-sheet.png`; one map: `--map NAME` → `scratch/sim-<map>.png`.
- Content smoke test (every map, 4p and 12p, determinism): `cargo test --release -p rounds-content --test smoke -- --nocapture`.
- Renderer demo: `target/debug/rounds-port --headless --frames 120 --screenshot scratch/demo.png`.
- Windows: `scripts/fetch-windows-toolchain.sh && scripts/build-windows.sh && scripts/run-windows.sh --headless --frames 10 --screenshot scratch/win.png`.
- Netsim: `scripts/net/dev-server.sh` (ports via ROUNDS_WZ_PORT/ROUNDS_WZ_UDP_PORT; 2998 is taken by another project) then `cargo run -p rounds-net --example netsim -- run --locals 4x3`.
- Map screenshot: `target/debug/rounds-port --headless --map 3 --screenshot scratch/map-3.png`; sheet: `--map-sheet scratch/maps-hd.png --size 640x360`.
- Bot match: `target/debug/rounds-port --headless --bot-match --players 12 --shots 240,1560 --screenshot scratch/m.png` (prints a final `checksum:` line; `--pick-mode together`; mods: `--pick-n 2,7`).
- Prefab particle effects: `target/debug/rounds-port --headless --fx-preview A_Explosion,PlayerDeath --shots 5,20 --screenshot scratch/fx.png` (`all` = every prefab).
- Windowed on the private Xvfb: `scripts/dev/xvfb.sh` (+ `scripts/dev/xvfb-wm.sh`), then `env -u WAYLAND_DISPLAY DISPLAY=:61 SDL_VIDEODRIVER=x11 SDL_AUDIODRIVER=disk SDL_DISKAUDIOFILE=scratch/sdl-audio.raw target/debug/rounds-port [--virtual-pads FILE]`.
- Online test with Windows: `scripts/build-windows.sh && ONLINE_TEST_WINDOWS=1 scripts/net/online-test.sh win`.
- Reader validation: `cargo run --release -p unity-assets --example validate`.
- Sim benchmark: `cargo run --release -p rounds-sim --bin sim-bench -- 6000 --real` (real cards; needs `cache/import`).
- Card tests: `cargo test --release -p rounds-sim --test cards --test stress` (use `cache/import` or `ROUNDS_IMPORT_DIR`).

## Facts established

- Unity 2022.3.34f1, Mono, SerializedFile format 22, **no type trees** in the
  build. Scenes: `level0` = Main, `level1` = Lobby, `level2`–`level71` = 70
  maps (names in `BuildSettings`).
- Fixed timestep 0.014 s; Physics2D: 8 velocity / 3 position iterations.
- 79 `CardInfo` objects (78 named cards) across `resources.assets` and
  `sharedassets0.assets`.
- Audio: 924+ clips, FSB5 Vorbis (`m_CompressionFormat` 1). Textures: DXT5
  (306), RGBA32 (280), Alpha8 (79), RGB24 (3).
- Original networking: Photon PUN, 58 RPCs in 25 files; `PlayerAssigner.maxPlayers = 4`.
- `scripts/dev/dump_assets.py`: UnityPy + TypeTreeGeneratorAPI dump. Its
  generated MonoBehaviour header reads `m_Script` 3 bytes early; the script
  corrects it when resolving class names.
