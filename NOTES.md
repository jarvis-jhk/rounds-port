# Notes

Running log of the project's maintenance: decisions, CI, releases, open
items. Newest first.

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
