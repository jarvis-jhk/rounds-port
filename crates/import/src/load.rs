//! Loading an import dir back into memory.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::de::DeserializeOwned;

use crate::format::{self, FORMAT_VERSION, ObjId};

/// Everything the importer wrote, except bulk data (PNGs, Oggs, prefabs),
/// which is loaded on demand.
#[derive(Clone, Debug)]
pub struct GameData {
    /// The import dir.
    pub dir: PathBuf,
    /// `index.json`.
    pub index: format::Index,
    /// `textures.json`.
    pub textures: BTreeMap<ObjId, format::Texture>,
    /// `sprites.json`.
    pub sprites: BTreeMap<ObjId, format::Sprite>,
    /// `audio.json`.
    pub audio: BTreeMap<ObjId, format::AudioClip>,
    /// `cards.json`.
    pub cards: Vec<format::Card>,
    /// `fonts.json`.
    pub fonts: Vec<format::Font>,
    /// `player.json`.
    pub player: format::Player,
    /// All maps in build order.
    pub maps: Vec<format::Map>,
    /// `main.json` (the main scene), if the import has it.
    pub main: Option<format::Scene>,
    /// `materials.json` (empty if the import predates it).
    pub materials: BTreeMap<ObjId, format::Material>,
    /// `post_profiles.json` (empty if the import predates it).
    pub post_profiles: BTreeMap<ObjId, format::PostProfile>,
    /// `sounds.json` (empty before revision 1).
    pub sounds: format::SoundBank,
    /// `categories.json`: `CardCategory` id -> name (empty before revision 1).
    pub categories: BTreeMap<ObjId, String>,
    /// `localization.json` (empty before revision 1 or without bundles).
    pub localization: format::Localization,
    /// `animations.json` (empty before revision 1).
    pub animations: format::Animations,
    /// `faces.json`: the character creator catalog (empty before revision 3).
    pub faces: format::Faces,
}

/// Files added to format version 1 after its first release; an import
/// without them is re-imported (see [`crate::import`]).
pub const LATER_FILES: &[&str] = &[
    "main.json",
    "materials.json",
    "post_profiles.json",
    "sounds.json",
    "categories.json",
    "localization.json",
    "animations.json",
    "faces.json",
];

fn read<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let s = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_slice(&s).with_context(|| format!("parsing {}", path.display()))
}

impl GameData {
    /// Loads an import dir; fails on a format version mismatch.
    pub fn load(dir: impl AsRef<Path>) -> Result<GameData> {
        let dir = dir.as_ref().to_path_buf();
        let index: format::Index = read(&dir.join("index.json"))?;
        if index.format_version != FORMAT_VERSION {
            bail!(
                "{}: import format {} but this build reads {FORMAT_VERSION}; re-import",
                dir.display(),
                index.format_version
            );
        }
        let maps = index
            .scenes
            .iter()
            .filter_map(|s| s.map_file.as_ref())
            .filter(|f| dir.join(f).is_file())
            .map(|f| read(&dir.join(f)))
            .collect::<Result<_>>()?;
        let optional = |name: &str| dir.join(name).is_file().then(|| dir.join(name));
        Ok(GameData {
            main: optional("main.json").map(|p| read(&p)).transpose()?,
            materials: optional("materials.json")
                .map(|p| read(&p))
                .transpose()?
                .unwrap_or_default(),
            post_profiles: optional("post_profiles.json")
                .map(|p| read(&p))
                .transpose()?
                .unwrap_or_default(),
            sounds: optional("sounds.json")
                .map(|p| read(&p))
                .transpose()?
                .unwrap_or_default(),
            categories: optional("categories.json")
                .map(|p| read(&p))
                .transpose()?
                .unwrap_or_default(),
            localization: optional("localization.json")
                .map(|p| read(&p))
                .transpose()?
                .unwrap_or_default(),
            animations: optional("animations.json")
                .map(|p| read(&p))
                .transpose()?
                .unwrap_or_default(),
            faces: optional("faces.json")
                .map(|p| read(&p))
                .transpose()?
                .unwrap_or_default(),
            textures: read(&dir.join("textures.json"))?,
            sprites: read(&dir.join("sprites.json"))?,
            audio: read(&dir.join("audio.json"))?,
            cards: read(&dir.join("cards.json"))?,
            fonts: read(&dir.join("fonts.json"))?,
            player: read(&dir.join("player.json"))?,
            maps,
            index,
            dir,
        })
    }

    /// Loads a prefab (cards, spawned objects, player) by GameObject ID.
    pub fn prefab(&self, id: &str) -> Result<format::Prefab> {
        read(&self.dir.join(format::prefab_file(id)))
    }

    /// Absolute path of a texture's PNG.
    pub fn texture_path(&self, id: &str) -> Option<PathBuf> {
        self.textures.get(id).map(|t| self.dir.join(&t.png))
    }

    /// Absolute path of an audio clip's Ogg file.
    pub fn audio_path(&self, id: &str) -> Option<PathBuf> {
        self.audio.get(id).map(|a| self.dir.join(&a.file))
    }

    /// Card text in `locale` (falls back to en-US, then to the card's
    /// serialized English text): `(name, description)`.
    pub fn card_text<'a>(&'a self, card: &'a format::Card, locale: &str) -> (&'a str, &'a str) {
        let t = |key: &Option<String>| {
            key.as_deref()
                .and_then(|k| self.localization.text(locale, "StringTableCards", k))
        };
        (
            t(&card.name_key).unwrap_or(&card.name),
            t(&card.description_key).unwrap_or(&card.description),
        )
    }

    /// Absolute paths of the Ogg files of a sound cue (a SoundEvent name),
    /// per container layer; empty if unknown.
    pub fn sound_cue_files(&self, cue: &str) -> Vec<Vec<PathBuf>> {
        let Some((_, e)) = self.sounds.event_by_name(cue) else {
            return vec![];
        };
        self.sounds
            .clip_files(e)
            .into_iter()
            .map(|files| files.iter().map(|f| self.dir.join(f)).collect())
            .collect()
    }

    /// Finds a map by scene name.
    pub fn map(&self, name: &str) -> Option<&format::Map> {
        self.maps.iter().find(|m| m.name == name)
    }
}

/// Decodes a PNG written by the importer into `(width, height, rgba8)`.
pub fn read_png(path: &Path) -> Result<(u32, u32, Vec<u8>)> {
    let d = png::Decoder::new(std::io::BufReader::new(fs::File::open(path)?));
    let mut r = d.read_info()?;
    let mut buf = vec![0; r.output_buffer_size().context("png size")?];
    let info = r.next_frame(&mut buf)?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        bail!("{}: expected RGBA8", path.display());
    }
    buf.truncate(info.buffer_size());
    Ok((info.width, info.height, buf))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Loads the dev import (`cache/import`) when it exists; skipped otherwise
    /// (the repo holds no game data).
    #[test]
    fn loads_dev_import_if_present() {
        let Some(root) = std::env::var_os("ROUNDS_ROOT") else {
            return;
        };
        let dir = Path::new(&root).join("cache/import");
        if !dir.join("index.json").is_file() {
            return;
        }
        let d = GameData::load(&dir).unwrap();
        assert_eq!(d.index.format_version, FORMAT_VERSION);
        if d.index.format_revision >= 1 {
            // Sound cues resolve to files, Transform ids are exported, card
            // text resolves.
            let files = d.sound_cue_files("SFX_Character_Jump_Ground_SE");
            assert!(files.iter().flatten().any(|f| f.is_file()), "{files:?}");
            assert!(!d.categories.is_empty());
            let barrage = d.cards.iter().find(|c| c.name_key.is_some()).unwrap();
            assert!(!d.card_text(barrage, "de").0.is_empty());
            for (_, e) in d.sounds.events.iter().take(20) {
                for l in &e.layers {
                    assert!(d.sounds.containers.contains_key(&l.container), "{}", e.name);
                }
            }
        }
        if d.index.format_revision >= 2 {
            // Every font has a typed table with in-atlas rects and a space.
            for f in &d.fonts {
                let i = f.info.as_ref().unwrap();
                assert!(
                    i.glyph_for(' ').is_some_and(|g| g.advance > 0.0),
                    "{}",
                    f.name
                );
                for g in &i.glyphs {
                    let [x, y, w, h] = g.rect;
                    assert!(x >= 0.0 && y >= 0.0, "{} {:?}", f.name, g.rect);
                    assert!(x + w <= i.atlas_width as f32 && y + h <= i.atlas_height as f32);
                }
            }
            // Skin prefabs carry the body-fill particle system.
            assert_eq!(d.player.skin_prefabs.len(), 4);
            for go in d.player.skin_prefabs.values() {
                let p = d.prefab(go).unwrap();
                assert!(
                    p.objects.iter().any(|o| o.particles.is_some()),
                    "{}",
                    p.name
                );
            }
            let main = d.main.as_ref().unwrap();
            assert!(main.objects.iter().any(|o| o.particles.is_some()));
        }
        if d.index.format_revision >= 3 {
            // The catalog lists resolve to exported prefabs with the item.
            let f = &d.faces;
            assert!(!f.eyes.is_empty() && !f.mouths.is_empty() && !f.accessories.is_empty());
            for it in f.eyes.iter().chain(&f.mouths).chain(&f.accessories) {
                assert!(
                    it.sprite
                        .as_ref()
                        .is_some_and(|s| d.sprites.contains_key(s))
                );
                let p = d.prefab(&it.prefab).unwrap();
                assert!(p.objects[0].components.iter().any(|c| c.id == it.id));
            }
            assert!(f.eyes.iter().all(|i| i.item_type == 0));
            assert!(f.mouths.iter().all(|i| i.item_type == 1));
            assert!(f.default_eyes.is_some() && f.default_mouth.is_some());
            assert_eq!(f.presets.len(), 10);
        }
        assert!(!d.maps.is_empty() && !d.cards.is_empty());
        for c in &d.cards {
            assert_eq!(d.prefab(&c.id).unwrap().id, c.id);
        }
        for m in &d.maps {
            for o in &m.objects {
                if let Some(s) = o.sprite.as_ref().and_then(|s| s.sprite.as_ref()) {
                    assert!(d.sprites.contains_key(s), "{}: sprite {s}", m.name);
                }
            }
        }
    }

    #[test]
    fn missing_dir_is_an_error() {
        assert!(GameData::load("/nonexistent/import").is_err());
    }
}
