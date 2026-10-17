//! Debug renders of the sim's collision state on imported maps.
//!
//! `sim-sheet [--data DIR] [--map NAME|INDEX] [--frame N] [--players N]
//!            [--seed N] [--out FILE] [--tile WxH] [--cols N]`
//!
//! Runs an arms-race match without the initial card pick on each map (the
//! rotation restricted to that map), driven by [`rounds_content::bots`], and
//! draws the state `--frame` ticks (default 300) after the first battle
//! started. With `--map` it writes one image (`scratch/sim-<map>.png`),
//! otherwise a contact sheet of every map (`scratch/sim-sheet.png`).

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use rayon::prelude::*;
use rounds_content::bots::Bots;
use rounds_content::raster;
use rounds_import::GameData;
use rounds_sim::flow::Phase;
use rounds_sim::{Content, MapId, MatchSetup, World};

fn run(
    content: &Content,
    map: u16,
    players: usize,
    seed: u64,
    frame: u32,
    opt: (bool, bool),
) -> World {
    let (picks, abs) = opt;
    let mut s = MatchSetup::ffa(players, seed);
    s.pick_phase = picks;
    s.map_rotation = vec![MapId(map)];
    let mut w = World::new(content, &s);
    let mut bots = Bots::new(if abs { seed } else { seed ^ map as u64 }, players);
    let mut since_battle: Option<u32> = abs.then_some(0);
    for _ in 0..20_000 {
        if let Some(t) = since_battle.as_mut() {
            if *t >= frame {
                break;
            }
            *t += 1;
        } else if w.phase() == Phase::Battle {
            since_battle = Some(0);
        }
        let inputs = bots.inputs(&w);
        w.step(content, &inputs);
    }
    w
}

fn main() -> Result<()> {
    let mut data_dir = rounds_content::dev_import_dir();
    let mut map: Option<String> = None;
    let mut frame = 300u32;
    let mut players = 4usize;
    let mut seed = 1u64;
    let mut out: Option<PathBuf> = None;
    let mut tile = (480u32, 270u32);
    let mut cols = 7usize;
    let mut opt = (false, false);
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut val = || args.next().with_context(|| format!("{a} needs a value"));
        match a.as_str() {
            "--data" => data_dir = val()?.into(),
            "--map" => map = Some(val()?),
            "--frame" => frame = val()?.parse()?,
            "--players" => players = val()?.parse()?,
            "--seed" => seed = val()?.parse()?,
            "--out" => out = Some(val()?.into()),
            "--cols" => cols = val()?.parse()?,
            "--picks" => opt.0 = true,
            "--abs" => opt.1 = true,
            "--tile" => {
                let v = val()?;
                let (w, h) = v.split_once('x').context("--tile WxH")?;
                tile = (w.parse()?, h.parse()?);
            }
            other => bail!("unknown argument {other}"),
        }
    }
    let data = GameData::load(&data_dir).context("loading the import")?;
    let mut built = rounds_content::build(&data).map_err(|r| anyhow::anyhow!("{r}"))?;
    let content = built.content();
    let scratch = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scratch");
    std::fs::create_dir_all(&scratch)?;
    match map {
        Some(m) => {
            let idx = m
                .parse::<usize>()
                .ok()
                .filter(|i| *i < content.maps.len())
                .or_else(|| content.maps.iter().position(|d| d.name == m))
                .with_context(|| format!("no map {m}"))?;
            let w = run(&content, idx as u16, players, seed, frame, opt);
            let name = &content.maps[idx].name;
            let img = raster::render(
                &content,
                &w,
                1920,
                1080,
                &format!("{idx} {name} F{}", w.frame()),
            );
            let path = out.unwrap_or_else(|| scratch.join(format!("sim-{name}.png")));
            img.save_png(&path)?;
            println!("wrote {}", path.display());
        }
        None => {
            let tiles: Vec<_> = (0..content.maps.len())
                .into_par_iter()
                .map(|i| {
                    let w = run(&content, i as u16, players, seed, frame, opt);
                    raster::render(
                        &content,
                        &w,
                        tile.0,
                        tile.1,
                        &format!("{} {}", i + 2, content.maps[i].name),
                    )
                })
                .collect();
            let img = raster::sheet(&tiles, cols).context("empty sheet")?;
            let path = out.unwrap_or_else(|| scratch.join("sim-sheet.png"));
            img.save_png(&path)?;
            println!("wrote {} ({} maps)", path.display(), tiles.len());
        }
    }
    Ok(())
}
