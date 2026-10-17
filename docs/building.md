# Building and developing

Everything a developer needs lives in `./tools` (git-ignored): the Rust
toolchain, .NET and ILSpy (only for the reference decompilation), a Python
venv, the Windows cross toolchain. Nothing is installed system-wide and
nothing outside the project directory is written. Players do not need any of
this: see [players.md](players.md).

## Setup

Prerequisites on the host: `curl`, `python3`, a C compiler (`cc`), the SDL2
runtime library (`libSDL2-2.0.so.0`; no `-dev` package needed locally) and a
Vulkan or OpenGL driver. For the Windows run script also `bwrap`
(bubblewrap) and Steam's "Proton - Experimental".

```sh
scripts/bootstrap-tools.sh     # rustup + stable, Windows target, dotnet, ilspycmd, venv, tools/lib/libSDL2.so
source env.sh                  # every shell: PATH, RUSTUP_HOME, CARGO_HOME, ... all under ./tools
cargo build --release          # or: cargo build  (dev profile: dependencies at opt-level 3)
```

`env.sh` also exports `ROUNDS_GAME_DIR` (default: the Steam install,
`~/.steam/steam/steamapps/common/ROUNDS`) and `ROUNDS_PORT_DATA_DIR`
(`cache/userdata`, so development never touches `~/.local/share`). Override
either before sourcing.

On a CI-like machine with `libsdl2-dev` installed, `tools/lib` is not needed:
`crates/game/build.rs` only adds it to the link path when it exists
(`ROUNDS_SDL2_LIB_DIR` overrides both).

## The game data (import)

The repository holds no game data. The game and the tests read the player's
own ROUNDS install (read-only) through a converted copy:

```sh
target/release/rounds-port --import              # find ROUNDS, convert into the user data dir
target/release/rounds-port --import --game /path/to/ROUNDS
cargo run --release -p rounds-import --bin rounds-import   # dev copy: cache/import (used by tests and tools)
```

`--import` looks for the install in this order: `--game DIR`,
`$ROUNDS_GAME_DIR`, `$WARPZONE_DATA_ROUNDS` (set by the Warpzone launcher),
then every Steam library; see [platform.md](platform.md). A warm re-import
takes about a second; a changed import format (`FORMAT_VERSION` in
`crates/import/src/format.rs`) re-imports on the next start.

## Running

Headless only on the development machine (no windows on the desktop):

```sh
target/release/rounds-port --headless --map 3 --screenshot scratch/map-3.png
target/release/rounds-port --headless --bot-match --players 12 --shots 240,1560 --screenshot scratch/m.png
target/release/rounds-port --headless --screen main --screenshot scratch/menu.png
target/release/rounds-port --list-devices
```

The windowed game is `target/release/rounds-port` with no arguments. All
options: `rounds-port --help`, and [platform.md](platform.md).
`RUST_LOG=info|debug` turns logging up; `WGPU_BACKEND=vulkan|gl` and
`WGPU_ADAPTER_NAME=llvmpipe` select the GPU (llvmpipe = software Vulkan,
useful for CI and for checking GPU-independent output).

## Tests, formatting, lints

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo test --release --workspace         # faster simulation tests, slower build
```

Tests that need the game data (`rounds-sim` card tests and stress tests,
`rounds-content` smoke test, `rounds-import` dev-import test) look for
`cache/import` (or `ROUNDS_IMPORT_DIR` / `ROUNDS_IMPORT`) and **return early
when it is absent**, so a plain checkout (and CI) passes without ROUNDS. The
renderer's pixel tests need any Vulkan/GL adapter (lavapipe is enough) and
skip with a note without one. The rest, including the in-process online
matches of `rounds-net`, runs everywhere. Run the full set after
`cargo run --release -p rounds-import --bin rounds-import`.

## Online: dev server and netsim

```sh
scripts/net/dev-server.sh                  # local warpzone-server (ext/warpzone), http://127.0.0.1:2998
ROUNDS_WZ_PORT=3998 ROUNDS_WZ_UDP_PORT=3479 scripts/net/dev-server.sh   # if 2998 is taken
export WARPZONE_SERVER=http://127.0.0.1:2998
cargo run -p rounds-net --example netsim -- run --locals 4x3 --frames 3000
scripts/net/netsim-matrix.sh               # the whole multi-process matrix (minutes, keeps all cores busy)
scripts/net/online-test.sh                 # full headless matches of the real binary, 12 players over 3-4 machines
```

`ext/warpzone` is a re-clonable checkout of the Warpzone repository (server,
launcher, docs). The dev server serves our descriptor from `packaging/warpzone/`
and validates it with `check-games` on every start. Details, link simulation
(`ROUNDS_NET_DELAY_MS`, `ROUNDS_NET_JITTER_MS`, `ROUNDS_NET_LOSS`,
`ROUNDS_NET_FORCE_RELAY`) and results: [net.md](net.md).

## Windows (cross-compiled from Linux)

```sh
scripts/fetch-windows-toolchain.sh   # llvm-mingw + SDL2 mingw release into tools/windows (SHA-256 pinned)
scripts/build-windows.sh             # -> target/dist/windows/{rounds-port.exe, SDL2.dll}
scripts/run-windows.sh --list-devices
scripts/run-windows.sh --headless --map 3 --screenshot scratch/win.png
```

`run-windows.sh` runs the exe in Proton's Wine inside a bubblewrap sandbox
that can only write to the project; it refuses arguments that would open a
window. See [platform.md](platform.md#windows-cross-compiled-from-linux).

## Release archives

```sh
scripts/package-linux.sh [--bundle-sdl] [--verify]    # dist/rounds-port-<version>-linux-x86_64.tar.gz
scripts/package-windows.sh [--verify]                 # dist/rounds-port-<version>-windows-x86_64.zip
scripts/package-checksums.sh                          # dist/SHA256SUMS (both scripts run it)
```

`<version>` is `[workspace.package] version` in `Cargo.toml` (also the version
the game reports to Warpzone). `dist/` is git-ignored and excluded from the
backup. Each archive has one top-level folder `rounds-port-<version>/` with the
binary, a README for players, `LICENSE-*`, `THIRD_PARTY_NOTICES.md` and
`licenses/` (the licenses of all linked Rust crates, generated by
`packaging/gen-licenses.py`). Archives are reproducible for a given
`SOURCE_DATE_EPOCH` and build.

* **Linux**: the release binary is stripped (21 MB). `play.sh` starts it
  (prefers a bundled `lib/` when present). SDL2 is **not** bundled by default:
  every desktop distribution and SteamOS ship `libSDL2-2.0.so.0`, the binary
  links the soname, and a copy built against the build host's X11/Wayland/audio
  stack is often worse than the system's. `--bundle-sdl` copies the build
  host's library into `lib/` (zlib license included) for systems without SDL2.
  The script refuses a binary that needs a newer glibc than 2.35 (Ubuntu
  22.04, which CI builds on; `ROUNDS_MAX_GLIBC` overrides). One tarball serves
  desktop Linux and the Steam Deck (the descriptor maps both platforms to it).
* **Windows**: `rounds-port.exe` (stripped with llvm-strip) and `SDL2.dll`
  (official SDL2 2.32.10 mingw build); `--verify` unpacks the zip into
  `scratch/` and runs it under `run-windows.sh`: `--help`, `--list-devices`,
  the renderer demo and, with a dev import present, a map screenshot (which is
  byte-identical to the Linux build's).

## Continuous integration and releases

`.github/workflows/ci.yml` (push to `main`, pull requests): `cargo fmt --check`,
`clippy -D warnings`, `cargo test --workspace`, a Linux package build with a
smoke test of the unpacked tarball (`--list-devices`, headless renderer demo on
lavapipe) and the Windows cross-build with the pinned llvm-mingw/SDL2. All on
`ubuntu-22.04` with `libsdl2-dev` from apt; Rust is installed into `tools/` by
`.github/actions/setup-rust` so `env.sh` and the scripts behave as on a
developer machine.

`.github/workflows/release.yml` runs on a tag `v<version>` (must equal the
version in `Cargo.toml`): builds both archives, writes `SHA256SUMS` over them
and creates the GitHub release (a prerelease if the tag has a `-suffix`).

To cut a release: bump `[workspace.package] version`, update `Cargo.lock`
(any `cargo build` does it), commit, tag
`v<version>`, push the tag. If the import format version changed, update the
two `present` paths in `packaging/warpzone/rounds-port.toml`.

## Warpzone descriptor

`packaging/warpzone/rounds-port.toml` tells Warpzone's server, website and
launcher how to install and start the game:

* `[install]` names the release assets (`rounds-port-*-linux-x86_64.tar.gz`,
  `...-windows-x86_64.zip`, `SHA256SUMS`); the launcher verifies downloads
  against GitHub's digest or `SHA256SUMS`, unpacks into
  `<warpzone data>/games/rounds-port/<version>/` and writes `.warpzone-managed`
  there (rounds-port has no updater of its own, so there is nothing to silence).
* `[launch]` runs `rounds-port-*/play.sh` (Linux, Steam Deck) or
  `rounds-port-*/rounds-port.exe`; `--warpzone-join <room>` and
  `--warpzone-host` are handled by the game.
* `[[data]]` finds ROUNDS (Steam libraries, Proton prefixes; `ROUNDS_Data/globalgamemanagers`
  or `ROUNDS.exe`), passes it as `WARPZONE_DATA_ROUNDS` on every start, and runs
  `rounds-port --import --game {path}` once until
  `import-v1/index.json` exists in the data dir.

Validate after every change (uses the dev server build in `scratch/net`):

```sh
scripts/net/dev-server.sh --build
scratch/net/wz-target/debug/warpzone-server check-games packaging/warpzone
```
