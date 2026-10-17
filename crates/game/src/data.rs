//! Startup data flow: find the install, (re-)import into the user data dir
//! when missing or outdated, load [`GameData`].

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use rounds_import::GameData;

use crate::paths;

/// Where the game data comes from.
#[derive(Clone, Debug, Default)]
pub struct DataOptions {
    /// `--game DIR`.
    pub game: Option<PathBuf>,
    /// `--data DIR`: use this import dir as is (no install needed).
    pub import_dir: Option<PathBuf>,
}

/// What [`prepare`] decided.
#[derive(Clone, Debug)]
pub struct Plan {
    /// The import dir to load.
    pub import_dir: PathBuf,
    /// Import to run first (`None`: use `import_dir` as is).
    pub import: Option<rounds_import::Options>,
    /// Description of the install, for logs and the progress screen.
    pub install: Option<paths::Install>,
}

/// Decides which import dir to use and whether an import must run. Cheap
/// (no import runs here).
pub fn prepare(opts: &DataOptions) -> Result<Plan> {
    if let Some(d) = &opts.import_dir {
        return Ok(Plan {
            import_dir: d.clone(),
            import: None,
            install: None,
        });
    }
    let data = paths::data_dir()
        .context("no user data dir (set ROUNDS_PORT_DATA_DIR, or HOME / APPDATA)")?;
    let import_dir = data.join(rounds_import::versioned_dir_name());
    let install = match paths::find_install(opts.game.as_deref()) {
        Ok(i) => i,
        Err(tried) => {
            // An existing import is still usable without the install.
            if import_dir.join("index.json").is_file() {
                log::warn!("ROUNDS install not found; using the existing import");
                return Ok(Plan {
                    import_dir,
                    import: None,
                    install: None,
                });
            }
            return Err(anyhow!(
                "ROUNDS is not installed or was not found. Pass --game <dir> or set \
                 ROUNDS_GAME_DIR to the folder containing ROUNDS_Data. Tried:\n  {}",
                if tried.is_empty() {
                    "(no Steam library found)".to_string()
                } else {
                    tried.join("\n  ")
                }
            ));
        }
    };
    Ok(Plan {
        import: Some(rounds_import::Options {
            game_dir: install.dir.clone(),
            out_dir: import_dir.clone(),
            force: false,
        }),
        import_dir,
        install: Some(install),
    })
}

/// Runs the plan's import if needed (skips quickly when up to date).
pub fn run_import(plan: &Plan) -> Result<()> {
    let Some(opts) = &plan.import else {
        return Ok(());
    };
    if let Some(parent) = opts.out_dir.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let report = rounds_import::import(opts).with_context(|| {
        format!(
            "importing {} into {}",
            opts.game_dir.display(),
            opts.out_dir.display()
        )
    })?;
    if report.up_to_date {
        log::info!("import {} is up to date", opts.out_dir.display());
    } else {
        for w in &report.warnings {
            log::warn!("import: {w}");
        }
        println!(
            "imported game data into {} in {:.1}s",
            opts.out_dir.display(),
            report.seconds
        );
    }
    Ok(())
}

/// Loads the import dir.
pub fn load(dir: &Path) -> Result<GameData> {
    GameData::load(dir).with_context(|| format!("loading game data from {}", dir.display()))
}

/// The whole flow, synchronously (headless, tools).
pub fn prepare_and_load(opts: &DataOptions) -> Result<GameData> {
    let plan = prepare(opts)?;
    if let Some(i) = &plan.install {
        log::info!("ROUNDS install: {} ({})", i.dir.display(), i.source);
    }
    run_import(&plan)?;
    load(&plan.import_dir)
}
