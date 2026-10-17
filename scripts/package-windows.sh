#!/bin/bash
# Builds dist/rounds-port-<version>-windows-x86_64.zip (and dist/SHA256SUMS)
# with the llvm-mingw cross toolchain (scripts/fetch-windows-toolchain.sh,
# scripts/build-windows.sh).
#
# Layout:
#   rounds-port-<version>/{rounds-port.exe, SDL2.dll, README.txt, LICENSE-*,
#                          THIRD_PARTY_NOTICES.md, licenses/ (incl. SDL2 zlib)}
#
# Usage: scripts/package-windows.sh [--no-build] [--verify]
#   --no-build  reuse target/dist/windows/ from the last scripts/build-windows.sh
#   --verify    unpack the zip into scratch/ and run it headless under
#               scripts/run-windows.sh (Proton's Wine, sandboxed): --help,
#               --list-devices, a renderer-demo screenshot and, when an import
#               exists (cache/import or the user data dir), a map screenshot
set -euo pipefail
source "$(dirname "$0")/../env.sh"
cd "$ROUNDS_ROOT"
source scripts/package-common.sh

BUILD=1 VERIFY=0
for a in "$@"; do
    case $a in
        --no-build) BUILD=0 ;;
        --verify) VERIFY=1 ;;
        *) echo "unknown option $a" >&2; exit 2 ;;
    esac
done

VERSION=$(pkg_version)
[ -n "$VERSION" ] || { echo "cannot read the version from Cargo.toml" >&2; exit 1; }
NAME="rounds-port-$VERSION"
OUT="$DIST/$NAME-windows-x86_64.zip"
STAGE="$DIST/.stage-windows"
EPOCH=$(pkg_epoch)

if [ $BUILD = 1 ]; then
    scripts/build-windows.sh >/dev/null   # fetches the toolchain if missing; release profile
fi
SRC=target/dist/windows
[ -f "$SRC/rounds-port.exe" ] && [ -f "$SRC/SDL2.dll" ] || { echo "$SRC incomplete: run scripts/build-windows.sh" >&2; exit 1; }
STRIP=tools/windows/llvm-mingw/bin/llvm-strip

pkg_clean_stage "$STAGE"
PKG="$STAGE/$NAME"
mkdir -p "$PKG"
install -m 755 "$SRC/rounds-port.exe" "$PKG/rounds-port.exe"
"$STRIP" --strip-all "$PKG/rounds-port.exe"
install -m 644 "$SRC/SDL2.dll" "$PKG/SDL2.dll"
# Notepad-friendly line endings for the readme.
sed 's/$/\r/' packaging/windows/README.txt > "$PKG/README.txt"
pkg_stage_docs "$PKG" x86_64-pc-windows-gnu
install -m 644 packaging/licenses/SDL2-LICENSE.txt "$PKG/licenses/SDL2-LICENSE.txt"

mkdir -p "$DIST"
chmod -R go-w "$STAGE"
find "$STAGE" -exec touch -h -d "@$EPOCH" {} +
rm -f "$OUT" "$OUT.part"
(cd "$STAGE" && find "$NAME" | LC_ALL=C sort | zip -X -9 -q "$OUT.part" -@)
mv "$OUT.part" "$OUT"
rm -rf "$STAGE"

if [ $VERIFY = 1 ]; then
    V="$ROUNDS_ROOT/scratch/package-verify-windows"
    rm -rf "$V"; mkdir -p "$V"
    unzip -q "$OUT" -d "$V"
    P="$V/$NAME"
    test -f "$P/rounds-port.exe" && test -f "$P/SDL2.dll"
    export EXE="$P/rounds-port.exe"
    scripts/run-windows.sh --help | sed -n 1,3p
    scripts/run-windows.sh --list-devices | sed -n 1,20p
    scripts/run-windows.sh --headless --frames 10 --screenshot "$V/demo.png"
    test -s "$V/demo.png" && echo "headless demo screenshot ok"
    DATA=""
    for d in "${ROUNDS_IMPORT_DIR:-}" cache/import; do
        [ -n "$d" ] && [ -f "$d/index.json" ] && { DATA=$(readlink -f "$d"); break; }
    done
    if [ -n "$DATA" ]; then
        scripts/run-windows.sh --headless --data "$DATA" --map 3 --screenshot "$V/map3.png"
        test -s "$V/map3.png" && echo "headless map screenshot ok ($V/map3.png)"
    fi
fi

scripts/package-checksums.sh "$DIST" >/dev/null
echo "$OUT ($(human "$OUT"))"
