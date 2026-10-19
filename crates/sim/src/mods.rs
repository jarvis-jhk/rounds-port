//! Built-in mods (docs/MODS.md): optional rule changes after popular
//! community mods of the original, reimplemented from their documented
//! behaviour. All off by default; with everything off the simulation is the
//! vanilla one, bit for bit (`tests/mods.rs`). The set is part of
//! [`crate::MatchSetup`], so every rollback peer runs the host's rules.

use crate::cards::HAND_SIZE;
use serde::{Deserialize, Serialize};

/// Credits and links of one built-in mod, for menus and docs.
#[derive(Clone, Copy, Debug)]
pub struct ModInfo {
    /// Stable id (settings files, logs).
    pub id: &'static str,
    /// Name of the original mod.
    pub name: &'static str,
    pub author: &'static str,
    pub url: &'static str,
    /// One line in our own words.
    pub about: &'static str,
}

pub const PICK_N_CARDS: ModInfo = ModInfo {
    id: "pick_n_cards",
    name: "Pick N Cards",
    author: "Pykess",
    url: "https://thunderstore.io/c/rounds/p/Pykess/Pick_N_Cards/",
    about: "Several picks per pick phase and a different hand size.",
};

/// Every built-in mod, in menu order.
pub const ALL: &[ModInfo] = &[PICK_N_CARDS];

/// The enabled mods of a match and their settings. `Default` = all off.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mods {
    /// [`PICK_N_CARDS`]: `None` = off.
    #[serde(default)]
    pub pick_n_cards: Option<PickNCards>,
}

/// Settings of [`PICK_N_CARDS`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PickNCards {
    /// Cards each picker takes per pick phase, one hand after another
    /// ([`PickNCards::MAX_PICKS`] at most).
    pub picks: u8,
    /// Cards dealt per hand ([`PickNCards::MAX_HAND`] at most).
    pub hand_size: u8,
}

impl PickNCards {
    pub const MAX_PICKS: u8 = 5;
    pub const MAX_HAND: u8 = 10;
}

impl Default for PickNCards {
    /// What players usually turn it on for: two picks, vanilla hand.
    fn default() -> Self {
        PickNCards {
            picks: 2,
            hand_size: HAND_SIZE as u8,
        }
    }
}

impl Mods {
    /// No mod is enabled (vanilla rules).
    pub fn is_vanilla(&self) -> bool {
        *self == Mods::default()
    }

    /// How many mods are on.
    pub fn enabled_count(&self) -> usize {
        self.pick_n_cards.is_some() as usize
    }

    /// Settings clamped to what the simulation supports.
    pub fn sanitized(self) -> Mods {
        Mods {
            pick_n_cards: self.pick_n_cards.map(|p| PickNCards {
                picks: p.picks.clamp(1, PickNCards::MAX_PICKS),
                hand_size: p.hand_size.clamp(1, PickNCards::MAX_HAND),
            }),
        }
    }

    /// Cards per hand.
    pub fn hand_size(&self) -> usize {
        self.pick_n_cards
            .map_or(HAND_SIZE, |p| p.hand_size as usize)
    }

    /// Picks per player per pick phase.
    pub fn picks(&self) -> u8 {
        self.pick_n_cards.map_or(1, |p| p.picks)
    }

    /// Mix the settings into the checksum (only called when not vanilla, so
    /// the vanilla checksum is unchanged).
    pub(crate) fn hash_into(&self, h: &mut crate::world::Hasher) {
        if let Some(p) = self.pick_n_cards {
            h.u32(0x504E_0000 | (p.picks as u32) << 8 | p.hand_size as u32);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_vanilla_and_old_setups_parse() {
        assert!(Mods::default().is_vanilla());
        assert_eq!(Mods::default().hand_size(), HAND_SIZE);
        assert_eq!(Mods::default().picks(), 1);
        // A setup from before the field existed reads as "all off".
        let m: Mods = serde_json::from_str("{}").unwrap();
        assert!(m.is_vanilla());
    }

    #[test]
    fn sanitized_clamps() {
        let m = Mods {
            pick_n_cards: Some(PickNCards {
                picks: 0,
                hand_size: 99,
            }),
        }
        .sanitized();
        assert_eq!(m.picks(), 1);
        assert_eq!(m.hand_size(), PickNCards::MAX_HAND as usize);
    }
}
