# Source this: keeps every toolchain and cache inside the project directory.
R="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export ROUNDS_ROOT="$R"
export RUSTUP_HOME="$R/tools/rustup" CARGO_HOME="$R/tools/cargo"
export DOTNET_ROOT="$R/tools/dotnet" DOTNET_CLI_HOME="$R/tools/dotnet-home"
export NUGET_PACKAGES="$R/tools/nuget" DOTNET_CLI_TELEMETRY_OPTOUT=1 DOTNET_NOLOGO=1 DOTNET_ROLL_FORWARD=Major
export npm_config_cache="$R/tools/npm-cache" PIP_CACHE_DIR="$R/tools/pip-cache"
export XDG_CACHE_HOME="$R/tools/xdg-cache" WINEPREFIX="$R/tools/wineprefix"
export PATH="$R/tools/cargo/bin:$R/tools/dotnet:$R/tools/dotnet-home/.dotnet/tools:$R/tools/venv/bin:$PATH"
# Location of the original game (read-only; never written to).
# A read-only /data/rounds (a sandboxed build worker) wins over the Steam default.
if [ -z "${ROUNDS_GAME_DIR:-}" ] && [ -d /data/rounds ]; then ROUNDS_GAME_DIR=/data/rounds; fi
export ROUNDS_GAME_DIR="${ROUNDS_GAME_DIR:-$HOME/.steam/steam/steamapps/common/ROUNDS}"
# Dev: the game's user data dir (import cache, settings, Warpzone identity) stays in the project.
export ROUNDS_PORT_DATA_DIR="$R/cache/userdata"
