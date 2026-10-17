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

## Later

- **Warpzone tier 3**: a ranked queue (1v1 and 2v2) that reports placements
  with the simulation's checksum; maps or card sets as mods where they need
  no code.
- **macOS** (wgpu on Metal, SDL2 framework).

## Done

- v0.1.0 (prerelease): all 79 cards and 70 maps, up to 12 players local and
  online with rollback, teams, Linux and Windows builds, Warpzone tier 2.
- Crash reports: panics are reported anonymously and become GitHub issues
  (`docs/players.md`, "Crash reports").
