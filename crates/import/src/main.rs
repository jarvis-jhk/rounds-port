//! CLI: `rounds-import [--game DIR] [--out DIR] [--force]`.
//!
//! Defaults: `--game $ROUNDS_GAME_DIR`, `--out cache/import` (relative to
//! `$ROUNDS_ROOT` when set).

use std::path::PathBuf;
use std::process::ExitCode;

fn usage() -> ExitCode {
    eprintln!("usage: rounds-import [--game DIR] [--out DIR] [--force]");
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let mut game = std::env::var_os("ROUNDS_GAME_DIR").map(PathBuf::from);
    let root = std::env::var_os("ROUNDS_ROOT")
        .map(PathBuf::from)
        .unwrap_or_default();
    let mut out = root.join("cache/import");
    let mut force = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--game" => match args.next() {
                Some(g) => game = Some(g.into()),
                None => return usage(),
            },
            "--out" => match args.next() {
                Some(o) => out = o.into(),
                None => return usage(),
            },
            "--force" => force = true,
            _ => return usage(),
        }
    }
    let Some(game_dir) = game else {
        eprintln!("no game dir: pass --game or set ROUNDS_GAME_DIR");
        return ExitCode::FAILURE;
    };
    let opts = rounds_import::Options {
        game_dir,
        out_dir: out.clone(),
        force,
    };
    match rounds_import::import(&opts) {
        Ok(r) if r.up_to_date => {
            println!("{} is up to date (use --force to re-import)", out.display());
            ExitCode::SUCCESS
        }
        Ok(r) => {
            for w in &r.warnings {
                eprintln!("warning: {w}");
            }
            let counts: Vec<String> = r.counts.iter().map(|(k, v)| format!("{v} {k}")).collect();
            println!(
                "imported into {} in {:.1}s: {}",
                out.display(),
                r.seconds,
                counts.join(", ")
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("import failed: {e:#}");
            ExitCode::FAILURE
        }
    }
}
