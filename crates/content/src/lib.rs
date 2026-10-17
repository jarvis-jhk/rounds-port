//! `rounds-content`: converts imported game data ([`rounds_import::GameData`])
//! into simulation content ([`rounds_sim::ContentDefs`]).
//!
//! - maps: [`maps`] (static colliders, props, joints, ropes, smashers,
//!   destructibles, saws, moving platforms, spawn points, camera size);
//! - cards, spawnable objects, projectiles: [`cards`];
//! - player prefab defaults and the comparison with the sim's hard-coded
//!   tuning: [`player`];
//! - a debug rasteriser of the sim's collision state: [`raster`];
//! - deterministic bot inputs for headless smoke runs: [`bots`];
//! - SoundEvent names for the sim's cues and their validation: [`sounds`].
//!
//! The conversion is deterministic (plain f32 arithmetic, libm through
//! `rounds_sim::math`, ordered containers), so every peer derives identical
//! content from identical game files.

pub mod bots;
pub mod cards;
pub mod fields;
pub mod maps;
pub mod player;
pub mod raster;
pub mod report;
pub mod sounds;
pub mod xform;

use rounds_import::GameData;
use rounds_sim::content::{ContentDefs, Tuning};

pub use player::PlayerDefaults;
pub use report::{Report, TuningDiff};

/// The converted content plus everything the sim has no slot for yet.
#[derive(Clone, Debug)]
pub struct Built {
    pub defs: ContentDefs,
    /// Player prefab values (the sim currently hard-codes these; see
    /// [`Report::tuning`] for the comparison).
    pub player: PlayerDefaults,
    pub report: Report,
}

impl Built {
    /// Builds the sim's [`rounds_sim::Content`] (resolving prefab names);
    /// its resolution warnings are appended to the report.
    pub fn content(&mut self) -> rounds_sim::Content {
        let c = rounds_sim::Content::new(self.defs.clone());
        for w in &c.warnings {
            self.report.warn(format!("sim: {w}"));
        }
        c
    }
}

/// Converts everything. Fails (with the report) only on errors that make the
/// content unusable (no maps, a map without spawn points, no cards).
#[allow(clippy::result_large_err)] // built once at start; the report is the error
pub fn build(data: &GameData) -> Result<Built, Report> {
    let mut report = Report::default();
    let maps: Vec<_> = data
        .maps
        .iter()
        .map(|m| maps::convert_map(m, &mut report))
        .collect();
    if maps.is_empty() {
        report.error("no maps in the import");
    }
    let player = player::read(data, &mut report);
    let seeds: Vec<String> = player.bullet_id.iter().cloned().collect();
    let cards = cards::convert(data, &seeds, &mut report);
    if cards.cards.is_empty() {
        report.error("no cards in the import");
    }
    let mut tuning = Tuning::default();
    if let Some(m) = maps.first() {
        tuning.bounds_half = maps::bounds_half(m.camera_size);
    }
    if player.can_see_player_mask != 0 {
        tuning.line_of_sight_mask = player.can_see_player_mask;
    }
    let sound_events = sounds::event_names(data, &mut report);
    // Presentation data the sim does not use, summarised for content-check.
    report.count("card categories", data.categories.len());
    report.count("localisation locales", data.localization.locales.len());
    report.count(
        "cards with a localised name key",
        data.cards.iter().filter(|c| c.name_key.is_some()).count(),
    );
    if data.localization.locales.is_empty() {
        report.warn("no localisation tables in the import; card text = serialized English");
    }
    report
        .tuning
        .extend(player::compare(&player, data, &tuning));
    let mut projectiles = cards.projectiles;
    // Index 0 must be the player's default bullet.
    if let Some(name) = player.bullet.as_deref()
        && let Some(i) = projectiles.iter().position(|p| p.name == name)
    {
        let p = projectiles.remove(i);
        projectiles.insert(0, p);
    } else {
        report.warn("player's default bullet not found; using the built-in standard bullet");
        projectiles.insert(0, rounds_sim::content::ProjectileDef::default());
    }
    let defs = ContentDefs {
        maps,
        cards: cards.cards,
        pool: cards.pool,
        objects: cards.objects,
        projectiles,
        tuning,
        sound_events,
    };
    if report.errors.is_empty() {
        Ok(Built {
            defs,
            player,
            report,
        })
    } else {
        Err(report)
    }
}

/// The dev import dir: `$ROUNDS_IMPORT`, else `<workspace>/cache/import`.
pub fn dev_import_dir() -> std::path::PathBuf {
    if let Some(d) = std::env::var_os("ROUNDS_IMPORT") {
        return d.into();
    }
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../cache/import")
}
