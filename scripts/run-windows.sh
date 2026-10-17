#!/bin/bash
# Runs the cross-compiled rounds-port.exe under Proton's Wine, sandboxed so
# that nothing outside the project directory can be written:
#   - bubblewrap mounts / read-only, the project read-write, gives Wine a
#     private /tmp (tools/wine-tmp; wineserver puts its socket there) and a
#     private in-memory /dev/shm;
#   - HOME/XDG dirs and the Wine prefix live in tools/.
#
# Display: Wine's Vulkan (winevulkan) needs a user driver. With Wine's
# "null" graphics driver, Vulkan instance creation hangs (Proton
# experimental-11.0), so Wine connects to an X server as its graphics
# driver: by default the project's private Xvfb (scripts/dev/xvfb.sh, :61),
# so nothing ever appears on the real desktop and windowed runs are fine.
#   WINE_DISPLAY=:N     another private X display (default :61)
#   WINE_USER_DISPLAY=1 use $DISPLAY instead (the real desktop!); then only
#                       --headless/--list-devices/--help are allowed unless
#                       WINE_ALLOW_WINDOW=1
#   WINE_NULLDRV=1      use Wine's null graphics driver (no X connection;
#                       currently only --help works)
#
# Usage: scripts/run-windows.sh [rounds-port args...]
#   e.g. scripts/run-windows.sh --headless --frames 10 --screenshot scratch/win.png
# Env: PROTON_DIR (default: Steam's "Proton - Experimental"), EXE (default:
# target/dist/windows/rounds-port.exe), WINEDEBUG (default -all).
set -euo pipefail
source "$(dirname "$0")/../env.sh"
cd "$ROUNDS_ROOT"

PROTON_DIR=${PROTON_DIR:-"$HOME/.steam/steam/steamapps/common/Proton - Experimental"}
WINE="$PROTON_DIR/files/bin/wine"
EXE=${EXE:-target/dist/windows/rounds-port.exe}
[ -x "$WINE" ] || { echo "Proton wine not found at $WINE" >&2; exit 1; }
[ -f "$EXE" ] || { echo "$EXE missing: run scripts/build-windows.sh" >&2; exit 1; }

if [ "${WINE_USER_DISPLAY:-0}" = 1 ]; then
    safe=0
    for a in "$@"; do
        case "$a" in --headless|--list-devices|--help|-h|--version|-V) safe=1 ;; esac
    done
    if [ $safe = 0 ] && [ "${WINE_ALLOW_WINDOW:-0}" != 1 ]; then
        echo "refusing to open a window on the real desktop; set WINE_ALLOW_WINDOW=1" >&2
        exit 2
    fi
elif [ "${WINE_NULLDRV:-0}" != 1 ]; then
    export DISPLAY="$("$ROUNDS_ROOT/scripts/dev/xvfb.sh" "${WINE_DISPLAY:-:61}")"
    unset WAYLAND_DISPLAY
fi

T="$ROUNDS_ROOT/tools"
mkdir -p "$T/wine-home" "$T/wine-tmp" "$T/wine-xdg/config" "$T/wine-xdg/data" "$WINEPREFIX"

display_args=()
if [ "${WINE_NULLDRV:-0}" = 1 ]; then
    display_args=(--unsetenv DISPLAY --unsetenv WAYLAND_DISPLAY)
fi

sandbox() {
    bwrap \
        --ro-bind / / \
        --dev-bind /dev /dev \
        --tmpfs /dev/shm \
        --proc /proc \
        --bind "$ROUNDS_ROOT" "$ROUNDS_ROOT" \
        --bind "$T/wine-tmp" /tmp \
        --setenv HOME "$T/wine-home" \
        --setenv XDG_CONFIG_HOME "$T/wine-xdg/config" \
        --setenv XDG_DATA_HOME "$T/wine-xdg/data" \
        --setenv XDG_CACHE_HOME "$XDG_CACHE_HOME" \
        --setenv WINEPREFIX "$WINEPREFIX" \
        --setenv WINEDLLPATH "$PROTON_DIR/files/lib/vkd3d:$PROTON_DIR/files/lib/wine" \
        --setenv LD_LIBRARY_PATH "$PROTON_DIR/files/lib/x86_64-linux-gnu:$PROTON_DIR/files/lib/i386-linux-gnu" \
        --setenv WINEDLLOVERRIDES "winemenubuilder.exe=d;mscoree=d;mshtml=d;dxgi=n" \
        --setenv WINEDEBUG "${WINEDEBUG:--all}" \
        "${display_args[@]}" \
        --chdir "$ROUNDS_ROOT" \
        "$WINE" "$@"
}

# First run creates the prefix.
if [ ! -f "$WINEPREFIX/system.reg" ]; then
    sandbox wineboot -u
fi

# Graphics driver selection (HKCU\Software\Wine\Drivers\Graphics).
if [ "${WINE_NULLDRV:-0}" = 1 ]; then
    grep -q '"Graphics"="null"' "$WINEPREFIX/user.reg" || sandbox reg add 'HKCU\Software\Wine\Drivers' /v Graphics /d null /f >/dev/null
elif grep -q '"Graphics"=' "$WINEPREFIX/user.reg"; then
    sandbox reg delete 'HKCU\Software\Wine\Drivers' /v Graphics /f >/dev/null
fi

# wgpu's exe imports dxgi.dll. Wine's own dxgi needs wined3d + vkd3d, which
# Proton only wires up through its launcher; like Proton does for games, use
# DXVK's dxgi.dll (Vulkan-based) inside the prefix. Rendering itself goes
# through wgpu's Vulkan backend (winevulkan -> host Vulkan driver).
SYS32="$WINEPREFIX/drive_c/windows/system32"
DXVK_DXGI="$PROTON_DIR/files/lib/wine/dxvk/x86_64-windows/dxgi.dll"
cmp -s "$DXVK_DXGI" "$SYS32/dxgi.dll" || cp -f "$DXVK_DXGI" "$SYS32/dxgi.dll"

sandbox "$EXE" "$@"
