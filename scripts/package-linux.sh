#!/bin/bash
# Builds dist/rounds-port-<version>-linux-x86_64.tar.gz (and dist/SHA256SUMS).
#
# Layout (one top-level folder, as the Warpzone descriptor expects):
#   rounds-port-<version>/{rounds-port, play.sh, README.txt, LICENSE-*,
#                          THIRD_PARTY_NOTICES.md, licenses/, [lib/libSDL2-2.0.so.0]}
#
# Usage: scripts/package-linux.sh [--bundle-sdl] [--no-build] [--verify]
#   --bundle-sdl  also ship the build host's libSDL2-2.0.so.0 in lib/ (play.sh
#                 puts it first on LD_LIBRARY_PATH). Off by default: SDL2 is on
#                 every desktop distro and on the Steam Deck (SteamOS ships it
#                 because Steam needs it); a bundled copy built against the build
#                 host's X11/Wayland/audio libraries is often worse than the
#                 system's own. The binary's NEEDED entry is the soname, so it
#                 runs against any SDL2 >= 2.26.
#   --no-build    reuse target/release/rounds-port
#   --verify      unpack the archive into scratch/ and smoke-test it
#
# Build on the oldest glibc you want to support (the binary needs the build
# host's glibc or newer; CI builds on ubuntu-22.04 = glibc 2.35, which covers
# the Steam Deck). Needs `sdl2` development files or tools/lib/libSDL2.so
# (scripts/bootstrap-tools.sh creates the symlink).
set -euo pipefail
source "$(dirname "$0")/../env.sh"
cd "$ROUNDS_ROOT"
source scripts/package-common.sh

BUNDLE_SDL=0 BUILD=1 VERIFY=0
for a in "$@"; do
    case $a in
        --bundle-sdl) BUNDLE_SDL=1 ;;
        --no-build) BUILD=0 ;;
        --verify) VERIFY=1 ;;
        *) echo "unknown option $a" >&2; exit 2 ;;
    esac
done

VERSION=$(pkg_version)
[ -n "$VERSION" ] || { echo "cannot read the version from Cargo.toml" >&2; exit 1; }
NAME="rounds-port-$VERSION"
OUT="$DIST/$NAME-linux-x86_64.tar.gz"
STAGE="$DIST/.stage-linux"
EPOCH=$(pkg_epoch)

if [ $BUILD = 1 ]; then
    cargo build --release --locked -p rounds-port
fi
BIN=target/release/rounds-port
[ -x "$BIN" ] || { echo "$BIN missing" >&2; exit 1; }

# The binary must not need a newer glibc than the oldest system we support.
MAX_GLIBC="${ROUNDS_MAX_GLIBC:-2.35}"
if command -v readelf >/dev/null; then
    # Version requirements flagged WEAK (Rust std's optional pidfd_* symbols)
    # do not stop the loader on an older glibc, so they don't count.
    need=$(readelf -V "$BIN" | grep -E 'Name: GLIBC_[0-9.]+ +Flags: none' | grep -o 'GLIBC_[0-9.]*' | sed 's/GLIBC_//' | sort -uV | tail -1)
    if [ -n "$need" ] && [ "$(printf '%s\n%s\n' "$need" "$MAX_GLIBC" | sort -V | tail -1)" != "$MAX_GLIBC" ]; then
        echo "$BIN needs glibc $need > $MAX_GLIBC: build on an older system (ROUNDS_MAX_GLIBC overrides)" >&2
        exit 1
    fi
    echo "glibc needed: ${need:-none} (limit $MAX_GLIBC)"
fi

pkg_clean_stage "$STAGE"
PKG="$STAGE/$NAME"
mkdir -p "$PKG"
install -m 755 "$BIN" "$PKG/rounds-port"
if command -v strip >/dev/null; then
    strip --strip-all "$PKG/rounds-port"
else
    echo "warning: strip not found, shipping the unstripped binary" >&2
fi
install -m 755 packaging/linux/play.sh "$PKG/play.sh"
install -m 644 packaging/linux/README.txt "$PKG/README.txt"
pkg_stage_docs "$PKG" x86_64-unknown-linux-gnu

if [ $BUNDLE_SDL = 1 ]; then
    sdl=$(ldd "$BIN" | awk '/libSDL2-2\.0\.so\.0/ {print $3}')
    [ -f "$sdl" ] || { echo "libSDL2-2.0.so.0 not found" >&2; exit 1; }
    mkdir -p "$PKG/lib"
    install -m 755 "$(readlink -f "$sdl")" "$PKG/lib/libSDL2-2.0.so.0"
    install -m 644 packaging/licenses/SDL2-LICENSE.txt "$PKG/licenses/SDL2-LICENSE.txt"
fi

mkdir -p "$DIST"
chmod -R go-w "$STAGE"
find "$STAGE" -exec touch -h -d "@$EPOCH" {} +
(cd "$STAGE" && tar --sort=name --owner=0 --group=0 --numeric-owner --mtime="@$EPOCH" -cf - "$NAME") \
    | gzip -9n > "$OUT.part"
mv "$OUT.part" "$OUT"
rm -rf "$STAGE"

if [ $VERIFY = 1 ]; then
    V="$ROUNDS_ROOT/scratch/package-verify-linux"
    rm -rf "$V"; mkdir -p "$V"
    tar -xzf "$OUT" -C "$V"
    P="$V/$NAME"
    test -x "$P/play.sh" && test -x "$P/rounds-port"
    "$P/play.sh" --help | sed -n 1,3p
    # No window, no sound card needed.
    SDL_AUDIODRIVER=dummy SDL_VIDEODRIVER=dummy "$P/play.sh" --list-devices | sed -n 1,20p
    # Renderer demo (no game data): needs any Vulkan/GL adapter, lavapipe is enough.
    if SDL_AUDIODRIVER=dummy "$P/play.sh" --headless --frames 10 --screenshot "$V/demo.png"; then
        test -s "$V/demo.png" && echo "headless demo screenshot ok"
    else
        echo "warning: headless demo failed (no GPU adapter?)" >&2
        [ "${VERIFY_STRICT:-0}" = 1 ] && exit 1
    fi
fi

scripts/package-checksums.sh "$DIST" >/dev/null
echo "$OUT ($(human "$OUT"))"
