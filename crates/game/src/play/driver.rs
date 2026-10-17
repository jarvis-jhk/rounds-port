//! Where simulation frames come from.
//!
//! A [`MatchDriver`] owns the sim worlds and advances them; the presentation
//! only reads `previous()`/`current()` and the events. The local driver
//! steps once per tick with every player's input. An online driver (GGRS
//! from `rounds-net`, later) implements the same trait: it receives only the
//! local players' inputs, gets confirmed/predicted inputs for remote ones,
//! saves and loads world snapshots on GGRS request (`World: Clone`), may
//! advance several frames in one tick (catch-up after a rollback) or none
//! (waiting), and re-delivers events of re-simulated frames; the
//! [`EventFilter`] makes each `(frame, seq)` event fire once.

use std::collections::BTreeSet;
use std::sync::Arc;

use rounds_sim::{Content, Event, MatchSetup, PlayerInput, World};

/// Result of one [`MatchDriver::advance`] call.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Advance {
    /// Sim frames stepped (including re-simulated ones).
    pub frames: u32,
    /// Set when a rollback happened: the first frame that was re-simulated.
    /// Events of later frames that were already presented and are not
    /// re-produced may be cancelled (long sounds).
    pub rolled_back_to: Option<u32>,
}

/// How a match ended (for the app flow).
#[derive(Clone, Debug, PartialEq)]
pub enum MatchOutcome {
    /// The sim's flow reached `Ended` ("No" at the rematch prompt).
    Ended,
    /// Online: every machine reported the end; `agreed` = same frame and
    /// checksum everywhere.
    OnlineEnded {
        agreed: bool,
        reports: Vec<rounds_net::DoneReport>,
    },
    /// Online: the match cannot continue (host left, we were dropped).
    Aborted(String),
}

/// Connection state for the online overlay.
#[derive(Clone, Debug, Default)]
pub struct NetStatus {
    pub stats: rounds_net::NetStats,
    pub synchronising: bool,
    pub frozen: bool,
    pub gone_slots: Vec<usize>,
    pub desyncs: usize,
}

pub trait MatchDriver {
    /// Advances by one presentation tick (0.014 s) with the inputs of the
    /// players simulated on this machine, `(slot, input)`.
    fn advance(&mut self, local: &[(usize, PlayerInput)]) -> Advance;
    /// State before the last stepped frame (interpolation start).
    fn previous(&self) -> &World;
    /// Latest state (confirmed or predicted).
    fn current(&self) -> &World;
    /// Moves out every event produced since the last call, in frame order.
    fn drain_events(&mut self, out: &mut Vec<Event>);
    fn content(&self) -> &Arc<Content>;
    /// Set once the match is over.
    fn outcome(&self) -> Option<MatchOutcome> {
        let w = self.current();
        matches!(w.phase(), rounds_sim::flow::Phase::Ended).then_some(MatchOutcome::Ended)
    }
    /// Keep the world of `frame` once it is final (confirmed online).
    fn request_snapshot(&mut self, _frame: u32) {}
    /// A requested world that became final.
    fn take_snapshot(&mut self) -> Option<World> {
        None
    }
    /// Online connection state (`None` for local play).
    fn net_status(&self) -> Option<NetStatus> {
        None
    }
    /// True if pausing may stop the simulation (local play).
    fn uses_local_pause(&self) -> bool {
        true
    }
}

/// Local play: every player is on this machine, one step per tick.
pub struct LocalDriver {
    content: Arc<Content>,
    prev: World,
    cur: World,
    inputs: Vec<PlayerInput>,
    events: Vec<Event>,
    /// Steps per tick (1; 0 pauses; > 1 fast-forwards headless runs).
    pub speed: u32,
    snapshots: Vec<u32>,
    ready: Vec<World>,
}

impl LocalDriver {
    /// Sets the pick mode before the first step (`World::set_pick_mode`).
    pub fn with_pick_mode(mut self, mode: rounds_sim::PickMode) -> LocalDriver {
        self.cur.set_pick_mode(mode);
        self.prev.set_pick_mode(mode);
        self
    }

    pub fn new(content: Arc<Content>, setup: &MatchSetup) -> LocalDriver {
        let cur = World::new(&content, setup);
        let n = setup.slots.len();
        LocalDriver {
            prev: cur.clone(),
            cur,
            content,
            inputs: vec![PlayerInput::default(); n],
            events: Vec::new(),
            speed: 1,
            snapshots: Vec::new(),
            ready: Vec::new(),
        }
    }
}

impl LocalDriver {
    /// Debug: gives every player the named cards before the match starts.
    pub fn give_cards(&mut self, names: &[String]) -> Vec<String> {
        let mut unknown = Vec::new();
        for n in names {
            let id = self.content.card_by_name(n).or_else(|| {
                let up = n.to_uppercase();
                self.content
                    .cards
                    .iter()
                    .position(|c| c.name.to_uppercase() == up)
                    .map(|i| rounds_sim::CardId(i as u16))
            });
            match id {
                Some(id) => {
                    for p in 0..self.cur.players().len() {
                        self.cur.debug_give_card(&self.content, p, id);
                    }
                }
                None => unknown.push(n.clone()),
            }
        }
        self.prev = self.cur.clone();
        unknown
    }
}

impl MatchDriver for LocalDriver {
    fn advance(&mut self, local: &[(usize, PlayerInput)]) -> Advance {
        self.inputs.fill(PlayerInput::default());
        for &(slot, input) in local {
            if let Some(i) = self.inputs.get_mut(slot) {
                *i = input;
            }
        }
        for _ in 0..self.speed {
            self.prev.clone_from(&self.cur);
            self.cur.step(&self.content, &self.inputs);
            self.events.extend_from_slice(self.cur.events());
            let f = self.cur.frame();
            if let Some(i) = self.snapshots.iter().position(|x| *x == f) {
                self.snapshots.remove(i);
                self.ready.push(self.cur.clone());
            }
        }
        Advance {
            frames: self.speed,
            rolled_back_to: None,
        }
    }

    fn previous(&self) -> &World {
        &self.prev
    }

    fn current(&self) -> &World {
        &self.cur
    }

    fn drain_events(&mut self, out: &mut Vec<Event>) {
        out.append(&mut self.events);
    }

    fn content(&self) -> &Arc<Content> {
        &self.content
    }

    fn request_snapshot(&mut self, frame: u32) {
        self.snapshots.push(frame);
    }

    fn take_snapshot(&mut self) -> Option<World> {
        if self.ready.is_empty() {
            None
        } else {
            Some(self.ready.remove(0))
        }
    }
}

/// Lets each `(frame, seq)` event through once (rollback re-delivers
/// events of re-simulated frames). Remembers a window of recent frames.
#[derive(Default)]
pub struct EventFilter {
    seen: BTreeSet<(u32, u32, u64)>,
    newest: u32,
}

/// Frames kept in the filter (> the longest rollback GGRS allows).
const WINDOW: u32 = 128;

impl EventFilter {
    /// True the first time an event key is seen. The key is (frame, seq,
    /// kind): a re-simulated frame whose prediction was right re-produces
    /// the same keys; a corrected one may produce other kinds, which show.
    pub fn first(&mut self, e: &Event) -> bool {
        use std::hash::{Hash, Hasher};
        self.newest = self.newest.max(e.frame);
        if self.seen.len() > 4096 {
            let cut = self.newest.saturating_sub(WINDOW);
            self.seen = self.seen.split_off(&(cut, 0, 0));
        }
        let mut h = std::collections::hash_map::DefaultHasher::new();
        std::mem::discriminant(&e.kind).hash(&mut h);
        self.seen.insert((e.frame, e.seq, h.finish()))
    }

    #[allow(dead_code)] // for the online driver
    /// Forget events of frames after `frame` that were rolled back and not
    /// re-produced (so a later re-simulation can present them again).
    pub fn rollback(&mut self, frame: u32) {
        self.seen.retain(|(f, _, _)| *f < frame);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rounds_sim::EventKind;

    #[test]
    fn filter_passes_each_event_once() {
        let mut f = EventFilter::default();
        let e = Event {
            frame: 5,
            seq: 0,
            kind: EventKind::MatchStarted,
        };
        assert!(f.first(&e));
        assert!(!f.first(&e));
        f.rollback(5);
        assert!(f.first(&e));
        let e2 = Event {
            seq: 1,
            ..e.clone()
        };
        assert!(f.first(&e2));
    }
}
