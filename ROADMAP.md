# Roadmap

What comes next for rounds-port. `docs/STATUS.md` records what is done;
this file is the plan. Ideas and requests: open an issue.

## Current priorities

1. **Launcher flow end to end** on warpzone.virtualtabletop.io: install,
   find data, import, play, host and join through the launcher.
2. **Steam Deck defaults**: fullscreen and pad-friendly menus when
   `SteamDeck=1`.
3. **Real hardware**: a session on a real GPU with physical controllers, and
   a real Windows machine (so far: Xvfb, lavapipe and Proton only).
4. **Feel pass against the original**: movement, bloom strength, menu
   brightness, sounds (needs reference footage).
5. **Missing effects**: chromatic aberration, particle trails and particle
   collision.
6. **Input gaps**: right-hand keyboard aim keys, glyph prompts outside the
   lobby.

## macOS support

A macOS build alongside Linux and Windows, after the current priorities.

- **Build**: Apple Silicon and Intel (a universal binary via `lipo`, or two
  archives), built and smoke-tested (`--headless` screenshot, cards and
  stress tests) on a GitHub `macos` runner in `ci.yml`, and attached to
  every release by `release.yml`.
- **Player docs**: install and start notes for macOS in `docs/players.md`.
  Unsigned is fine; document the Gatekeeper workaround (right-click → Open,
  or `xattr -dr com.apple.quarantine`).
- **Known risks**:
  - Rendering: wgpu must run on Metal there (no Vulkan or GL fallback worth
    having); the SDL window has to hand wgpu a Metal layer, and HiDPI
    (points vs. pixels) needs checking for menus and the offscreen path.
  - SDL2: bundle `SDL2.framework` (or link it statically) in the app
    bundle with a correct rpath; SDL 2 is in maintenance, so the SDL 3
    question comes up sooner here.
  - Controllers: macOS game-controller support goes through SDL's
    GCController/IOKit backends; mappings, hot-plug and 12 pads at once
    need a real Mac to verify.
  - Game data: ROUNDS has no native macOS release, so the player's files
    usually come from a Windows install (copied over, or via
    `steamcmd +@sSteamCmdForcePlatformType windows`); the Steam library
    scan (`Library/Application Support/Steam`) will rarely find it, so the
    docs and the data-dir prompt matter more. User data and cache paths go
    under `~/Library/Application Support`.
  - Warpzone client (`warpzone-core`): builds and connects on macOS
    untested; the launcher flow (find data, start the game) needs a macOS
    variant.
  - Platform paths written but never run on a Mac: system font lookup
    (`sysfonts.rs`), the Steam root scan (`paths.rs`), crash reports.

## Later

- **Warpzone tier 3**: a ranked queue (1v1 and 2v2) that reports placements
  with the simulation's checksum; maps or card sets as mods where they need
  no code.

## Done

- v0.1.0 (prerelease): all 79 cards and 70 maps, up to 12 players local and
  online with rollback, teams, Linux and Windows builds, Warpzone tier 2.
- Crash reports: panics are reported anonymously and become GitHub issues
  (`docs/players.md`, "Crash reports").
