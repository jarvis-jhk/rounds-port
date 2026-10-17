# rounds-port

A clean-room reimplementation of [ROUNDS](https://store.steampowered.com/app/1557740/ROUNDS/)
in Rust, for Linux and Windows (macOS later): up to 12 players, local on one
screen with controllers, online through [Warpzone](https://github.com/jarvis-jhk/warpzone),
or both mixed.

**Status: early development.** The simulation, all 79 cards and 70 maps, the
renderer with the original's look, the menus and the rollback netcode exist and
are tested headless; the local 12-player lobby and the online menus are being
finished, and the windowed game has not been shaken out on real desktops yet.
There is no stable release. Details and the milestone list:
[docs/STATUS.md](docs/STATUS.md).

You need your own copy of ROUNDS. rounds-port contains no game data: it reads
the art, sounds, maps and cards from your installation at first start
(read-only, converted into its own data folder). Not affiliated with Landfall
Games.

## Features

- The original's rules and feel: deterministic fixed-step simulation (0.014 s),
  all cards and maps, the original look (lighting, soft shadows, particles),
  menus, card picks, rounds and points.
- Up to **12 players**: any mix of local players (keyboard and mouse,
  keyboard-only halves, any number of controllers) and online players.
  Free-for-all or teams of any split.
- **Online over Warpzone**: public and private rooms, join codes, invite links,
  NAT traversal with relay fallback, **rollback netcode** (GGRS) so remote
  players feel local. Drop-outs are handled mid-match.
- Linux (x86_64, Steam Deck) and Windows (x86_64) builds; the Windows build
  is cross-compiled from Linux and runs under Wine/Proton too.
- Headless mode (offscreen rendering, no window, no sound card) for
  screenshots, bot matches and automated tests.

## Install and play

See [docs/players.md](docs/players.md): install (Warpzone launcher, or the
tarball / zip from the releases page), first start and import, controls,
local and online multiplayer, troubleshooting.

## Screenshots

*Placeholder: screenshots are added once there is a release. The repository
never contains images made from the original's art; project screenshots will
only show our own UI or be hosted outside the repository.*

## Building and contributing

```sh
scripts/bootstrap-tools.sh   # toolchains into ./tools (nothing outside the project)
source env.sh
cargo build --release
```

[docs/building.md](docs/building.md) covers the dev setup, the import, tests,
the Windows cross-build, the online dev server and netsim, packaging and CI.

| Doc | What |
|---|---|
| [docs/players.md](docs/players.md) | Installing and playing |
| [docs/building.md](docs/building.md) | Building, testing, packaging, releases |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | Crates, data flow, simulation design |
| [docs/platform.md](docs/platform.md) | Platform layer, renderer, local match |
| [docs/net.md](docs/net.md) | Online play, rollback, test results |
| [docs/STATUS.md](docs/STATUS.md) | Milestones and what is next |
| [ROADMAP.md](ROADMAP.md) | Current priorities |
| [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) | Third-party licenses |

The project is clean-room: behaviour is described in our own words in
separate specs (kept private, because they describe the original's internals),
and the Rust code is written from those; no game assets or decompiled code are
in the repository (see `CLAUDE.md`).

## License

Code: MIT or Apache-2.0, at your option ([LICENSE-MIT](LICENSE-MIT),
[LICENSE-APACHE](LICENSE-APACHE)). ROUNDS and its assets are (c) Landfall
Games and are not part of this project. Third-party components:
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
