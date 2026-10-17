//! Online play over Warpzone (rounds-net): the shared session handle,
//! what the lobby sends (rules, faces) and how a `MatchStart` becomes the
//! sim's `MatchSetup`. The driver is in [`driver`]; the screens are in
//! `menu::online`.

pub mod driver;

use std::cell::RefCell;
use std::rc::Rc;

use rounds_import::GameData;
use rounds_net::{MatchStart, NetSession, TeamMode as NetTeams};
use rounds_sim::{Content, GameMode, MapId, MatchSetup, PlayerInput, SlotSetup, TeamMode, World};
use serde::{Deserialize, Serialize};

use crate::settings::{Face, MatchRules};

/// The GGRS session type of the game: sim inputs, sim worlds.
pub type Net = NetSession<PlayerInput, World>;
pub type NetRef = Rc<RefCell<Net>>;

/// Version string machines must share: the game version plus the import's
/// fingerprint (different game data would desync immediately).
pub fn game_version(data: &GameData) -> String {
    let fp: String = data
        .index
        .source_fingerprint
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(12)
        .collect();
    format!("{}+{}", env!("CARGO_PKG_VERSION"), fp)
}

/// The host's rules for the next match (`MatchSettings::game`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NetRules {
    pub rules: MatchRules,
    pub sandbox: bool,
}

impl NetRules {
    pub fn encode(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    pub fn decode(b: &[u8]) -> NetRules {
        serde_json::from_slice(b).unwrap_or_default()
    }
}

/// A face as `SlotRequest::extra` (8 numbers, ui.md 2.6 "replicated as
/// the 8 values").
/// Ids as u16, offsets as i16 thousandths: 24 bytes.
pub fn encode_face(f: &Face) -> Vec<u8> {
    let mut out = Vec::with_capacity(24);
    for c in 0..4 {
        out.extend_from_slice(&f.item(c).0.to_le_bytes());
    }
    for c in 0..4 {
        for v in f.item(c).1 {
            let q = (v * 1000.0).round().clamp(-32767.0, 32767.0) as i16;
            out.extend_from_slice(&q.to_le_bytes());
        }
    }
    out
}

pub fn decode_face(b: &[u8]) -> Face {
    let mut f = Face::default();
    if b.len() < 24 {
        return f;
    }
    let u = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
    let o = |i: usize| i16::from_le_bytes([b[i], b[i + 1]]) as f32 / 1000.0;
    for c in 0..4 {
        f.set_item(c, u(c * 2));
        f.set_offset(c, [o(8 + c * 4), o(10 + c * 4)]);
    }
    f
}

/// Map rotation for a pool: every map not excluded (all if that leaves
/// none).
pub fn map_rotation(content: &Content, rules: &MatchRules) -> Vec<MapId> {
    let v: Vec<MapId> = (0..content.maps.len() as u16)
        .filter(|i| !rules.maps.excluded.contains(i))
        .map(MapId)
        .collect();
    if v.len() == content.maps.len() {
        Vec::new()
    } else {
        v
    }
}

/// Skin id of a team member: the team's colour, `+ 12 * k` for the k-th
/// member (slightly different shades, see `PlayAssets::skin`).
pub fn team_skin(team_colour: u8, member: usize) -> u8 {
    team_colour % 12 + 12 * (member.min(19) as u8)
}

/// Builds the sim setup every machine derives from the host's start
/// message (identical on all of them).
pub fn setup_from_start(content: &Content, ms: &MatchStart) -> MatchSetup {
    let rules = NetRules::decode(&ms.settings.game);
    let teams = matches!(ms.settings.team_mode, NetTeams::Teams(_));
    let mut members = [0usize; 13];
    let slots = ms
        .players
        .iter()
        .map(|p| {
            if teams {
                let t = p.team as usize % 13;
                let k = members[t];
                members[t] += 1;
                SlotSetup {
                    team: p.team,
                    skin: team_skin(p.team, k),
                }
            } else {
                SlotSetup {
                    team: p.slot,
                    skin: p.colour,
                }
            }
        })
        .collect();
    MatchSetup {
        slots,
        mode: if rules.sandbox {
            GameMode::Sandbox
        } else {
            GameMode::ArmsRace
        },
        teams: if teams {
            TeamMode::Teams
        } else {
            TeamMode::FreeForAll
        },
        seed: ms.seed,
        points_to_win_round: rules.rules.points_to_win_round.max(1),
        rounds_to_win_game: rules.rules.rounds_to_win.max(1),
        pick_phase: rules.rules.pick_phase,
        map_rotation: map_rotation(content, &rules.rules),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_roundtrip() {
        let mut r = NetRules::default();
        r.rules.rounds_to_win = 2;
        r.rules.maps.excluded = vec![3, 4];
        assert_eq!(NetRules::decode(&r.encode()), r);
        assert_eq!(NetRules::decode(b"garbage"), NetRules::default());
        let f = Face {
            eyes: 3,
            mouth_offset: [0.5, -1.0],
            ..Face::default()
        };
        assert_eq!(decode_face(&encode_face(&f)), f);
        assert!(encode_face(&f).len() <= rounds_net::wire::MAX_SLOT_EXTRA);
    }
}
