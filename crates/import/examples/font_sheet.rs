//! Rasterises a test string with every imported TextMeshPro font from its
//! SDF atlas and typed glyph table (`Font::info`), one PNG per font:
//! `scratch/fonts-<name>.png`. Checks that every glyph rect lies inside
//! the atlas.
//!
//! `cargo run --release -p rounds-import --example font_sheet -- [IMPORT_DIR] [OUT_DIR]`

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use rounds_import::GameData;
use rounds_import::load::read_png;

const TEXT: &[&str] = &[
    "The quick brown fox jumps over",
    "the lazy dog. ROUNDS 0123456789",
    "AV To Wa: kerning, (spaces)  !?",
];

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = PathBuf::from(args.first().map_or("cache/import", String::as_str));
    let out = PathBuf::from(args.get(1).map_or("scratch", String::as_str));
    std::fs::create_dir_all(&out)?;
    let data = GameData::load(&dir)?;
    for font in &data.fonts {
        let info = font
            .info
            .as_ref()
            .with_context(|| format!("{}: no typed info (re-import)", font.name))?;
        let (aw, ah) = (info.atlas_width as f32, info.atlas_height as f32);
        for g in &info.glyphs {
            let [x, y, w, h] = g.rect;
            if x < 0.0 || y < 0.0 || x + w > aw || y + h > ah {
                bail!(
                    "{}: glyph {} rect {:?} outside the atlas",
                    font.name,
                    g.index,
                    g.rect
                );
            }
        }
        let atlas_id = font.atlases.first().context("no atlas")?;
        let (tw, th, px) = read_png(&data.texture_path(atlas_id).context("atlas png")?)?;
        // Alpha channel = SDF value (Alpha8 atlases import as white + alpha).
        let sdf = |x: f32, y: f32| -> f32 {
            let x = x.clamp(0.0, tw as f32 - 1.001);
            let y = y.clamp(0.0, th as f32 - 1.001);
            let (x0, y0) = (x.floor() as usize, y.floor() as usize);
            let (fx, fy) = (x - x0 as f32, y - y0 as f32);
            let a = |xx: usize, yy: usize| px[(yy * tw as usize + xx) * 4 + 3] as f32 / 255.0;
            let top = a(x0, y0) * (1.0 - fx) + a(x0 + 1, y0) * fx;
            let bot = a(x0, y0 + 1) * (1.0 - fx) + a(x0 + 1, y0 + 1) * fx;
            top * (1.0 - fy) + bot * fy
        };
        let size = 40.0;
        let s = size / info.point_size.max(1.0);
        let line = info.line_height * s;
        let (w, h) = (900usize, (line * TEXT.len() as f32 + 20.0) as usize);
        let mut img = vec![255u8; w * h * 4];
        let mut missing = Vec::new();
        for (li, text) in TEXT.iter().enumerate() {
            let baseline = 10.0 + info.ascent * s + li as f32 * line;
            let mut pen = 10.0;
            let mut prev: Option<u32> = None;
            for ch in text.chars() {
                let Some(g) = info.glyph_for(ch) else {
                    missing.push(ch);
                    continue;
                };
                if let Some(p) = prev {
                    pen += info.kerning_x(p, g.index) * s;
                }
                prev = Some(g.index);
                let [rx, ry, rw, rh] = g.rect_png;
                if rw > 0.0 && rh > 0.0 {
                    // Quad = glyph rect plus padding, placed by the bearings.
                    let pad = info.padding;
                    let x0 = pen + (g.bearing_x - pad) * s;
                    let y0 = baseline - (g.bearing_y + pad) * s;
                    let (qw, qh) = ((rw + 2.0 * pad) * s, (rh + 2.0 * pad) * s);
                    for py in y0.floor().max(0.0) as usize..((y0 + qh).ceil() as usize).min(h) {
                        for qx in x0.floor().max(0.0) as usize..((x0 + qw).ceil() as usize).min(w) {
                            let u = (qx as f32 + 0.5 - x0) / s;
                            let v = (py as f32 + 0.5 - y0) / s;
                            if u < 0.0 || v < 0.0 || u > rw + 2.0 * pad || v > rh + 2.0 * pad {
                                continue;
                            }
                            let d = sdf(rx - pad + u, ry - pad + v);
                            let cov = ((d - 0.5) / 0.08 + 0.5).clamp(0.0, 1.0);
                            let o = (py * w + qx) * 4;
                            for c in 0..3 {
                                let cur = img[o + c] as f32;
                                img[o + c] = (cur * (1.0 - cov)) as u8;
                            }
                        }
                    }
                }
                pen += g.advance * s;
            }
        }
        let name: String = font
            .name
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        let path = out.join(format!("fonts-{name}.png"));
        let f = std::io::BufWriter::new(std::fs::File::create(&path)?);
        let mut e = png::Encoder::new(f, w as u32, h as u32);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        e.write_header()?.write_image_data(&img)?;
        println!(
            "{}: {} glyphs, {} chars, {} kerning pairs, padding {} -> {}{}",
            font.name,
            info.glyphs.len(),
            info.characters.len(),
            info.kerning.len(),
            info.padding,
            path.display(),
            if missing.is_empty() {
                String::new()
            } else {
                format!(" (missing {missing:?})")
            }
        );
    }
    Ok(())
}
