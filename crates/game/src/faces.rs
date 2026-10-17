//! Character creator items (ui.md 2.6): the catalog of eyes, mouths and
//! accessories, and drawing a [`Face`] on a player body.
//!
//! The catalog comes from the main scene's `CharacterCreatorItemLoader`
//! (`eyes`, `mouths`, `accessories`: references to `CharacterItem`
//! components) resolved through the imported prefabs that carry those
//! components (sprite, `offset`, `scale`, colour, `moveHealthBarUp`). Items
//! whose prefab is not in the import fall back to the imported sprites
//! named `E_*` / `M_*` / `A_*` with default offsets (docs/platform.md
//! "Faces"), so presets are selectable with any import.

use std::collections::HashMap;

use glam::{Affine2, Vec2};
use rounds_import::GameData;
use rounds_render::{Color, DrawList, Gpu, Layer, Renderer};

use crate::play::assets::{FaceItem, PlayAssets};
use crate::settings::Face;

/// One selectable item.
#[derive(Clone, Debug, PartialEq)]
pub struct FaceItemDef {
    pub name: String,
    /// Sprite id in the import (`None` = an empty item).
    pub sprite: Option<String>,
    /// Default offset in face units.
    pub offset: Vec2,
    pub scale: f32,
    pub color: Color,
    /// Raises the health bar (tall accessories).
    pub health_bar_up: f32,
}

/// Categories: 0 eyes, 1 mouths, 2 and 3 accessories (two detail slots).
pub const CATEGORIES: usize = 4;
pub const CATEGORY_KEYS: [(&str, &str); CATEGORIES] = [
    ("CATEGORY_EYES", "Eyes"),
    ("CATEGORY_MOUTHS", "Mouths"),
    ("CATEGORY_DETAIL1", "Detail"),
    ("CATEGORY_DETAIL2", "Detail 2"),
];

const ITEM_COLOR: Color = Color::rgb(0.1509, 0.1509, 0.1509);

#[derive(Clone, Debug, Default)]
pub struct FaceCatalog {
    pub eyes: Vec<FaceItemDef>,
    pub mouths: Vec<FaceItemDef>,
    pub accessories: Vec<FaceItemDef>,
    /// Items resolved from `CharacterItem` prefabs (the rest are fallbacks).
    pub from_prefabs: usize,
}

impl FaceCatalog {
    pub fn list(&self, category: usize) -> &[FaceItemDef] {
        match category {
            0 => &self.eyes,
            1 => &self.mouths,
            _ => &self.accessories,
        }
    }

    /// Builds the catalog from the import: `faces.json` (import revision
    /// 3: the loader lists, item id = list index as in the original), else
    /// the older prefab scan with sprite-name fallbacks.
    pub fn load(data: &GameData) -> FaceCatalog {
        let f = &data.faces;
        if !f.eyes.is_empty() && !f.mouths.is_empty() {
            let conv = |list: &[rounds_import::format::FaceItem]| -> Vec<FaceItemDef> {
                list.iter()
                    .map(|it| FaceItemDef {
                        // The prefab names are numbered copies
                        // ("P_E_Neutral 12"); the sprite name tells them apart.
                        name: it
                            .sprite
                            .as_ref()
                            .and_then(|s| data.sprites.get(s))
                            .map_or_else(|| it.name.clone(), |s| s.name.clone()),
                        sprite: it.sprite.clone(),
                        offset: Vec2::new(it.offset[0], it.offset[1]),
                        scale: it.scale,
                        color: Color::rgba(it.color[0], it.color[1], it.color[2], it.color[3]),
                        health_bar_up: it.move_health_bar_up,
                    })
                    .collect()
            };
            let cat = FaceCatalog {
                eyes: conv(&f.eyes),
                mouths: conv(&f.mouths),
                accessories: conv(&f.accessories),
                from_prefabs: f.eyes.len() + f.mouths.len() + f.accessories.len(),
            };
            log::info!(
                "faces: {} eyes, {} mouths, {} accessories (faces.json)",
                cat.eyes.len(),
                cat.mouths.len(),
                cat.accessories.len()
            );
            return cat;
        }
        let items = character_items(data);
        let lists = loader_lists(data);
        let mut cat = FaceCatalog::default();
        let sprite_names: Vec<(&str, &str)> = data
            .sprites
            .iter()
            .map(|(id, s)| (s.name.as_str(), id.as_str()))
            .collect();
        for (c, (list, prefix, default_offset)) in [
            (&lists.0, "E_", Vec2::new(0.0, 0.35)),
            (&lists.1, "M_", Vec2::new(0.0, -1.3)),
            (&lists.2, "A_", Vec2::new(0.0, 0.0)),
        ]
        .into_iter()
        .enumerate()
        {
            let mut out: Vec<FaceItemDef> = Vec::new();
            for r in list {
                if let Some(it) = items.get(r) {
                    out.push(it.clone());
                }
            }
            cat.from_prefabs += out.len();
            // Fallback: imported sprites by name (deduplicated against the
            // prefab items).
            let mut names: Vec<(&str, &str)> = sprite_names
                .iter()
                .copied()
                .filter(|(n, _)| n.starts_with(prefix))
                .collect();
            names.sort_by_key(|a| natural_key(a.0));
            if c == 0 {
                bring_first(&mut names, "E_1_Neutral");
            } else if c == 1 {
                bring_first(&mut names, "M_2_Neutral");
            }
            for (n, id) in names {
                if out.iter().any(|d| d.sprite.as_deref() == Some(id)) {
                    continue;
                }
                out.push(FaceItemDef {
                    name: n.to_owned(),
                    sprite: Some(id.to_owned()),
                    offset: default_offset,
                    scale: 1.0,
                    color: ITEM_COLOR,
                    health_bar_up: 0.0,
                });
            }
            if c == 2 {
                // Accessories start with "none" unless the loader's first
                // item is already an empty one.
                if out.first().is_none_or(|d| d.sprite.is_some()) {
                    out.insert(
                        0,
                        FaceItemDef {
                            name: "None".into(),
                            sprite: None,
                            offset: Vec2::ZERO,
                            scale: 1.0,
                            color: ITEM_COLOR,
                            health_bar_up: 0.0,
                        },
                    );
                }
            }
            match c {
                0 => cat.eyes = out,
                1 => cat.mouths = out,
                _ => cat.accessories = out,
            }
        }
        log::info!(
            "faces: {} eyes, {} mouths, {} accessories ({} from CharacterItem prefabs)",
            cat.eyes.len(),
            cat.mouths.len(),
            cat.accessories.len(),
            cat.from_prefabs
        );
        cat
    }

    /// The built-in presets (shown before the player edits them): a few
    /// different expressions so 12 players are told apart.
    pub fn default_presets(&self) -> Vec<Face> {
        let find = |list: &[FaceItemDef], names: &[&str]| -> u16 {
            names
                .iter()
                .find_map(|n| list.iter().position(|d| d.name == *n))
                .unwrap_or(0) as u16
        };
        let e = &self.eyes;
        let m = &self.mouths;
        let presets: [(&[&str], &[&str]); 10] = [
            (&["E_1_Neutral"], &["M_2_Neutral"]),
            (&["E_16_Happy"], &["M_1_Smile", "M_Smile_1"]),
            (&["E_15_MoreAngri", "E_10_grumpy"], &["M_2_Angry"]),
            (&["E_9_Sad"], &["M_2_Sad"]),
            (&["E_4_Smirk"], &["M_2_Smile2"]),
            (&["E_5_Hearts"], &["M_4_Kiss"]),
            (&["E_6_Rings"], &["M_Ooh_1"]),
            (&["E_7_Slanted"], &["M_4_Grimace"]),
            (&["E_22_Star"], &["M_2_Wide"]),
            (&["E_14_EyebrowRise"], &["M_4_Toung"]),
        ];
        presets
            .iter()
            .map(|(es, ms)| Face {
                eyes: find(e, es),
                mouth: find(m, ms),
                ..Face::default()
            })
            .collect()
    }
}

fn bring_first(names: &mut Vec<(&str, &str)>, name: &str) {
    if let Some(i) = names.iter().position(|(n, _)| *n == name) {
        let x = names.remove(i);
        names.insert(0, x);
    }
}

/// Sort key that orders `E_2` before `E_10`.
fn natural_key(s: &str) -> Vec<(u32, String)> {
    let mut out = Vec::new();
    let mut num = String::new();
    let mut txt = String::new();
    for ch in s.chars() {
        if ch.is_ascii_digit() {
            if !txt.is_empty() {
                out.push((u32::MAX, std::mem::take(&mut txt)));
            }
            num.push(ch);
        } else {
            if !num.is_empty() {
                out.push((num.parse().unwrap_or(0), String::new()));
                num.clear();
            }
            txt.push(ch.to_ascii_lowercase());
        }
    }
    if !num.is_empty() {
        out.push((num.parse().unwrap_or(0), String::new()));
    }
    if !txt.is_empty() {
        out.push((u32::MAX, txt));
    }
    out
}

/// `CharacterCreatorItemLoader` lists (component references) of the main
/// scene: eyes, mouths, accessories.
fn loader_lists(data: &GameData) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut out = (Vec::new(), Vec::new(), Vec::new());
    let Some(main) = &data.main else { return out };
    for o in &main.objects {
        for c in &o.components {
            if c.script.as_deref() != Some("CharacterCreatorItemLoader") {
                continue;
            }
            let refs = |k: &str| -> Vec<String> {
                c.fields[k]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|r| r["$ref"].as_str().map(str::to_owned))
                    .collect()
            };
            out = (refs("eyes"), refs("mouths"), refs("accessories"));
        }
    }
    out
}

/// Every `CharacterItem` component found in the imported prefabs, by
/// component id.
fn character_items(data: &GameData) -> HashMap<String, FaceItemDef> {
    let mut out = HashMap::new();
    let Ok(dir) = std::fs::read_dir(data.dir.join("prefabs")) else {
        return out;
    };
    for e in dir.flatten() {
        let Ok(text) = std::fs::read_to_string(e.path()) else {
            continue;
        };
        if !text.contains("\"CharacterItem\"") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        for o in v["objects"].as_array().into_iter().flatten() {
            for c in o["components"].as_array().into_iter().flatten() {
                if c["script"] != "CharacterItem" {
                    continue;
                }
                let Some(id) = c["id"].as_str() else { continue };
                let f = &c["fields"];
                let num = |v: &serde_json::Value, d: f32| v.as_f64().map_or(d, |x| x as f32);
                let sprite = f["sprite"]["$ref"]
                    .as_str()
                    .or_else(|| o["sprite"]["sprite"].as_str())
                    .map(str::to_owned);
                let color = o["sprite"]["color"]
                    .as_array()
                    .filter(|a| a.len() >= 3)
                    .map(|a| {
                        Color::rgba(
                            num(&a[0], 0.15),
                            num(&a[1], 0.15),
                            num(&a[2], 0.15),
                            a.get(3).map_or(1.0, |x| num(x, 1.0)),
                        )
                    })
                    .unwrap_or(ITEM_COLOR);
                out.insert(
                    id.to_owned(),
                    FaceItemDef {
                        name: o["name"].as_str().unwrap_or("?").to_owned(),
                        sprite,
                        offset: Vec2::new(num(&f["offset"]["x"], 0.0), num(&f["offset"]["y"], 0.0)),
                        scale: num(&f["scale"], 1.0),
                        color,
                        health_bar_up: num(&f["moveHealthBarUp"], 0.0),
                    },
                );
            }
        }
    }
    out
}

/// The catalog's sprites, resolved for drawing.
pub struct FaceSprites {
    cats: [Vec<Option<FaceItem>>; 3],
}

impl FaceSprites {
    pub fn new(
        gpu: &Gpu,
        renderer: &mut Renderer,
        assets: &mut PlayAssets,
        catalog: &FaceCatalog,
    ) -> FaceSprites {
        let mut resolve = |list: &[FaceItemDef]| -> Vec<Option<FaceItem>> {
            list.iter()
                .map(|d| {
                    let id = d.sprite.as_deref()?;
                    let sprite = assets.sprite_id(gpu, renderer, id)?;
                    Some(FaceItem {
                        sprite,
                        offset: d.offset,
                        scale: d.scale,
                        color: d.color,
                    })
                })
                .collect()
        };
        FaceSprites {
            cats: [
                resolve(&catalog.eyes),
                resolve(&catalog.mouths),
                resolve(&catalog.accessories),
            ],
        }
    }

    /// The drawable items of a face (offsets include the dragged ones).
    pub fn items(&self, face: &Face) -> Vec<FaceItem> {
        let mut out = Vec::new();
        for c in 0..CATEGORIES {
            let (id, off) = face.item(c);
            let list = &self.cats[c.min(2)];
            let item = list
                .get(id as usize)
                .or_else(|| if c < 2 { list.first() } else { None })
                .copied()
                .flatten();
            if let Some(mut it) = item {
                it.offset += Vec2::new(off[0], off[1]);
                out.push(it);
            }
        }
        out
    }
}

/// Draws a body and face on the screen layer: `center` in pixels, `radius`
/// of the body in pixels. `facing` mirrors the face (1 or -1).
#[allow(clippy::too_many_arguments)]
pub fn draw_face_screen(
    list: &mut DrawList,
    sort: i32,
    assets: &PlayAssets,
    items: &[FaceItem],
    center: Vec2,
    radius: f32,
    body: Color,
    facing: f32,
) {
    match assets.body {
        Some(b) => {
            let s = b.fill(
                Affine2::from_translation(center),
                Vec2::splat(radius * 2.0),
                body,
            );
            list.sprite(Layer::Screen, sort, s);
        }
        None => list.circle(Layer::Screen, sort, center, radius, body),
    }
    // The player body is 1 unit across in world space (radius 0.5) and the
    // face root is scaled 0.16 (player_view): one face unit = 0.32 radius.
    let unit = radius * 0.32;
    for it in items {
        let off = it.offset * 1.05;
        let fx = if off.x == 0.0 { 1.0 } else { facing };
        let t = Affine2::from_translation(center)
            * Affine2::from_scale(Vec2::new(unit, -unit))
            * Affine2::from_translation(Vec2::new(off.x * fx, off.y))
            * Affine2::from_scale(Vec2::new(it.scale * 1.05 * facing, it.scale * 1.05));
        list.sprite(Layer::Screen, sort + 1, it.sprite.sprite(t, it.color));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order() {
        let mut v = vec!["E_10_a", "E_2_b", "E_1_Neutral"];
        v.sort_by_key(|s| natural_key(s));
        assert_eq!(v, vec!["E_1_Neutral", "E_2_b", "E_10_a"]);
    }
}
