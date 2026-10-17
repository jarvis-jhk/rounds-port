//! [`OnlineDriver`]: the match driver over a rounds-net GGRS session
//! (docs/net.md "API for the game crate").
//!
//! Each tick it polls the session, hands GGRS the local slots' inputs and
//! fulfils its requests: `Save` = `World::clone` + `World::checksum` (desync
//! detection), `Load` = restore (rollback), `Advance` = one sim step with
//! every slot's input. Slots of machines that dropped out arrive as
//! `Disconnected` zero inputs; [`gone_input`] turns them into the sim's
//! forfeit bit, so the dropped players leave the match (skipped in picks,
//! their team no longer alive) at the same frame on every machine. While we are ahead of the slowest
//! machine every tenth tick is skipped (about 10 % slower), and GGRS wait
//! recommendations skip whole ticks.
//!
//! The match ends when the sim's flow reaches `Ended` ("No" at the rematch
//! prompt) at a frame every machine has confirmed: the driver reports that
//! frame and its checksum (`finish_match`) and keeps advancing until the
//! host's `MatchEnded` arrives.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use rounds_net::{InputStatus, NetEvent, Request};
use rounds_sim::flow::Phase;
use rounds_sim::{Content, Event, PlayerInput, World};

use super::NetRef;
use crate::play::driver::{Advance, MatchDriver, MatchOutcome, NetStatus};

/// Input of a slot whose machine left: the forfeit bit. GGRS reports the
/// dropped machine's slots as `Disconnected` from the resume frame `D + 1`
/// on, identically on every machine (they are zero-input dummies of the
/// host), so the sim's `World::set_disconnected` happens at the same frame
/// everywhere (match-flow.md 16, "forfeit / disconnect"). Going through the
/// input instead of calling `set_disconnected` after the `Load` keeps
/// rollbacks safe: a re-loaded state that already has the removal queued or
/// applied is not touched twice. Later frames repeat the bit; the sim
/// ignores input of disconnected players.
pub fn gone_input() -> PlayerInput {
    PlayerInput {
        buttons: PlayerInput::BTN_FORFEIT,
        ..Default::default()
    }
}

pub struct OnlineDriver {
    net: NetRef,
    content: Arc<Content>,
    prev: World,
    cur: World,
    events: Vec<Event>,
    /// Per simulated frame: (checksum, flow ended). Re-simulation
    /// overwrites; old frames are pruned.
    history: BTreeMap<u32, (u64, bool)>,
    finished: Option<(u32, u64)>,
    outcome: Option<MatchOutcome>,
    slow: u32,
    skip: u32,
    /// Snapshot requests: frame -> latest simulated world of that frame.
    snapshots: BTreeMap<u32, Option<World>>,
    ready_snapshots: Vec<World>,
    gone_slots: BTreeSet<usize>,
    frozen: bool,
    desyncs: Vec<String>,
    pub log: Vec<String>,
    started: bool,
    /// Print a status line this often (headless test runs).
    pub status_every: Option<std::time::Duration>,
    last_status: std::time::Instant,
}

impl OnlineDriver {
    pub fn new(net: NetRef, content: Arc<Content>, world: World) -> OnlineDriver {
        OnlineDriver {
            net,
            content,
            prev: world.clone(),
            cur: world,
            events: Vec::new(),
            history: BTreeMap::new(),
            finished: None,
            outcome: None,
            slow: 0,
            skip: 0,
            snapshots: BTreeMap::new(),
            ready_snapshots: Vec::new(),
            gone_slots: BTreeSet::new(),
            frozen: false,
            desyncs: Vec::new(),
            log: Vec::new(),
            started: false,
            status_every: None,
            last_status: std::time::Instant::now(),
        }
    }

    fn say(&mut self, s: String) {
        if self.status_every.is_some() {
            println!("{s}");
        } else {
            log::info!("{s}");
        }
        self.log.push(s);
    }

    fn handle_events(&mut self) {
        loop {
            let e = self.net.borrow_mut().next_event();
            let Some(e) = e else { break };
            match e {
                NetEvent::Running => {
                    self.started = true;
                    self.say("net: running".into());
                }
                NetEvent::MatchFrozen { gone } => {
                    self.frozen = true;
                    self.say(format!("net: frozen, dropping {gone:?}"));
                }
                NetEvent::PlayersDropped {
                    machines,
                    slots,
                    from_frame,
                } => {
                    self.frozen = false;
                    self.gone_slots.extend(slots.iter().copied());
                    self.say(format!(
                        "net: resumed at frame {from_frame} without {machines:?} (slots {slots:?})"
                    ));
                }
                NetEvent::Desync {
                    frame,
                    local,
                    remote,
                    machine,
                } => {
                    let s = format!(
                        "DESYNC at frame {frame} with {machine}: {local:032x} vs {remote:032x}"
                    );
                    self.desyncs.push(s.clone());
                    self.say(s);
                }
                NetEvent::MatchEnded {
                    agreed, reports, ..
                } => {
                    self.say(format!(
                        "net: match ended, {} reports, agreed={agreed}",
                        reports.len()
                    ));
                    self.print_result(agreed, reports.len());
                    self.outcome = Some(MatchOutcome::OnlineEnded { agreed, reports });
                }
                NetEvent::MatchAborted { reason } => {
                    self.say(format!("net: match aborted: {reason}"));
                    if self.outcome.is_none() {
                        self.outcome = Some(MatchOutcome::Aborted(reason));
                    }
                }
                NetEvent::RoomLeft { reason } => {
                    self.say(format!("net: room left: {reason}"));
                    if self.outcome.is_none() {
                        self.outcome = Some(MatchOutcome::Aborted(reason));
                    }
                }
                NetEvent::WaitRecommendation { frames } => {
                    self.skip = self.skip.max(frames.min(8));
                }
                NetEvent::NetworkInterrupted { machine, .. } => {
                    self.say(format!("net: {machine} interrupted"))
                }
                NetEvent::NetworkResumed { machine } => self.say(format!("net: {machine} resumed")),
                _ => {}
            }
        }
    }

    fn step(&mut self, inputs: &[(PlayerInput, InputStatus)]) {
        let ins: Vec<PlayerInput> = inputs
            .iter()
            .map(|(i, st)| {
                if *st == InputStatus::Disconnected {
                    gone_input()
                } else {
                    *i
                }
            })
            .collect();
        self.prev.clone_from(&self.cur);
        self.cur.step(&self.content, &ins);
        self.events.extend_from_slice(self.cur.events());
        let f = self.cur.frame();
        let ended = matches!(self.cur.phase(), Phase::Ended);
        self.history.insert(f, (self.cur.checksum(), ended));
        if let Some(s) = self.snapshots.get_mut(&f) {
            *s = Some(self.cur.clone());
        }
    }

    /// Reports the end once its frame is confirmed everywhere.
    fn check_finish(&mut self) {
        if self.finished.is_some() {
            return;
        }
        let confirmed = self.net.borrow().confirmed_frame();
        if confirmed < 0 {
            return;
        }
        let confirmed = confirmed as u32;
        let first_end = self
            .history
            .iter()
            .find(|(f, (_, ended))| *ended && **f <= confirmed)
            .map(|(f, (c, _))| (*f, *c));
        if let Some((f, c)) = first_end {
            match self.net.borrow_mut().finish_match(f as i32, c as u128) {
                Ok(()) => {
                    self.finished = Some((f, c));
                }
                Err(e) => log::warn!("finish_match: {e}"),
            }
            if let Some((f, c)) = self.finished {
                let winner = self.winner();
                self.say(format!(
                    "net: finished at frame {f} checksum {c:016x} winner {winner:?}"
                ));
            }
        }
        // Prune history and promote confirmed snapshots.
        let keep = confirmed.saturating_sub(256);
        self.history.retain(|f, _| *f >= keep);
        let done: Vec<u32> = self
            .snapshots
            .iter()
            .filter(|(f, w)| **f <= confirmed && w.is_some())
            .map(|(f, _)| *f)
            .collect();
        for f in done {
            if let Some(Some(w)) = self.snapshots.remove(&f) {
                self.ready_snapshots.push(w);
            }
        }
    }

    /// The machine-readable result line `scripts/net/online-test.sh` reads.
    fn print_result(&self, agreed: bool, reports: usize) {
        let net = self.net.borrow();
        let s = net
            .last_match_stats()
            .cloned()
            .unwrap_or_else(|| net.stats());
        let (f, c) = self.finished.unwrap_or((0, 0));
        let ping = s
            .machines
            .iter()
            .filter_map(|m| m.ggrs.as_ref().map(|g| g.ping))
            .max()
            .unwrap_or(0);
        let relayed = s
            .machines
            .iter()
            .filter(|m| m.relayed == Some(true))
            .count();
        println!(
            "RESULT machine={} frame={f} checksum={c:016x} winner={} agreed={agreed} reports={reports} desyncs={} rollbacks={} resim={} maxdepth={} stalls={} resumes={} ping={ping} relayed_links={relayed} gone_slots={}",
            net.me().map_or(-1, |m| m.0 as i32),
            self.winner().map_or("-".to_string(), |w| w.to_string()),
            self.desyncs.len().max(s.desyncs as usize),
            s.rollbacks,
            s.resimulated_frames,
            s.max_rollback_depth,
            s.prediction_stalls,
            s.resumes,
            self.gone_slots.len(),
        );
    }

    /// Team with the most rounds (the game's winner at the end).
    pub fn winner(&self) -> Option<u8> {
        let r = self.cur.rounds();
        let max = *r.iter().max()?;
        if max == 0 {
            return None;
        }
        r.iter().position(|x| *x == max).map(|t| t as u8)
    }
}

impl MatchDriver for OnlineDriver {
    fn advance(&mut self, local: &[(usize, PlayerInput)]) -> Advance {
        self.net.borrow_mut().poll();
        self.handle_events();
        let mut adv = Advance::default();
        if self.outcome.is_some() {
            return adv;
        }
        if self.skip > 0 {
            self.skip -= 1;
            return adv;
        }
        if self.net.borrow().frames_ahead() > 0 {
            self.slow += 1;
            if self.slow.is_multiple_of(10) {
                return adv;
            }
        }
        let reqs = {
            let mut net = self.net.borrow_mut();
            if net.current_match().is_none() {
                return adv;
            }
            for &(slot, input) in local {
                let _ = net.add_local_input(slot, input);
            }
            net.advance()
        };
        for r in reqs {
            match r {
                Request::Save { cell, frame } => {
                    let c = self.cur.checksum();
                    cell.save(frame, Some(self.cur.clone()), Some(c as u128));
                }
                Request::Load { cell, frame } => {
                    if let Some(w) = cell.load() {
                        self.cur = w;
                        self.prev.clone_from(&self.cur);
                    }
                    let f = frame.max(0) as u32;
                    adv.rolled_back_to = Some(adv.rolled_back_to.map_or(f, |x: u32| x.min(f)));
                }
                Request::Advance { inputs } => {
                    self.step(&inputs);
                    adv.frames += 1;
                }
            }
        }
        self.handle_events();
        self.check_finish();
        if let Some(every) = self.status_every
            && self.last_status.elapsed() >= every
        {
            self.last_status = std::time::Instant::now();
            let s = self.net.borrow().stats();
            println!(
                "status frame {} confirmed {} ahead {} phase {:?} rounds {:?} rollbacks {} stalls {}",
                s.current_frame,
                s.confirmed_frame,
                s.frames_ahead,
                self.cur.phase(),
                self.cur.rounds(),
                s.rollbacks,
                s.prediction_stalls
            );
        }
        adv
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

    fn outcome(&self) -> Option<MatchOutcome> {
        self.outcome.clone()
    }

    fn request_snapshot(&mut self, frame: u32) {
        self.snapshots.entry(frame).or_insert(None);
    }

    fn take_snapshot(&mut self) -> Option<World> {
        if self.ready_snapshots.is_empty() {
            None
        } else {
            Some(self.ready_snapshots.remove(0))
        }
    }

    fn net_status(&self) -> Option<NetStatus> {
        let net = self.net.borrow();
        let s = net.stats();
        Some(NetStatus {
            stats: s,
            synchronising: !self.started,
            frozen: self.frozen,
            gone_slots: self.gone_slots.iter().copied().collect(),
            desyncs: self.desyncs.len(),
        })
    }

    fn uses_local_pause(&self) -> bool {
        false
    }
}
