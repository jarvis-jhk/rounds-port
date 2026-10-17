# Shared helpers of scripts/package-linux.sh and scripts/package-windows.sh
# (sourced, after env.sh and `cd "$ROUNDS_ROOT"`).

DIST="${ROUNDS_DIST_DIR:-$ROUNDS_ROOT/dist}"

# Release version: the workspace version, which is also what the game reports
# to Warpzone (CARGO_PKG_VERSION) and what the release tag must say.
pkg_version() {
    sed -n '/^\[workspace\.package\]/,/^\[/{s/^version *= *"\(.*\)"/\1/p}' "$ROUNDS_ROOT/Cargo.toml" | head -1
}

# Timestamp for reproducible archives: $SOURCE_DATE_EPOCH or now.
pkg_epoch() { echo "${SOURCE_DATE_EPOCH:-$(date +%s)}"; }

# pkg_stage_docs DIR TRIPLE: licenses and notices into a package directory.
pkg_stage_docs() {
    local dir=$1 triple=$2
    install -m 644 LICENSE-MIT LICENSE-APACHE THIRD_PARTY_NOTICES.md "$dir/"
    mkdir -p "$dir/licenses"
    install -m 644 crates/render/assets/fonts/OFL.txt "$dir/licenses/OFL.txt"
    python3 -I packaging/gen-licenses.py "$dir/licenses/RUST_DEPENDENCIES.txt" "$triple"
    chmod 644 "$dir/licenses/RUST_DEPENDENCIES.txt"
}

# pkg_clean_stage DIR: recreate an empty staging directory.
pkg_clean_stage() { rm -rf "$1"; mkdir -p "$1"; }

human() { LC_ALL=C numfmt --to=iec --suffix=B "$(stat -c %s "$1")"; }
