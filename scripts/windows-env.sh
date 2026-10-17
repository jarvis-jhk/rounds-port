# Source this (after env.sh) to cross-compile for Windows with the
# project-local llvm-mingw from scripts/fetch-windows-toolchain.sh:
#   source env.sh && source scripts/windows-env.sh
#   cargo build --release --target x86_64-pc-windows-gnu -p rounds-port
# llvm-mingw's bin dir goes on PATH because rustc calls
# x86_64-w64-mingw32-dlltool for raw-dylib imports (windows-rs crates).
_W="$ROUNDS_ROOT/tools/windows"
if [ ! -x "$_W/llvm-mingw/bin/x86_64-w64-mingw32-clang" ]; then
    echo "Windows toolchain missing: run scripts/fetch-windows-toolchain.sh" >&2
    return 1 2>/dev/null || exit 1
fi
export PATH="$_W/llvm-mingw/bin:$PATH"
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER="$_W/llvm-mingw/bin/x86_64-w64-mingw32-clang"
export CC_x86_64_pc_windows_gnu="$_W/llvm-mingw/bin/x86_64-w64-mingw32-clang"
export AR_x86_64_pc_windows_gnu="$_W/llvm-mingw/bin/llvm-ar"
unset _W
