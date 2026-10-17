//! Builds sim content from an import dir and prints the conversion report:
//! counts, unsupported and ignored components, effect scripts handed to the
//! sim, tuning comparison, warnings. Also cross-checks every map object's
//! imported world matrix against the composition of its local transforms.
//!
//! Usage: `content-check [--data DIR] [--json FILE] [--quiet]`
//! (`--json` writes the converted `ContentDefs`; `--quiet` hides warnings
//! that repeat per object, printing only their count).

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use rounds_content::xform::{affine, affine_diff, compose_world};
use rounds_import::GameData;

fn main() -> Result<()> {
    let mut data_dir = rounds_content::dev_import_dir();
    let mut json: Option<PathBuf> = None;
    let mut quiet = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--data" => data_dir = args.next().context("--data DIR")?.into(),
            "--json" => json = Some(args.next().context("--json FILE")?.into()),
            "--quiet" => quiet = true,
            "-h" | "--help" => {
                println!("content-check [--data DIR] [--json FILE] [--quiet]");
                return Ok(());
            }
            other => bail!("unknown argument {other}"),
        }
    }
    let data = GameData::load(&data_dir)
        .with_context(|| format!("loading {} (run rounds-import first)", data_dir.display()))?;
    let t0 = std::time::Instant::now();
    let mut built = match rounds_content::build(&data) {
        Ok(b) => b,
        Err(report) => {
            println!("{report}");
            bail!(
                "content conversion failed with {} errors",
                report.errors.len()
            );
        }
    };
    let build_ms = t0.elapsed().as_secs_f64() * 1000.0;
    // Transform cross-check.
    let mut worst = (0.0f32, String::new());
    let mut checked = 0usize;
    for m in &data.maps {
        let composed = compose_world(&m.objects);
        for (o, w) in m.objects.iter().zip(&composed) {
            let d = affine_diff(&affine(o.world.matrix), &w.to_affine());
            checked += 1;
            if d > worst.0 {
                worst = (d, format!("{} {}", m.name, o.name));
            }
        }
    }
    let content = built.content();
    if quiet {
        let n = built.report.warnings.len();
        let mut kinds = std::collections::BTreeMap::<String, usize>::new();
        for w in &built.report.warnings {
            let k = w
                .rsplit_once(": ")
                .map(|(_, k)| k.to_string())
                .unwrap_or_else(|| w.clone());
            *kinds.entry(k).or_default() += 1;
        }
        built.report.warnings = kinds
            .into_iter()
            .map(|(k, c)| format!("{c:5} x {k}"))
            .collect();
        println!("({n} warnings grouped by message)");
    }
    println!("{}", built.report);
    println!(
        "transforms: {checked} map objects, max |imported - composed| = {:.2e} ({})",
        worst.0, worst.1
    );
    println!(
        "sim content: {} maps, {} cards (pool {}), {} objects, {} projectiles; built in {build_ms:.0} ms",
        content.maps.len(),
        content.cards.len(),
        content.pool.len(),
        content.objects.len(),
        content.projectiles.len()
    );
    if let Some(p) = json {
        std::fs::write(&p, serde_json::to_vec_pretty(&built.defs)?)?;
        println!("wrote {}", p.display());
    }
    Ok(())
}
