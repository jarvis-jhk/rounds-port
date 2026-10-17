//! Sound cue names for `SoundEvent` references in effect prefabs
//! (presentation only). Cue names are SoundEvent asset names; the adapter
//! supplies the id -> name table from the import
//! ([`crate::content::ContentDefs::sound_events`]), and the presentation
//! resolves a name to clips through the import's `sounds.json`
//! (`rounds_import::format::SoundBank::event_by_name`).
//!
//! Names are interned process-wide so events can keep carrying
//! `&'static str` cues: the set is bounded by the game data (a few hundred
//! names) no matter how often content is rebuilt.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, OnceLock};

fn names() -> &'static Mutex<BTreeSet<&'static str>> {
    static N: OnceLock<Mutex<BTreeSet<&'static str>>> = OnceLock::new();
    N.get_or_init(Default::default)
}

fn ids() -> &'static Mutex<BTreeMap<String, &'static str>> {
    static I: OnceLock<Mutex<BTreeMap<String, &'static str>>> = OnceLock::new();
    I.get_or_init(Default::default)
}

/// The `'static` copy of a cue name (allocated once per distinct name).
pub fn intern(name: &str) -> &'static str {
    let mut set = names().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = set.get(name) {
        return s;
    }
    let s: &'static str = Box::leak(name.to_owned().into_boxed_str());
    set.insert(s);
    s
}

/// Registers SoundEvent ids -> names (called by `Content::new`) and returns
/// the interned table.
pub fn register(table: &BTreeMap<String, String>) -> BTreeMap<String, &'static str> {
    let mut out = BTreeMap::new();
    let mut g = ids().lock().unwrap_or_else(|e| e.into_inner());
    for (id, name) in table {
        let n = intern(name);
        g.insert(id.clone(), n);
        out.insert(id.clone(), n);
    }
    out
}

/// Cue name of a SoundEvent id registered by any `Content` built so far.
/// Prefer [`crate::Content::sound_event_name`].
pub fn cue_for(id: &str) -> Option<&'static str> {
    ids()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(id)
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interning_and_registration() {
        let a = intern("SFX_Test_A_SE");
        let b = intern(&String::from("SFX_Test_A_SE"));
        assert!(std::ptr::eq(a, b));
        let t: BTreeMap<String, String> = [("f:1".to_string(), "SFX_Test_B_SE".to_string())].into();
        let m = register(&t);
        assert_eq!(m["f:1"], "SFX_Test_B_SE");
        assert_eq!(cue_for("f:1"), Some("SFX_Test_B_SE"));
        assert_eq!(cue_for("f:404"), None);
    }
}
