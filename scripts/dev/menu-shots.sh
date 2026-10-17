#!/usr/bin/env bash
# Headless screenshots of every menu screen -> scratch/menus/<screen>.png
#   scripts/dev/menu-shots.sh [--size 1920x1080] [screen ...]
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
source env.sh
BIN="${MENU_SHOTS_BIN:-$ROOT/target/release/rounds-port}"
[ -x "$BIN" ] || cargo build --release -p rounds-port
SIZE=1920x1080
if [ "${1:-}" = "--size" ]; then SIZE=$2; shift 2; fi
SCREENS=("$@")
[ ${#SCREENS[@]} = 0 ] && SCREENS=(main local options controls maps lobby lobby12 lobby-teams editor
  register online host browse code onlinelobby pause)
mkdir -p scratch/menus
export ROUNDS_NO_SAVE=1
for s in "${SCREENS[@]}"; do
  frames=40
  [ "$s" = pause ] && frames=460
  "$BIN" --headless --screen "$s" --frames $frames --size "$SIZE" --screenshot "scratch/menus/$s.png" \
    2>&1 | grep -E "^screenshot|rror|panic" || true
done
