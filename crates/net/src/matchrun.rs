//! The GGRS side of one match: sessions ("epochs"), the soft prediction
//! limit, frame offsets, and freeze/resume when machines drop.
//!
//! GGRS 0.13 cannot drop a peer safely in sessions with more than two
//! machines: each machine disconnects the peer at the last frame *it*
//! received, and when those differ GGRS has to roll back past inputs it has
//! already discarded (`input_queue.rs` assertion), or past its window. So we
//! never let GGRS disconnect anybody. Instead the host freezes the match,
//! every machine reports the newest frame it has *fully confirmed and
//! simulated*, the host picks the minimum `D`, and every machine restores its
//! saved state of frame `D + 1` and continues in a fresh GGRS session in
//! which the dropped machine's slots are dummy local players of the host
//! that always send the zero input (reported as `InputStatus::Disconnected`).
//! Every machine thus sees the dropped players' real inputs up to frame `D`
//! and zero inputs from `D + 1` on: deterministic.

use crate::input::{NetInput, Wire};
use crate::session::{GgrsConfig, NetError, NetEvent, NetStats};
use crate::transport::{Transport, WzSocket};
use crate::wire::{HOST_MACHINE, MachineId, MatchStart};
use ggrs::{
    DesyncDetection, GameStateCell, GgrsError, GgrsEvent, GgrsRequest, InputStatus, P2PSession,
    PlayerType, SessionBuilder, SessionState,
};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::time::Duration;

/// A saved-state slot handed to the game. Frames are sim frames (they keep
/// counting across the internal GGRS sessions of one match).
pub struct StateCell<S> {
    cell: GameStateCell<S>,
    base: i32,
}

impl<S> Clone for StateCell<S> {
    fn clone(&self) -> Self {
        StateCell {
            cell: self.cell.clone(),
            base: self.base,
        }
    }
}

impl<S> StateCell<S> {
    /// Store the state of sim frame `frame` (the frame of the request).
    pub fn save(&self, frame: i32, state: Option<S>, checksum: Option<u128>) {
        self.cell.save(frame - self.base, state, checksum);
    }
}

impl<S: Clone> StateCell<S> {
    pub fn load(&self) -> Option<S> {
        self.cell.load()
    }
}

/// What the game must do, in order, after [`crate::NetSession::advance`].
pub enum Request<I, S> {
    /// Save the current state: `cell.save(frame, Some(state.clone()),
    /// Some(checksum))`. The checksum feeds desync detection.
    Save { cell: StateCell<S>, frame: i32 },
    /// Restore the state saved for `frame`: `state = cell.load().unwrap()`.
    Load { cell: StateCell<S>, frame: i32 },
    /// Step the sim once with one input per slot (index = global slot).
    /// `InputStatus::Disconnected` (always with the zero input) marks
    /// players whose machine dropped out.
    Advance { inputs: Vec<(I, InputStatus)> },
}

pub(crate) struct MatchRun<I: NetInput, S: 'static> {
    pub info: MatchStart,
    pub me: MachineId,
    /// Slots played on this machine.
    pub local: Vec<usize>,
    pub gone: BTreeSet<MachineId>,
    pub epoch: u8,
    /// Sim frame of GGRS frame 0 in the current session.
    base: i32,
    session: P2PSession<GgrsConfig<I, S>>,
    inputs: Vec<Option<I>>,
    /// Sim frame -> the cell its state was saved in (newest window only).
    cells: BTreeMap<i32, GameStateCell<S>>,
    /// Newest sim frame whose inputs were all confirmed when the requests of
    /// the last advance were produced (so its state is final once they ran).
    safe_confirmed: i32,
    pub frozen: Option<u8>,
    /// The state this session started from (after a resume) ...
    restart: Option<(StateCell<S>, i32)>,
    /// ... still to be handed to the game as a Load request.
    load_pending: bool,
    pub running: bool,
    pub finished: bool,
}

fn build<I: NetInput, S: 'static>(
    info: &MatchStart,
    me: MachineId,
    epoch: u8,
    gone: &BTreeSet<MachineId>,
    t: &Rc<RefCell<Transport>>,
) -> Result<P2PSession<GgrsConfig<I, S>>, NetError> {
    let p = info.ggrs;
    let g = |e: GgrsError| NetError::Ggrs(e.to_string());
    let mut b = SessionBuilder::<GgrsConfig<I, S>>::new()
        .with_num_players(info.players.len())
        .map_err(g)?
        .with_input_delay(p.input_delay as usize)
        .with_max_prediction_window(p.rollback_window.max(p.max_prediction) as usize)
        .with_fps(p.fps.max(1) as usize)
        .map_err(g)?
        .with_desync_detection_mode(if p.desync_interval > 0 {
            DesyncDetection::On {
                interval: p.desync_interval as u32,
            }
        } else {
            DesyncDetection::Off
        })
        // Drops are decided by the host (freeze/resume), never by GGRS.
        .with_disconnect_timeout(Duration::from_secs(600))
        .with_disconnect_notify_delay(Duration::from_millis(p.disconnect_notify_ms as u64));
    for pl in &info.players {
        let owner = if gone.contains(&pl.machine) {
            HOST_MACHINE
        } else {
            pl.machine
        };
        let ty = if owner == me {
            PlayerType::Local
        } else {
            PlayerType::Remote(owner)
        };
        b = b.add_player(ty, pl.slot as usize).map_err(g)?;
    }
    t.borrow_mut().set_match_tag(tag(info.match_id, epoch));
    b.start_p2p_session(WzSocket(t.clone())).map_err(g)
}

fn tag(match_id: u32, epoch: u8) -> u16 {
    ((match_id as u16) << 4) | (epoch as u16 & 0xf)
}

impl<I: NetInput, S: 'static> MatchRun<I, S> {
    pub fn new(
        info: MatchStart,
        me: MachineId,
        t: &Rc<RefCell<Transport>>,
    ) -> Result<Self, NetError> {
        let gone = BTreeSet::new();
        let session = build(&info, me, 0, &gone, t)?;
        let local = info.slots_of(me);
        let n = info.players.len();
        Ok(MatchRun {
            info,
            me,
            local,
            gone,
            epoch: 0,
            base: 0,
            session,
            inputs: vec![None; n],
            cells: BTreeMap::new(),
            safe_confirmed: -1,
            frozen: None,
            restart: None,
            load_pending: false,
            running: false,
            finished: false,
        })
    }

    pub fn add_input(&mut self, slot: usize, input: I) -> Result<(), NetError> {
        if !self.local.contains(&slot) {
            return Err(NetError::NotLocal(slot));
        }
        self.inputs[slot] = Some(input);
        Ok(())
    }

    pub fn current_frame(&self) -> i32 {
        self.session.current_frame() + self.base
    }

    pub fn confirmed_frame(&self) -> i32 {
        let c = self.session.confirmed_frame();
        if c < 0 { self.base - 1 } else { c + self.base }
    }

    pub fn frames_ahead(&self) -> i32 {
        self.session.frames_ahead()
    }

    pub fn network_stats(&self, m: MachineId) -> Option<ggrs::NetworkStats> {
        if self.gone.contains(&m) {
            return None;
        }
        let h = *self.info.slots_of(m).first()?;
        self.session.network_stats(h).ok()
    }

    pub fn poll(&mut self) {
        self.session.poll_remote_clients();
    }

    fn gone_slot(&self, slot: usize) -> bool {
        self.info
            .players
            .get(slot)
            .is_some_and(|p| self.gone.contains(&p.machine))
    }

    pub fn advance(&mut self, stats: &mut NetStats) -> Vec<Request<I, S>> {
        let mut out = Vec::new();
        if self.frozen.is_some() {
            self.session.poll_remote_clients();
            return out;
        }
        if self.load_pending
            && let Some((cell, frame)) = self.restart.clone()
        {
            self.load_pending = false;
            out.push(Request::Load { cell, frame });
        }
        for pl in &self.info.players {
            let slot = pl.slot as usize;
            let mine = self.local.contains(&slot);
            let dummy = self.me == HOST_MACHINE && self.gone.contains(&pl.machine);
            if mine || dummy {
                let i =
                    if mine { self.inputs[slot].take() } else { None }.unwrap_or_else(I::zeroed);
                let _ = self.session.add_local_input(slot, Wire(i));
            }
        }
        let ggrs_frame = self.session.current_frame();
        let ahead = ggrs_frame - self.session.confirmed_frame();
        if self.session.current_state() != SessionState::Running {
            self.session.poll_remote_clients();
            return out;
        }
        if ahead > self.info.ggrs.max_prediction as i32 {
            self.session.poll_remote_clients();
            stats.prediction_stalls += 1;
            return out;
        }
        match self.session.advance_frame() {
            Ok(reqs) => {
                let base = self.base;
                let mut after_load = false;
                for r in reqs {
                    out.push(match r {
                        GgrsRequest::SaveGameState { cell, frame } => {
                            self.cells.insert(frame + base, cell.clone());
                            Request::Save {
                                cell: StateCell { cell, base },
                                frame: frame + base,
                            }
                        }
                        GgrsRequest::LoadGameState { cell, frame } => {
                            stats.rollbacks += 1;
                            stats.max_rollback_depth = stats
                                .max_rollback_depth
                                .max((ggrs_frame - frame).max(0) as u32);
                            after_load = true;
                            Request::Load {
                                cell: StateCell { cell, base },
                                frame: frame + base,
                            }
                        }
                        GgrsRequest::AdvanceFrame { inputs } => {
                            if after_load {
                                stats.resimulated_frames += 1;
                            }
                            let inputs = inputs
                                .into_iter()
                                .enumerate()
                                .map(|(slot, (w, s))| {
                                    if self.gone_slot(slot) {
                                        (I::zeroed(), InputStatus::Disconnected)
                                    } else {
                                        (w.0, s)
                                    }
                                })
                                .collect();
                            Request::Advance { inputs }
                        }
                    });
                }
                if after_load {
                    // The last advance is the new frame, not a re-simulation.
                    stats.resimulated_frames = stats.resimulated_frames.saturating_sub(1);
                }
                let conf = self.session.confirmed_frame();
                if conf >= 0 {
                    self.safe_confirmed = self.safe_confirmed.max(conf + base);
                }
                let keep_from = self.current_frame()
                    - self
                        .info
                        .ggrs
                        .rollback_window
                        .max(self.info.ggrs.max_prediction) as i32;
                self.cells.retain(|f, _| *f >= keep_from);
            }
            Err(GgrsError::PredictionThreshold) => stats.prediction_stalls += 1,
            Err(e) => log::warn!("advance_frame: {e}"),
        }
        out
    }

    /// Stop advancing; returns the frame `r` we can restart from (`r + 1`
    /// is saved and final here).
    pub fn freeze(&mut self, epoch: u8) -> i32 {
        self.frozen = Some(epoch);
        let newest_saved = self.cells.keys().next_back().copied().unwrap_or(self.base);
        self.safe_confirmed.min(newest_saved - 1).max(self.base - 1)
    }

    /// Continue from sim frame `frame` in a new session without `gone`.
    pub fn resume(
        &mut self,
        epoch: u8,
        frame: i32,
        gone: BTreeSet<MachineId>,
        t: &Rc<RefCell<Transport>>,
    ) -> Result<(), NetError> {
        // The state to restart from: a saved cell, the state this session
        // started from, or (nothing simulated yet) the initial state.
        let restart = match self.cells.get(&frame) {
            Some(c) => Some((
                StateCell {
                    cell: c.clone(),
                    base: self.base,
                },
                frame,
            )),
            None => match &self.restart {
                Some((c, f)) if *f == frame => Some((c.clone(), *f)),
                None if frame == self.base && self.current_frame() == self.base => None,
                _ => return Err(NetError::Ggrs(format!("no saved state for frame {frame}"))),
            },
        };
        self.gone = gone;
        self.epoch = epoch;
        self.session = build(&self.info, self.me, epoch, &self.gone, t)?;
        self.base = frame;
        self.cells.clear();
        self.safe_confirmed = frame - 1;
        self.load_pending = restart.is_some();
        self.restart = restart;
        self.frozen = None;
        self.running = false;
        Ok(())
    }

    /// GGRS events as [`NetEvent`]s (and the Running transition).
    pub fn events(&mut self, stats: &mut NetStats) -> Vec<NetEvent> {
        let mut out = Vec::new();
        for e in self.session.events() {
            out.push(match e {
                GgrsEvent::Synchronizing { addr, total, count } => NetEvent::Synchronizing {
                    machine: addr,
                    count,
                    total,
                },
                GgrsEvent::Synchronized { addr } => NetEvent::Synchronized { machine: addr },
                GgrsEvent::Disconnected { addr } => NetEvent::Disconnected { machine: addr },
                GgrsEvent::NetworkInterrupted {
                    addr,
                    disconnect_timeout,
                } => NetEvent::NetworkInterrupted {
                    machine: addr,
                    disconnect_timeout_ms: disconnect_timeout as u64,
                },
                GgrsEvent::NetworkResumed { addr } => NetEvent::NetworkResumed { machine: addr },
                GgrsEvent::WaitRecommendation { skip_frames } => NetEvent::WaitRecommendation {
                    frames: skip_frames,
                },
                GgrsEvent::DesyncDetected {
                    frame,
                    local_checksum,
                    remote_checksum,
                    addr,
                } => {
                    stats.desyncs += 1;
                    NetEvent::Desync {
                        frame: frame + self.base,
                        local: local_checksum,
                        remote: remote_checksum,
                        machine: addr,
                    }
                }
            });
        }
        let was = self.running;
        self.running = self.session.current_state() == SessionState::Running;
        if self.running && !was {
            out.push(NetEvent::Running);
        }
        out
    }
}
