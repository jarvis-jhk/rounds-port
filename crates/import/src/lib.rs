//! `rounds-import`: converts the original game's Unity data into
//! rounds-port's own versioned format (PNG textures, Ogg audio, JSON for
//! sprites, maps, cards, prefabs, fonts), and loads it back ([`GameData`]).
//! The format is documented in `docs/formats/import.md`.

pub mod anim;
pub mod animator;
pub mod assets;
pub mod fonts;
pub mod format;
pub mod import;
pub mod load;
pub mod particles;
pub mod scene;

pub use format::{FORMAT_REVISION, FORMAT_VERSION};
pub use import::{Options, Report, import};
pub use load::GameData;

/// Directory name of an import of the current format inside the user data dir.
pub fn versioned_dir_name() -> String {
    format!("import-v{FORMAT_VERSION}")
}
