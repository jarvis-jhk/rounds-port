# Notes

Running log of the project's maintenance: decisions, CI, releases, open
items. Newest first.

## 2026-10-19

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
