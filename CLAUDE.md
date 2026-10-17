# rounds-port — rules for agents

A clean-room Rust reimplementation of ROUNDS (Landfall, Unity 2022.3.34f1) for
Linux and Windows (macOS later), with local play for up to 12 players on one
screen and online play over Warpzone. Read `docs/ARCHITECTURE.md` and
`docs/STATUS.md` first; `docs/STATUS.md` is the single source of truth for what
is done and what is next.

## Hard rules

- **No changes outside this directory.** Every tool and cache lives in
  `./tools` (`source env.sh` sets RUSTUP_HOME, CARGO_HOME, NUGET, pip, XDG cache,
  WINEPREFIX). No system packages, no `~/.cargo`, no global config.
- **The Steam install is read-only** (`$ROUNDS_GAME_DIR`). Never write into it.
- **Nothing from the game goes into git**: no assets, no decompiled code, no
  dumped data, no screenshots containing game art in committed files. The repo
  is public open source; the game's data is loaded from the player's own copy.
- **Clean room.** `ref/decomp/` (ILSpy output) and `ref/dump/` (JSON dump of
  every Unity object) are references only, git-ignored. Behaviour is described
  in our own words in `docs/spec/*.md` (formulas and constants are fine, code is
  not). Rust code is written from the specs; never transliterate C# from
  `ref/decomp`. If a spec is ambiguous, consult the decompilation, then fix the
  spec.
- **The specs are private.** `docs/spec/` is its own git checkout of the
  private repository `jarvis-jhk/rounds-port-spec` and is ignored here; commit
  spec changes there. Never publish, quote or paste them (this repo, issues,
  PRs, commit messages, release notes); code comments may cite a section
  (`docs/spec/weapons.md 7.2`), not quote it.
- **Agents do not run git** unless the orchestrator says so; the orchestrator
  commits.
- CPU-heavy work (> ~30 s: release builds, test suites, imports, long headless
  runs): wrap it in the machine's performance-mode helper when it has one.
- Headless only: no windows on the user's desktop. Use the offscreen renderer
  (`--headless`) for screenshots; put them in `scratch/` (git-ignored).

## Layout

| Path | What |
|---|---|
| `crates/unity` | `unity-assets`: Unity 2022.3 SerializedFile reader (no type trees), class readers, texture decode, FSB5 audio |
| `crates/import` | `rounds-import`: Steam install → our own data format in the user data dir |
| `crates/sim` | `rounds-sim`: deterministic, rollback-able game simulation (no I/O, no rendering) |
| `crates/render` | `rounds-render`: wgpu renderer, offscreen/headless capable |
| `crates/net` | `rounds-net`: GGRS rollback over Warpzone peer channels |
| `crates/game` | `rounds-port` binary: SDL2 platform layer, menus, app flow |
| `docs/spec/` | clean-room behaviour specs written from the reference (private checkout, see above; `review/` = sim-vs-spec review) |
| `scripts/` | bootstrap and dev scripts (`scripts/dev/dump_assets.py` regenerates `ref/dump`) |
| `ext/` | re-clonable third-party checkouts (warpzone, rea), git-ignored |
| `ref/`, `cache/`, `scratch/`, `tools/`, `target/` | git-ignored |

## Conventions

- `source env.sh` before cargo/python/dotnet. Python tools: `python -I`.
- Rust 2024 edition, `cargo fmt`, `cargo clippy` clean, unit tests next to the
  code. Determinism-sensitive code (sim) uses `libm` for transcendental math,
  no `HashMap` iteration in logic, no wall-clock time, no threads.
- Keep `docs/STATUS.md` current when finishing a step.
