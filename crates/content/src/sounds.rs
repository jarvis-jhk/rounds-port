//! Sound data for the sim: the SoundEvent id -> name table
//! (`ContentDefs::sound_events`, the cue names of effect prefabs' sound
//! references), and a check that every cue name the sim emits exists in the
//! import (the presentation resolves cues by name through
//! `rounds_import::format::SoundBank`).

use rounds_import::GameData;
use std::collections::BTreeMap;

use crate::report::Report;

/// Cue names the sim emits from code (`rounds_sim::events::cue`).
pub fn sim_cues() -> Vec<&'static str> {
    use rounds_sim::events::cue::*;
    let mut v = vec![
        JUMP,
        JUMP_BIG,
        LAND,
        LAND_BIG,
        STICK_WALL,
        STICK_WALL_BIG,
        BOUNCE,
        DIE,
        BLOCK_START,
        BLOCK_BLOCKED,
        BLOCK_RECHARGED,
        BLOCK_STATUS,
        CANT_SHOOT,
        RELOAD_COMPLETE,
        RELOAD_START,
        HEAL,
        DAMAGE_PASSIVE,
        DAMAGE_POISON,
        DAMAGE_LIFE_STEAL,
        DAMAGE_SCREEN_EDGE,
        SLOW_FREEZE,
        HOMING_FOUND,
        TARGET_BOUNCE,
        IMPACT_BULLET_ON_BULLET,
        IMPACT_BOUNCE,
        BOX_IMPACT,
        BOX_DESTRUCTION,
        SAW_DAMAGE,
        SMASHER_MOVE,
        SMASHER_IMPACT,
        TEXT_APPEAR,
        TEXT_DISAPPEAR,
        LEVEL_IN,
        LEVEL_OUT,
        BALL_APPEAR,
        BALL_DISAPPEAR,
        CARD_SWITCH,
        CARD_PICK,
        CARDS_APPEAR,
        WIN_ROUND,
    ];
    v.extend(SPAWN);
    v
}

/// SoundEvent id -> name from the import, plus report entries: counts, sim
/// cues missing from the data, events without playable clips.
pub fn event_names(data: &GameData, r: &mut Report) -> BTreeMap<String, String> {
    let bank = &data.sounds;
    if bank.events.is_empty() {
        r.warn(
            "no sound events in the import (re-import for sounds.json); effect sound cues are off",
        );
        return BTreeMap::new();
    }
    r.count("sound events", bank.events.len());
    r.count("sound containers", bank.containers.len());
    r.count("sound component references", bank.references.len());
    let names = bank.event_ids_by_name();
    let mut missing = 0;
    for c in sim_cues() {
        if !names.contains_key(c) {
            missing += 1;
            r.warn(format!("sim cue {c} is not a SoundEvent in the import"));
        }
    }
    r.count("sim cues missing in the import", missing);
    let silent = bank
        .events
        .values()
        .filter(|e| bank.clip_files(e).iter().all(|f| f.is_empty()))
        .count();
    r.count("sound events without clips", silent);
    bank.events
        .iter()
        .map(|(id, e)| (id.clone(), e.name.clone()))
        .collect()
}
