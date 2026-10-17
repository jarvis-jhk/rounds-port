//! Host-side slot assignment: deterministic, pure, unit-tested.
//!
//! Order: machines by id (the host, 0, first), each machine's slots in its own
//! order, until [`MAX_SLOTS`]. Slots that do not fit are left out of the match
//! (the lobby shows which). Colours: a machine's request is honoured if still
//! free, else the lowest free colour. Teams: free-for-all gives every slot its
//! own team; with `n` teams a valid request is honoured, otherwise the slot
//! joins the smallest team (lowest index on ties).

use crate::wire::{MAX_SLOTS, MachineView, PlayerSlot, SlotRequest, TeamMode};

/// Number of distinct player colours (the original has more than 12 skins;
/// colour indices are taken modulo this when resolving clashes).
pub const COLOURS: u8 = 12;

pub fn clean_name(s: &str, fallback: &str) -> String {
    let t: String = s.chars().filter(|c| !c.is_control()).take(16).collect();
    let t = t.trim().to_string();
    if t.is_empty() {
        fallback.to_string()
    } else {
        t
    }
}

/// Assign global slots for the machines that play (lobby order, `waiting`
/// ignored: callers pass only the machines that should play).
pub fn assign(machines: &[MachineView], mode: TeamMode) -> Vec<PlayerSlot> {
    let mut ms: Vec<&MachineView> = machines.iter().collect();
    ms.sort_by_key(|m| m.machine);
    let mut out: Vec<PlayerSlot> = Vec::new();
    let mut used_colours = [false; COLOURS as usize];
    let teams = match mode {
        TeamMode::Ffa => 0,
        TeamMode::Teams(n) => n.clamp(2, MAX_SLOTS as u8),
    };
    let mut team_size = vec![0usize; teams as usize];
    for m in ms {
        for (li, req) in m.slots.iter().enumerate() {
            if out.len() >= MAX_SLOTS {
                return out;
            }
            let slot = out.len() as u8;
            let want = req.colour % COLOURS;
            let colour = if !used_colours[want as usize] {
                want
            } else {
                (0..COLOURS)
                    .find(|c| !used_colours[*c as usize])
                    .unwrap_or(want)
            };
            used_colours[colour as usize] = true;
            let team = if teams == 0 {
                slot
            } else {
                match req.team {
                    Some(t) if t < teams => t,
                    _ => (0..teams)
                        .min_by_key(|t| (team_size[*t as usize], *t))
                        .unwrap_or(0),
                }
            };
            if teams > 0 {
                team_size[team as usize] += 1;
            }
            out.push(PlayerSlot {
                slot,
                machine: m.machine,
                local_index: li as u8,
                name: clean_name(
                    &req.name,
                    &format!("{} {}", clean_name(&m.name, "player"), li + 1),
                ),
                colour,
                team,
                extra: req.extra.clone(),
            });
        }
    }
    out
}

/// How many of each machine's slots fit, in lobby order (for display).
pub fn accepted_slots(machines: &[MachineView]) -> Vec<(crate::wire::MachineId, usize)> {
    let mut ms: Vec<&MachineView> = machines.iter().collect();
    ms.sort_by_key(|m| m.machine);
    let mut left = MAX_SLOTS;
    ms.iter()
        .map(|m| {
            let n = m.slots.len().min(left);
            left -= n;
            (m.machine, n)
        })
        .collect()
}

pub fn sanitize_slots(v: Vec<SlotRequest>) -> Vec<SlotRequest> {
    v.into_iter()
        .take(MAX_SLOTS)
        .map(|s| SlotRequest {
            name: clean_name(&s.name, ""),
            extra: s
                .extra
                .into_iter()
                .take(crate::wire::MAX_SLOT_EXTRA)
                .collect(),
            ..s
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::MachineId;

    fn mach(id: u8, n: usize, colour: u8, team: Option<u8>) -> MachineView {
        MachineView {
            machine: MachineId(id),
            account: id as i64,
            name: format!("m{id}"),
            ready: true,
            waiting: false,
            slots: (0..n)
                .map(|i| SlotRequest {
                    name: format!("p{id}.{i}"),
                    colour,
                    team,
                    extra: Vec::new(),
                })
                .collect(),
        }
    }

    #[test]
    fn order_and_cap() {
        let ms = vec![
            mach(2, 5, 0, None),
            mach(0, 4, 0, None),
            mach(1, 5, 0, None),
        ];
        let s = assign(&ms, TeamMode::Ffa);
        assert_eq!(s.len(), 12);
        assert_eq!(s[0].machine, MachineId(0));
        assert_eq!(s[4].machine, MachineId(1));
        assert_eq!(s[9].machine, MachineId(2));
        assert_eq!(s[11].local_index, 2);
        assert!(
            s.iter()
                .enumerate()
                .all(|(i, p)| p.slot as usize == i && p.team as usize == i)
        );
        let mut c: Vec<u8> = s.iter().map(|p| p.colour).collect();
        c.sort();
        c.dedup();
        assert_eq!(c.len(), 12, "colours unique");
        assert_eq!(
            accepted_slots(&ms),
            vec![(MachineId(0), 4), (MachineId(1), 5), (MachineId(2), 3)]
        );
    }

    #[test]
    fn teams_requests_and_balance() {
        let ms = vec![
            mach(0, 3, 1, Some(1)),
            mach(1, 3, 1, None),
            mach(2, 2, 5, Some(9)),
        ];
        let s = assign(&ms, TeamMode::Teams(2));
        let t: Vec<u8> = s.iter().map(|p| p.team).collect();
        // three requested team 1, then balance: 0,0,0, then invalid request -> smallest (0 has 3, 1 has 3 -> 0), then 1
        assert_eq!(t, vec![1, 1, 1, 0, 0, 0, 0, 1]);
        assert_eq!(s[0].colour, 1);
        assert_eq!(s[1].colour, 0);
        assert_eq!(s[2].colour, 2);
        assert_eq!(s[6].colour, 6, "5 was taken by machine 1");
    }

    #[test]
    fn names() {
        assert_eq!(clean_name("  a\u{7}b  ", "x"), "ab");
        assert_eq!(clean_name("", "x"), "x");
        assert_eq!(clean_name("12345678901234567890", "x").len(), 16);
    }
}
