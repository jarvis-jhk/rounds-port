//! Adds the project-local SDL2 link directories (see docs/platform.md):
//! Linux: `tools/lib` (symlink to the system's libSDL2-2.0.so.0, no -dev
//! package needed); Windows (cross): the official SDL2 mingw development
//! release unpacked by `scripts/fetch-windows-toolchain.sh`, plus the
//! libgcc shims that script creates.
//! `ROUNDS_SDL2_LIB_DIR` overrides both.

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=ROUNDS_SDL2_LIB_DIR");
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        windows_resources();
    }
    if let Ok(dir) = std::env::var("ROUNDS_SDL2_LIB_DIR") {
        println!("cargo:rustc-link-search=native={dir}");
        return;
    }
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let target = std::env::var("TARGET").unwrap_or_default();
    let dirs: &[&str] = if target.contains("windows") {
        // SDL2 import library, plus libgcc/libgcc_eh shims that map GCC's
        // runtime onto llvm-mingw's compiler-rt/libunwind.
        &[
            "tools/windows/sdl2/x86_64-w64-mingw32/lib",
            "tools/windows/gcc-shim",
        ]
    } else {
        &["tools/lib"]
    };
    for d in dirs {
        let dir = root.join(d);
        if dir.exists() {
            println!(
                "cargo:rustc-link-search=native={}",
                dir.canonicalize().unwrap().display()
            );
        }
    }
}

/// Windows targets: an application icon (drawn here, procedurally: no game
/// art) and a VERSIONINFO resource, compiled with the llvm-mingw `windres`
/// (`WINDRES` overrides; on `PATH` through `scripts/windows-env.sh`) and
/// linked into the exe. Skipped with a warning when windres is missing.
fn windows_resources() {
    println!("cargo:rerun-if-env-changed=WINDRES");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let ico = out.join("rounds-port.ico");
    std::fs::write(&ico, icon::ico(&[16, 24, 32, 48, 64, 128, 256])).unwrap();
    let ver = std::env::var("CARGO_PKG_VERSION").unwrap();
    let mut nums: Vec<u32> = ver
        .split(['.', '-', '+'])
        .take(3)
        .map(|x| x.parse().unwrap_or(0))
        .collect();
    nums.resize(4, 0);
    let commas = nums
        .iter()
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let rc = format!(
        r#"1 ICON "{ico}"
1 VERSIONINFO
FILEVERSION {commas}
PRODUCTVERSION {commas}
FILEFLAGSMASK 0x3f
FILEFLAGS 0
FILEOS 0x40004
FILETYPE 1
FILESUBTYPE 0
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "CompanyName", "rounds-port contributors"
      VALUE "FileDescription", "rounds-port (clean-room ROUNDS reimplementation)"
      VALUE "FileVersion", "{ver}"
      VALUE "InternalName", "rounds-port"
      VALUE "LegalCopyright", "MIT OR Apache-2.0"
      VALUE "OriginalFilename", "rounds-port.exe"
      VALUE "ProductName", "rounds-port"
      VALUE "ProductVersion", "{ver}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#,
        ico = ico.display().to_string().replace('\\', "/"),
    );
    let rc_path = out.join("rounds-port.rc");
    std::fs::write(&rc_path, rc).unwrap();
    let obj = out.join("rounds-port-res.o");
    let windres =
        std::env::var("WINDRES").unwrap_or_else(|_| "x86_64-w64-mingw32-windres".to_string());
    let status = std::process::Command::new(&windres)
        .arg("--no-preprocess")
        .arg("-O")
        .arg("coff")
        .arg("-i")
        .arg(&rc_path)
        .arg("-o")
        .arg(&obj)
        .status();
    match status {
        Ok(s) if s.success() => {
            println!("cargo:rustc-link-arg-bins={}", obj.display());
        }
        other => println!(
            "cargo:warning=no exe icon/version resource ({windres}: {other:?}); \
             source scripts/windows-env.sh to put windres on PATH"
        ),
    }
}

/// A small procedural icon: two overlapping discs (orange and blue) with a
/// white outline on a dark rounded square. Writes a multi-size .ico with
/// 32-bit BMP images.
mod icon {
    fn pixel(x: f32, y: f32) -> [f32; 4] {
        // x, y in 0..1, y down. Rounded square background.
        let r = 0.18;
        let qx = (x - 0.5).abs() - (0.5 - r);
        let qy = (y - 0.5).abs() - (0.5 - r);
        let d_bg = (qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0)) - r;
        if d_bg > 0.0 {
            return [0.0; 4];
        }
        let mut c = [0.11, 0.11, 0.13, 1.0];
        let disc = |cx: f32, cy: f32, rad: f32| (x - cx).hypot(y - cy) - rad;
        for (cx, cy, col) in [
            (0.38, 0.56, [0.93, 0.42, 0.20]),
            (0.64, 0.42, [0.25, 0.47, 0.89]),
        ] {
            let d = disc(cx, cy, 0.24);
            if d < 0.0 {
                c = if d > -0.035 {
                    [0.97, 0.97, 0.97, 1.0]
                } else {
                    [col[0], col[1], col[2], 1.0]
                };
            }
        }
        c
    }

    fn image(size: u32) -> Vec<u8> {
        // 4x4 supersampling, BGRA, bottom-up rows, then a 1-bit AND mask.
        let mut px = Vec::with_capacity((size * size * 4) as usize);
        for row in (0..size).rev() {
            for col in 0..size {
                let mut acc = [0.0f32; 4];
                for sy in 0..4 {
                    for sx in 0..4 {
                        let x = (col as f32 + (sx as f32 + 0.5) / 4.0) / size as f32;
                        let y = (row as f32 + (sy as f32 + 0.5) / 4.0) / size as f32;
                        let p = pixel(x, y);
                        for k in 0..3 {
                            acc[k] += p[k] * p[3];
                        }
                        acc[3] += p[3];
                    }
                }
                let a = acc[3] / 16.0;
                let un = |v: f32| {
                    if acc[3] > 0.0 {
                        ((v / acc[3]) * 255.0).round().clamp(0.0, 255.0) as u8
                    } else {
                        0
                    }
                };
                px.extend_from_slice(&[un(acc[2]), un(acc[1]), un(acc[0]), (a * 255.0) as u8]);
            }
        }
        let mut out = Vec::new();
        out.extend_from_slice(&40u32.to_le_bytes());
        out.extend_from_slice(&(size as i32).to_le_bytes());
        out.extend_from_slice(&(size as i32 * 2).to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&[0u8; 24]);
        out.extend_from_slice(&px);
        let mask_row = (size.div_ceil(32) * 4) as usize;
        out.extend(std::iter::repeat_n(0u8, mask_row * size as usize));
        out
    }

    pub fn ico(sizes: &[u32]) -> Vec<u8> {
        let images: Vec<Vec<u8>> = sizes.iter().map(|&s| image(s)).collect();
        let mut out = Vec::new();
        out.extend_from_slice(&[0, 0, 1, 0]);
        out.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
        let mut offset = 6 + 16 * sizes.len() as u32;
        for (s, img) in sizes.iter().zip(&images) {
            let b = if *s >= 256 { 0 } else { *s as u8 };
            out.extend_from_slice(&[b, b, 0, 0]);
            out.extend_from_slice(&1u16.to_le_bytes());
            out.extend_from_slice(&32u16.to_le_bytes());
            out.extend_from_slice(&(img.len() as u32).to_le_bytes());
            out.extend_from_slice(&offset.to_le_bytes());
            offset += img.len() as u32;
        }
        for img in images {
            out.extend_from_slice(&img);
        }
        out
    }
}
