//! Presentation assets of a match, all from the player's import: sprites by
//! name or id, player skins (colours), the default face, TextMeshPro fonts.

use std::collections::HashMap;
use std::rc::Rc;

use glam::{Affine2, Vec2};
use rounds_import::GameData;
use rounds_render::{
    Color, FontId, Gpu, Mask, Renderer, SdfFontDesc, SdfGlyph, Sprite, TextureId, UvRect,
};

use crate::map_scene::TextureCache;

/// A sprite from the import, ready to place: the quad in sprite-local
/// world units (pivot at the origin) and its texture rect.
#[derive(Clone, Copy, Debug)]
pub struct SpriteRef {
    pub texture: TextureId,
    pub uv: UvRect,
    /// Bottom-left of the (trimmed) quad relative to the pivot, units.
    pub min: Vec2,
    pub size: Vec2,
    /// Full sprite rect size in units (for layout).
    pub rect_size: Vec2,
    /// Bottom-left of the trimmed quad inside the full rect, units.
    pub trim: Vec2,
}

impl SpriteRef {
    /// A world sprite with `transform` (sprite pivot at its origin).
    pub fn sprite(&self, transform: Affine2, tint: Color) -> Sprite {
        Sprite {
            texture: self.texture,
            uv: self.uv,
            transform: transform * Affine2::from_translation(self.min),
            size: self.size,
            pivot: Vec2::ZERO,
            tint,
            ..Default::default()
        }
    }

    pub fn masked(&self, transform: Affine2, tint: Color, mask: Mask) -> Sprite {
        Sprite {
            mask,
            ..self.sprite(transform, tint)
        }
    }

    /// A sprite stretched to fill `size` (world units) centred at the
    /// transform's origin, ignoring the pivot (UI images).
    pub fn fill(&self, transform: Affine2, size: Vec2, tint: Color) -> Sprite {
        let k = size / self.rect_size.max(Vec2::splat(1e-6));
        Sprite {
            texture: self.texture,
            uv: self.uv,
            transform: transform
                * Affine2::from_scale(k)
                * Affine2::from_translation(-self.rect_size * 0.5 + self.trim),
            size: self.size,
            pivot: Vec2::ZERO,
            tint,
            ..Default::default()
        }
    }
}

/// Colours of one player skin (match-flow.md 2.2, player.md 18.2).
#[derive(Clone, Copy, Debug)]
pub struct Skin {
    pub color: Color,
    pub background: Color,
    pub win_text: Color,
    pub particle: Color,
    /// Two-tone body fill (skin particle start colours).
    pub fill_min: Color,
    pub fill_max: Color,
}

fn hsv(h: f32, s: f32, v: f32) -> Color {
    let h = (h.rem_euclid(360.0)) / 60.0;
    let c = v * s;
    let x = c * (1.0 - ((h % 2.0) - 1.0).abs());
    let (r, g, b) = match h as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    Color::rgb(r + m, g + m, b + m)
}

fn scale(c: Color, k: f32) -> Color {
    Color::rgb((c.r * k).min(1.0), (c.g * k).min(1.0), (c.b * k).min(1.0))
}

/// Fill shades of the four original skins (player.md 18.2 table; the
/// `Skin_Player*` particle prefabs are not in the import).
const FILL: [([f32; 3], [f32; 3]); 4] = [
    ([0.472, 0.197, 0.122], [0.726, 0.343, 0.236]),
    ([0.187, 0.258, 0.491], [0.281, 0.421, 0.736]),
    ([0.557, 0.199, 0.188], [0.631, 0.271, 0.277]),
    ([0.222, 0.368, 0.165], [0.322, 0.528, 0.272]),
];

/// Hues of the port's extra skins 4..11 (match-flow.md 2.2 GENERALISE).
const EXTRA_HUES: [f32; 8] = [45.0, 280.0, 180.0, 330.0, 95.0, 245.0, 200.0, 25.0];

fn json_color(v: &serde_json::Value) -> Option<Color> {
    let f = |k: &str| v[k].as_f64().map(|x| x as f32);
    Some(Color::rgba(
        f("r")?,
        f("g")?,
        f("b")?,
        f("a").unwrap_or(1.0),
    ))
}

/// The 12 skins: 0..3 from the import's skin bank (spec values as a
/// fallback), 4..11 from the generalisation recipe.
pub fn skins(data: &GameData) -> Vec<Skin> {
    let spec: [[[f32; 3]; 4]; 4] = [
        [
            [0.6415, 0.2895, 0.1725],
            [0.3491, 0.2393, 0.2124],
            [0.9137, 0.4996, 0.3569],
            [0.6038, 0.2620, 0.0997],
        ],
        [
            [0.1683, 0.3127, 0.5849],
            [0.2196, 0.2587, 0.3098],
            [0.3569, 0.6010, 0.9137],
            [0.0980, 0.3228, 0.6039],
        ],
        [
            [0.6314, 0.2706, 0.2742],
            [0.3490, 0.2145, 0.2118],
            [0.9137, 0.3569, 0.3569],
            [0.6039, 0.0980, 0.0980],
        ],
        [
            [0.2627, 0.5373, 0.3267],
            [0.2196, 0.3098, 0.2817],
            [0.0, 0.6887, 0.2693],
            [0.0814, 0.5566, 0.2785],
        ],
    ];
    let c3 = |a: [f32; 3]| Color::rgb(a[0], a[1], a[2]);
    // The bank lists the skins in id order; each entry references a skin.
    let bank: Vec<Option<&serde_json::Value>> = data
        .player
        .skins
        .values()
        .find(|v| v["$script"] == "PlayerSkinBank")
        .and_then(|b| b["skins"].as_array())
        .map(|a| {
            a.iter()
                .map(|e| {
                    e["currentPlayerSkin"]["$ref"]
                        .as_str()
                        .and_then(|id| data.player.skins.get(id))
                })
                .collect()
        })
        .unwrap_or_default();
    let mut out = Vec::new();
    for i in 0..4 {
        let from_bank = bank.get(i).copied().flatten();
        let get = |k: &str, d: [f32; 3]| {
            from_bank
                .and_then(|s| json_color(&s[k]))
                .unwrap_or(c3(d))
                .with_alpha(1.0)
        };
        out.push(Skin {
            color: get("color", spec[i][0]),
            background: get("backgroundColor", spec[i][1]),
            win_text: get("winText", spec[i][2]),
            particle: get("particleEffect", spec[i][3]),
            fill_min: c3(FILL[i].0),
            fill_max: c3(FILL[i].1),
        });
    }
    for h in EXTRA_HUES {
        let color = hsv(h, 0.64, 0.6);
        out.push(Skin {
            color,
            background: hsv(h, 0.34, 0.33),
            win_text: hsv(h, 0.61, 0.91),
            particle: hsv(h, 0.84, 0.6),
            fill_min: scale(color, 0.74),
            fill_max: scale(color, 1.16),
        });
    }
    out
}

/// One face item (`CharacterItem`) of the default face.
#[derive(Clone, Copy, Debug)]
pub struct FaceItem {
    pub sprite: SpriteRef,
    /// Item offset in item units (inside the 0.16 face root).
    pub offset: Vec2,
    pub scale: f32,
    pub color: Color,
}

/// Loaded TextMeshPro fonts (embedded Noto Sans when missing).
#[derive(Clone, Copy, Debug)]
pub struct Fonts {
    pub light: FontId,
    pub regular: FontId,
    pub bold: FontId,
    pub liberation: FontId,
    pub ultralight: FontId,
}

pub struct PlayAssets {
    pub data: Rc<GameData>,
    pub textures: TextureCache,
    sprites: HashMap<String, Option<SpriteRef>>,
    names: HashMap<String, String>,
    pub skins: Vec<Skin>,
    pub eyes: Option<FaceItem>,
    pub mouth: Option<FaceItem>,
    pub fonts: Fonts,
    /// Common shapes.
    pub circle: Option<SpriteRef>,
    pub body: Option<SpriteRef>,
    pub hollow: Option<SpriteRef>,
    pub grass: Option<SpriteRef>,
    pub triangle: Option<SpriteRef>,
    pub hollow_triangle: Option<SpriteRef>,
}

impl PlayAssets {
    pub fn new(gpu: &Gpu, renderer: &mut Renderer, data: Rc<GameData>) -> PlayAssets {
        let names = data
            .sprites
            .iter()
            .map(|(id, s)| (s.name.clone(), id.clone()))
            .collect();
        let mut a = PlayAssets {
            skins: skins(&data),
            data,
            textures: TextureCache::default(),
            sprites: HashMap::new(),
            names,
            eyes: None,
            mouth: None,
            fonts: Fonts {
                light: FontId::DEFAULT,
                regular: FontId::DEFAULT,
                bold: FontId::DEFAULT,
                liberation: FontId::DEFAULT,
                ultralight: FontId::DEFAULT,
            },
            circle: None,
            body: None,
            hollow: None,
            grass: None,
            triangle: None,
            hollow_triangle: None,
        };
        a.circle = a.sprite_named(gpu, renderer, "Circe");
        a.body = a.sprite_named(gpu, renderer, "Target_Circle1");
        a.hollow = a.sprite_named(gpu, renderer, "CircleHollow4");
        a.grass = a.sprite_named(gpu, renderer, "Grass6");
        a.triangle = a.sprite_named(gpu, renderer, "Triangle");
        a.hollow_triangle = a.sprite_named(gpu, renderer, "HollowTriangle");
        a.eyes = a.face_item(gpu, renderer, "E_1_Neutral", Vec2::new(0.0, 0.35));
        a.mouth = a.face_item(gpu, renderer, "M_2_Neutral", Vec2::new(0.0, -1.3));
        a.fonts = a.load_fonts(gpu, renderer);
        a
    }

    /// The default face item: the `CharacterItem` prefab that shows `name`
    /// (its offset/scale/colour), or the sprite with spec offsets.
    fn face_item(
        &mut self,
        gpu: &Gpu,
        renderer: &mut Renderer,
        name: &str,
        default_offset: Vec2,
    ) -> Option<FaceItem> {
        let sprite = self.sprite_named(gpu, renderer, name)?;
        Some(FaceItem {
            sprite,
            offset: default_offset,
            scale: 1.0,
            color: Color::rgb(0.1509, 0.1509, 0.1509),
        })
    }

    fn load_fonts(&mut self, gpu: &Gpu, renderer: &mut Renderer) -> Fonts {
        let data = self.data.clone();
        let mut by_name: HashMap<String, FontId> = HashMap::new();
        for f in &data.fonts {
            let Some(atlas_id) = f.atlases.first() else {
                continue;
            };
            let Some((tex, (w, h))) = self.textures.get(gpu, renderer, &data, atlas_id) else {
                continue;
            };
            // The typed tables (import revision 2): rects in PNG space,
            // whitespace rects of upgraded legacy fonts repaired, kerning.
            let Some(info) = &f.info else {
                continue;
            };
            let unicode_of: HashMap<u32, u32> =
                info.characters.iter().map(|(&c, &g)| (g, c)).collect();
            let by_index: HashMap<u32, &rounds_import::format::FontGlyph> =
                info.glyphs.iter().map(|g| (g.index, g)).collect();
            let glyphs: Vec<SdfGlyph> = info
                .characters
                .iter()
                .filter_map(|(&c, gi)| {
                    let g = by_index.get(gi)?;
                    if g.atlas != 0 {
                        return None; // only the first atlas is loaded
                    }
                    Some(SdfGlyph {
                        unicode: c,
                        rect: g.rect_png,
                        bearing_x: g.bearing_x,
                        bearing_y: g.bearing_y,
                        advance: g.advance,
                    })
                })
                .collect();
            if glyphs.len() < 20 {
                continue;
            }
            let kerning = info
                .kerning
                .iter()
                .filter_map(|k| {
                    let dx = k.first_adjust[2] + k.second_adjust[0];
                    (dx != 0.0).then_some((
                        *unicode_of.get(&k.first)?,
                        *unicode_of.get(&k.second)?,
                        dx,
                    ))
                })
                .collect();
            let _ = (w, h);
            let id = renderer.add_sdf_font(SdfFontDesc {
                atlas: tex,
                atlas_size: (info.atlas_width.max(1), info.atlas_height.max(1)),
                point_size: info.point_size.max(1.0),
                ascent: info.ascent,
                descent: info.descent,
                line_height: info.line_height,
                padding: info.padding,
                glyphs,
                kerning,
                dilate: 0.0,
            });
            by_name.insert(f.name.clone(), id);
        }
        let pick = |n: &str| by_name.get(n).copied().unwrap_or(FontId::DEFAULT);
        Fonts {
            light: pick("Gravity-Light SDF"),
            regular: pick("Gravity-Regular SDF"),
            bold: pick("Gravity-Bold SDF"),
            ultralight: by_name
                .get("Gravity-UltraLight SDF")
                .copied()
                .unwrap_or(pick("Gravity-Light SDF")),
            liberation: by_name
                .get("LiberationSans SDF")
                .copied()
                .unwrap_or(pick("Gravity-Bold SDF")),
        }
    }

    /// A sprite by its asset name (first match).
    pub fn sprite_named(
        &mut self,
        gpu: &Gpu,
        renderer: &mut Renderer,
        name: &str,
    ) -> Option<SpriteRef> {
        let id = self.names.get(name)?.clone();
        self.sprite_id(gpu, renderer, &id)
    }

    /// A sprite by import id.
    pub fn sprite_id(&mut self, gpu: &Gpu, renderer: &mut Renderer, id: &str) -> Option<SpriteRef> {
        if let Some(s) = self.sprites.get(id) {
            return *s;
        }
        let data = self.data.clone();
        let r = (|| {
            let sp = data.sprites.get(id)?;
            let (tex, (tw, th)) =
                self.textures
                    .get(gpu, renderer, &data, sp.texture.as_deref()?)?;
            let ppu = sp.pixels_per_unit.max(1e-3);
            let [tx, ty, tw_px, th_px] = sp.texture_rect;
            let rect_size = Vec2::new(sp.rect[2], sp.rect[3]) / ppu;
            let pivot = Vec2::from(sp.pivot);
            let off = Vec2::from(sp.texture_rect_offset) / ppu;
            Some(SpriteRef {
                texture: tex,
                uv: UvRect::from_pixels(tx, ty, tw_px, th_px, tw, th),
                min: off - pivot * rect_size,
                size: Vec2::new(tw_px, th_px) / ppu,
                rect_size,
                trim: off,
            })
        })();
        self.sprites.insert(id.to_owned(), r);
        r
    }

    /// Skin `i`: 0..11 the 12 colours; `c + 12 * k` is colour `c` in the
    /// k-th member shade of a team (match-flow.md 2.2 GENERALISE: one hue
    /// per team, value varied per member; `winText` stays the team's).
    pub fn skin(&self, i: usize) -> Skin {
        let n = self.skins.len().max(1);
        let base = self.skins[i % n];
        let variant = i / 12;
        if variant == 0 {
            return base;
        }
        const K: [f32; 6] = [1.0, 1.25, 0.78, 1.45, 0.64, 1.12];
        let k = K[variant % K.len()] * if variant >= K.len() { 0.92 } else { 1.0 };
        Skin {
            color: scale(base.color, k),
            background: scale(base.background, 0.5 + 0.5 * k),
            win_text: base.win_text,
            particle: scale(base.particle, k),
            fill_min: scale(base.fill_min, k),
            fill_max: scale(base.fill_max, k),
        }
    }
}
