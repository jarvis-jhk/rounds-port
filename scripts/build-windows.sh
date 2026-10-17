#!/bin/bash
# Cross-compiles rounds-port.exe for x86_64 Windows and stages it with
# SDL2.dll in target/dist/windows/. Usage: scripts/build-windows.sh [--debug]
set -euo pipefail
source "$(dirname "$0")/../env.sh"
cd "$ROUNDS_ROOT"
[ -d tools/windows/llvm-mingw ] || scripts/fetch-windows-toolchain.sh
source scripts/windows-env.sh
PROFILE=release; FLAG=--release
if [ "${1:-}" = "--debug" ]; then PROFILE=debug; FLAG=; fi
cargo build $FLAG --target x86_64-pc-windows-gnu -p rounds-port
OUT=target/dist/windows
mkdir -p "$OUT"
cp -f "target/x86_64-pc-windows-gnu/$PROFILE/rounds-port.exe" "$OUT/"
cp -f tools/windows/sdl2/x86_64-w64-mingw32/bin/SDL2.dll "$OUT/"
echo "staged: $OUT/"
ls -la "$OUT"
