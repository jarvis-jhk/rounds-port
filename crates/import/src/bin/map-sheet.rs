//! Dev tool: renders every imported map into one contact sheet PNG.
//!
//! Usage: `map-sheet [--data DIR] [--out FILE] [--cols N] [--tile WxH] [--colliders]`
//! (defaults: `cache/import`, `scratch/maps-sheet.png`, 7 columns, 480x270).
//!
//! Sprites are drawn as textured, tinted quads with their world transforms,
//! sorted like Unity's 2D renderer (sorting layer, order in layer, then
//! depth). Objects without a visible sprite but with colliders are outlined;
//! `--colliders` outlines every collider. Each tile shows the camera area
//! (orthographic half-height `Map.size`, 16:9) and the map's build index.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use rayon::prelude::*;
use rounds_import::GameData;
use rounds_import::format::{Map, Object, Shape};
use rounds_import::load::read_png;
use tiny_skia::{
    Color, FillRule, Paint, PathBuilder, Pixmap, PixmapPaint, Rect, Stroke, Transform,
};

/// Background colour of a tile.
const BACKGROUND: [u8; 3] = [14, 14, 17];

struct SpriteImage {
    pixmap: Pixmap,
    /// Pixel → sprite-local units.
    local: Transform,
    /// Size in units (for sliced/tiled draw modes).
    size_units: [f32; 2],
}

fn premultiplied(w: u32, h: u32, rgba: &[u8]) -> Option<Pixmap> {
    let mut p = Pixmap::new(w, h)?;
    for (d, s) in p
        .data_mut()
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(rgba.as_chunks::<4>().0)
    {
        let a = s[3] as u16;
        d[0] = ((s[0] as u16 * a + 127) / 255) as u8;
        d[1] = ((s[1] as u16 * a + 127) / 255) as u8;
        d[2] = ((s[2] as u16 * a + 127) / 255) as u8;
        d[3] = s[3];
    }
    Some(p)
}

fn load_sprites(data: &GameData, maps: &[&Map]) -> HashMap<String, SpriteImage> {
    let mut ids: Vec<&str> = maps
        .iter()
        .flat_map(|m| m.objects.iter())
        .filter_map(|o| o.sprite.as_ref()?.sprite.as_deref())
        .collect();
    ids.sort();
    ids.dedup();
    let mut tex_ids: Vec<&str> = ids
        .iter()
        .filter_map(|s| data.sprites.get(*s)?.texture.as_deref())
        .collect();
    tex_ids.sort();
    tex_ids.dedup();
    let textures: HashMap<&str, (u32, u32, Vec<u8>)> = tex_ids
        .par_iter()
        .filter_map(|t| Some((*t, read_png(&data.texture_path(t)?).ok()?)))
        .collect();
    ids.par_iter()
        .filter_map(|id| {
            let s = data.sprites.get(*id)?;
            let (tw, th, rgba) = textures.get(s.texture.as_deref()?)?;
            let [x, y, w, h] = s.texture_rect;
            let (x0, y0) = (x.round().max(0.0) as u32, y.round().max(0.0) as u32);
            let (w, h) = (
                (w.round() as u32).min(tw - x0).max(1),
                (h.round() as u32).min(th - y0).max(1),
            );
            let mut crop = vec![0u8; (w * h * 4) as usize];
            for row in 0..h {
                let src = (((y0 + row) * tw + x0) * 4) as usize;
                crop[(row * w * 4) as usize..][..(w * 4) as usize]
                    .copy_from_slice(&rgba[src..src + (w * 4) as usize]);
            }
            let ppu = s.pixels_per_unit;
            let left = (s.texture_rect_offset[0] - s.pivot[0] * s.rect[2]) / ppu;
            let bottom = (s.texture_rect_offset[1] - s.pivot[1] * s.rect[3]) / ppu;
            Some((
                id.to_string(),
                SpriteImage {
                    pixmap: premultiplied(w, h, &crop)?,
                    local: Transform::from_row(
                        1.0 / ppu,
                        0.0,
                        0.0,
                        -1.0 / ppu,
                        left,
                        bottom + h as f32 / ppu,
                    ),
                    size_units: [s.rect[2] / ppu, s.rect[3] / ppu],
                },
            ))
        })
        .collect()
}

fn tinted(src: &Pixmap, c: [f32; 4]) -> Pixmap {
    let mut p = src.clone();
    let m = [c[0] * c[3], c[1] * c[3], c[2] * c[3], c[3]].map(|v| v.clamp(0.0, 1.0));
    for px in p.data_mut().as_chunks_mut::<4>().0 {
        for k in 0..4 {
            px[k] = (px[k] as f32 * m[k]).round() as u8;
        }
    }
    p
}

fn world_tf(o: &Object) -> Transform {
    let [a, b, c, d, tx, ty] = o.world.matrix;
    Transform::from_row(a, b, c, d, tx, ty)
}

fn outline(px: &mut Pixmap, o: &Object, view: Transform, color: Color) {
    let tf = world_tf(o).post_concat(view);
    let mut paint = Paint::default();
    paint.set_color(color);
    paint.anti_alias = true;
    let stroke = Stroke {
        width: 1.0,
        ..Default::default()
    };
    for c in &o.colliders {
        let mut pb = PathBuilder::new();
        let [ox, oy] = c.offset;
        match &c.shape {
            Shape::Box { size, .. } | Shape::Capsule { size, .. } => {
                if let Some(r) =
                    Rect::from_xywh(ox - size[0] / 2.0, oy - size[1] / 2.0, size[0], size[1])
                {
                    pb.push_rect(r);
                }
            }
            Shape::Circle { radius } => pb.push_circle(ox, oy, *radius),
            Shape::Polygon { paths } => {
                for p in paths {
                    for (i, q) in p.iter().enumerate() {
                        if i == 0 {
                            pb.move_to(q[0] + ox, q[1] + oy)
                        } else {
                            pb.line_to(q[0] + ox, q[1] + oy)
                        }
                    }
                    pb.close();
                }
            }
            Shape::Edge { points, .. } => {
                for (i, q) in points.iter().enumerate() {
                    if i == 0 {
                        pb.move_to(q[0] + ox, q[1] + oy)
                    } else {
                        pb.line_to(q[0] + ox, q[1] + oy)
                    }
                }
            }
        }
        if let Some(path) = pb.finish().and_then(|p| p.transform(tf)) {
            px.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
        }
    }
}

/// 3x5 pixel digits for the build index label.
const DIGITS: [u16; 10] = [
    0b111_101_101_101_111,
    0b010_110_010_010_111,
    0b111_001_111_100_111,
    0b111_001_111_001_111,
    0b101_101_111_001_001,
    0b111_100_111_001_111,
    0b111_100_111_101_111,
    0b111_001_010_010_010,
    0b111_101_111_101_111,
    0b111_101_111_001_111,
];

fn label(px: &mut Pixmap, n: usize, scale: f32) {
    let mut paint = Paint::default();
    paint.set_color_rgba8(255, 255, 255, 220);
    for (i, ch) in n.to_string().bytes().enumerate() {
        let bits = DIGITS[(ch - b'0') as usize];
        for row in 0..5 {
            for col in 0..3 {
                if bits >> (14 - (row * 3 + col)) & 1 == 1 {
                    let x = 4.0 + (i as f32 * 4.0 + col as f32) * scale;
                    let y = 4.0 + row as f32 * scale;
                    px.fill_rect(
                        Rect::from_xywh(x, y, scale, scale).unwrap(),
                        &paint,
                        Transform::identity(),
                        None,
                    );
                }
            }
        }
    }
}

fn render_map(
    map: &Map,
    sprites: &HashMap<String, SpriteImage>,
    w: u32,
    h: u32,
    colliders: bool,
) -> Pixmap {
    let mut px = Pixmap::new(w, h).unwrap();
    px.fill(Color::from_rgba8(
        BACKGROUND[0],
        BACKGROUND[1],
        BACKGROUND[2],
        255,
    ));
    let half_h = map
        .map
        .as_ref()
        .and_then(|m| m["size"].as_f64())
        .filter(|s| *s > 0.0)
        .unwrap_or(20.0) as f32;
    let s = h as f32 / (2.0 * half_h);
    let view = Transform::from_row(s, 0.0, 0.0, -s, w as f32 / 2.0, h as f32 / 2.0);
    let mut draws: Vec<(usize, &Object)> = map
        .objects
        .iter()
        .filter(|o| o.active_in_hierarchy)
        .enumerate()
        .filter(|(_, o)| {
            o.sprite
                .as_ref()
                .is_some_and(|r| r.enabled && r.color[3] > 0.0)
        })
        .collect();
    draws.sort_by(|(ia, a), (ib, b)| {
        let (ra, rb) = (a.sprite.as_ref().unwrap(), b.sprite.as_ref().unwrap());
        (ra.sorting_layer_index, ra.sorting_order)
            .cmp(&(rb.sorting_layer_index, rb.sorting_order))
            // Farther (larger z) first.
            .then(b.world.trs.position[2].total_cmp(&a.world.trs.position[2]))
            .then(ia.cmp(ib))
    });
    let mut tints: HashMap<(String, [u32; 4]), Pixmap> = HashMap::new();
    for (_, o) in &draws {
        let r = o.sprite.as_ref().unwrap();
        let Some(img) = r.sprite.as_ref().and_then(|id| sprites.get(id)) else {
            continue;
        };
        let key = (r.sprite.clone().unwrap(), r.color.map(f32::to_bits));
        let tex = tints
            .entry(key)
            .or_insert_with(|| tinted(&img.pixmap, r.color));
        let mut local = img.local;
        if r.draw_mode != 0 && img.size_units[0] > 0.0 && img.size_units[1] > 0.0 {
            local = local.post_scale(r.size[0] / img.size_units[0], r.size[1] / img.size_units[1]);
        }
        local = local.post_scale(
            if r.flip_x { -1.0 } else { 1.0 },
            if r.flip_y { -1.0 } else { 1.0 },
        );
        let tf = local.post_concat(world_tf(o)).post_concat(view);
        let paint = PixmapPaint {
            quality: tiny_skia::FilterQuality::Bilinear,
            ..Default::default()
        };
        px.draw_pixmap(0, 0, tex.as_ref(), &paint, tf, None);
    }
    // Whether an object or any descendant shows a sprite (children follow parents).
    let mut visible: Vec<bool> = map
        .objects
        .iter()
        .map(|o| {
            o.active_in_hierarchy
                && o.sprite
                    .as_ref()
                    .is_some_and(|r| r.enabled && r.sprite.is_some())
        })
        .collect();
    for i in (0..map.objects.len()).rev() {
        if visible[i]
            && let Some(p) = map.objects[i].parent
        {
            visible[p] = true;
        }
    }
    for (i, o) in map.objects.iter().enumerate() {
        if o.active_in_hierarchy && !o.colliders.is_empty() && (colliders || !visible[i]) {
            outline(&mut px, o, view, Color::from_rgba8(80, 220, 120, 200));
        }
    }
    let mut paint = Paint::default();
    paint.set_color_rgba8(255, 80, 80, 255);
    for sp in &map.spawn_points {
        let (x, y) = (
            w as f32 / 2.0 + sp.position[0] * s,
            h as f32 / 2.0 - sp.position[1] * s,
        );
        if let Some(c) = PathBuilder::from_circle(x, y, 2.5) {
            px.fill_path(&c, &paint, FillRule::Winding, Transform::identity(), None);
        }
    }
    label(&mut px, map.build_index, 2.0);
    px
}

fn main() -> Result<()> {
    let root = std::env::var_os("ROUNDS_ROOT")
        .map(PathBuf::from)
        .unwrap_or_default();
    let mut data_dir = root.join("cache/import");
    let mut out = root.join("scratch/maps-sheet.png");
    let (mut cols, mut tw, mut th, mut colliders) = (7u32, 480u32, 270u32, false);
    let mut only: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--data" => data_dir = args.next().context("--data DIR")?.into(),
            "--out" => out = args.next().context("--out FILE")?.into(),
            "--cols" => cols = args.next().context("--cols N")?.parse()?,
            "--tile" => {
                let t = args.next().context("--tile WxH")?;
                let (a, b) = t.split_once('x').context("--tile WxH")?;
                (tw, th) = (a.parse()?, b.parse()?);
            }
            "--colliders" => colliders = true,
            "--map" => only = Some(args.next().context("--map NAME")?),
            _ => anyhow::bail!("unknown argument {a}"),
        }
    }
    let data = GameData::load(&data_dir)?;
    let maps: Vec<&Map> = data
        .maps
        .iter()
        .filter(|m| {
            only.as_ref()
                .is_none_or(|n| &m.name == n || m.build_index.to_string() == *n)
        })
        .collect();
    let sprites = load_sprites(&data, &maps);
    let tiles: Vec<Pixmap> = maps
        .par_iter()
        .map(|m| render_map(m, &sprites, tw, th, colliders))
        .collect();
    let cols = cols.min(tiles.len().max(1) as u32);
    let rows = (tiles.len() as u32).div_ceil(cols);
    let gap = 4;
    let mut sheet =
        Pixmap::new(cols * (tw + gap) + gap, rows * (th + gap) + gap).context("sheet size")?;
    sheet.fill(Color::from_rgba8(60, 60, 64, 255));
    for (i, t) in tiles.iter().enumerate() {
        let (c, r) = (i as u32 % cols, i as u32 / cols);
        sheet.draw_pixmap(
            (gap + c * (tw + gap)) as i32,
            (gap + r * (th + gap)) as i32,
            t.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            None,
        );
    }
    if let Some(p) = out.parent() {
        std::fs::create_dir_all(p)?;
    }
    sheet.save_png(&out)?;
    println!(
        "{} maps -> {} ({}x{})",
        tiles.len(),
        out.display(),
        sheet.width(),
        sheet.height()
    );
    Ok(())
}
