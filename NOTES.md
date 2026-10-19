# Notes

Running log of the project's maintenance: decisions, CI, releases, open
items. Newest first.

## 2026-10-19

- Launcher crash report (exit 1 after 0 s, `--import --game
  ~/.local/share/Steam/steamapps/common/ROUNDS`, Linux, probably a Steam
  Deck with an SD-card library): "ROUNDS is not installed". The launcher had
  found that folder, but its check is case-insensitive and also accepts a
  folder with only `ROUNDS.exe`; ours wanted exactly `ROUNDS_Data`. Now:
  `*_Data` matched in any case (game and `AssetDir::open`), the install dir
  from Steam's `appmanifest_1557740.acf` is tried, `~/` is expanded, and the
  error says per path why it failed (missing, empty, exe but no data folder →
  "incomplete download? verify the game files"). The exact cause on the
  reporter's machine is unknown; the next report will say.
- Warpzone descriptor: `steamgriddb = 5281211` (launcher library tile, asked
  for by the warpzone lane for Warpzone 0.1.3). Servers <= 0.1.2 reject the
  field, so `check-games` needs a 0.1.3 dev server. The live copy on vtt and
  `games/rounds-port.toml` in jarvis-jhk/warpzone are not reachable from this
  lane: asked the warpzone lane to sync them once 0.1.3 runs on vtt.
- ROADMAP.md: "Popular mods, built in" (Jan's request, relayed by the adhoc
  lane): the top 20 ROUNDS mods, each optional. Slice 1: `docs/MODS.md`, the
  Thunderstore survey (downloads snapshot, frameworks and single patches
  folded in, licences). Slice 2: `rounds_sim::Mods` in `MatchSetup` and the
  lobby rules (online: travels in `NetRules`, old rules decode as all off),
  lobby MODS page, `--pick-n`, Pick N Cards as first mod (both pick modes,
  HUD fan and grid for 1-10 cards). `tests/mods.rs` pins vanilla checksums
  taken before the change; workspace tests (release, real data), fmt and
  clippy pass; netsim not run (needs a warpzone build in ext/).
- ROADMAP.md: "macOS support" milestone after the current priorities
  (requested via the adhoc lane for Jan); replaces the one-line macOS item
  under Later. Roadmap only, no implementation.

## 2026-10-17

- Taken over as a maintained project. CI green on b9b2791; v0.1.0
  (prerelease) built by `release.yml`. The prerelease flag stays on until
  the owner calls a release ready.
- Crash reports added: `crates/game/src/crash.rs` posts panics (release
  builds only, opt-out `ROUNDS_PORT_CRASH_REPORTS=0`) to a public ntfy topic;
  `.github/workflows/crash-reports.yml` polls it hourly and files issues
  labelled `crash` (one per fingerprint = version + panic location).
- `packaging/linux/README.txt` now asks for SDL 2.0.20 (what the binary
  needs since b9b2791).
- Open: Steam Deck defaults, launcher end-to-end check, real-hardware
  session, feel pass (see ROADMAP.md).
- Pre-push checks in the worker: fmt, clippy -D warnings, game tests, cards+stress, smoke (with real data) passed; netsim not run: `scripts/net/dev-server.sh` builds inside `ext/warpzone`, which workers must not do, and rounds-net/sim were not touched.
- Pipeline verified end to end: `--crash-test` → ntfy → workflow → issue #1 → lane request; #1 closed as the test. CI green on fc1d51c.
