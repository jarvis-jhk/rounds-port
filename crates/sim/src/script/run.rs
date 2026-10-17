//! Running compiled prefabs: object start, UnityEvent calls, per-tick
//! component updates, player hook dispatch, and the card effect behaviours
//! (cards.md 5.2, 6; weapons.md 19, 21).

use super::*;
use crate::content::Content;
use crate::effects::{BlockPhase, Hook};
use crate::events::{BlockKind, EventKind};
use crate::math::{V2, clamp, cos, pow, round_half_even, sin, v2};
use crate::objects::{Attach, ObjKind};
use crate::player::{ForceMode, PendingBlock, Pull};
use crate::world::{DT, World, ticks};

/// Real-time delay of a `DelayEvent`: one frame, then `WaitForSeconds`
/// (at least one more frame). cards.md 5.2 Clarified.
fn delay_ticks(time: f32) -> u32 {
    1 + ticks(time).max(1)
}

fn block_kind_index(k: BlockKind) -> u8 {
    match k {
        BlockKind::Default => 0,
        BlockKind::None => 1,
        BlockKind::ShieldCharge => 2,
        BlockKind::Echo => 3,
        BlockKind::Empower => 4,
    }
}

/// Initial per-instance state of a stateful component.
fn init_state(k: &Kind) -> CompState {
    match k {
        Kind::DelayEvent { .. } => CompState::Delay { pending: 0 },
        Kind::AttackLevel { level, .. } => CompState::Level { level: *level },
        Kind::LevelScale { .. } | Kind::FollowPlayer { .. } | Kind::CapScale { .. } => {
            CompState::StartScale {
                scale: 1.0,
                inited: false,
            }
        }
        Kind::PlayerInRangeTrigger { range, .. } => CompState::InRange {
            counter: 0.0,
            done: false,
            in_range: false,
            target: None,
            range: *range,
            started: false,
        },
        Kind::CounterUi { .. } | Kind::BlinkStep { .. } | Kind::LineEffect { .. } => {
            CompState::Timer {
                t: 0.0,
                done: false,
            }
        }
        Kind::StatsAfterDealingDamage { .. }
        | Kind::StatsWhenFullHp { .. }
        | Kind::ToggleStats { .. }
        | Kind::Chase { .. } => CompState::Toggle { on: false },
        Kind::AbyssalCountdown { .. } => CompState::Abyssal {
            counter: 0.0,
            remaining: 0.0,
            form: false,
            start_counter: 0.0,
        },
        Kind::ShieldCharge(_) => CompState::Charge(ChargeState {
            active: false,
            c: 0.0,
            used_time: 0.0,
            block_time: -1.0,
            dir: V2::ZERO,
            force: 0.0,
            damage: 0.0,
            knockback: 0.0,
            hits: 0,
            cancelled: false,
            wait: 0,
            from: V2::ZERO,
        }),
        Kind::Empower { .. } => CompState::Empower { armed: false },
        Kind::Teleport { .. } => CompState::Teleport {
            pending: 0,
            to: V2::ZERO,
        },
        Kind::RadarShot { .. } => CompState::Radar {
            shots: 0,
            ticks: 0,
            target: 0,
        },
        Kind::SpawnObjectOnDealDamage { .. } => CompState::Refresh {
            acc: 0.0,
            since: 1000.0,
        },
        Kind::CooldownCondition { cooldown, .. } => CompState::Cooldown {
            last: f32::NEG_INFINITY,
            cooldown: *cooldown,
            base: *cooldown,
        },
        Kind::CooldownWindUp { .. } => CompState::WindUp { value: 0.0 },
        Kind::DuringReloadTrigger { .. } => CompState::Reloading { was: false },
        Kind::Ensnare(_) => CompState::Ensnare {
            target: None,
            start: V2::ZERO,
            elapsed: 0.0,
            scale: 1.0,
            snapped: false,
        },
        Kind::Thruster { .. } => CompState::Thrust {
            dir: V2::ZERO,
            force: 0.0,
            started: false,
        },
        Kind::RotatingShooter { .. } => CompState::Shooter { reps: 0, ticks: 0 },
        Kind::LineRangeEffect { .. } => CompState::Done { done: false },
        _ => CompState::None,
    }
}

pub(crate) fn new_state(pf: &Prefab) -> Option<Box<ObjState>> {
    if pf.trivial {
        return None;
    }
    let mut comps = vec![CompState::None; pf.slots as usize];
    for c in &pf.comps {
        if let Some(s) = c.slot {
            comps[s as usize] = init_state(&c.kind);
        }
    }
    Some(Box::new(ObjState {
        active: pf.initial_active(),
        enabled: pf.initial_enabled(),
        started: 0,
        comps,
        recent_spawn: None,
        follow_stopped: false,
    }))
}

impl World {
    // ------------------------------------------------------------------
    // Lookup helpers
    // ------------------------------------------------------------------

    /// Index of a live object by id (objects are kept sorted by id).
    pub(crate) fn obj_index(&self, id: u32) -> Option<usize> {
        let i = self.objects.binary_search_by_key(&id, |o| o.id).ok()?;
        (!self.objects[i].dead).then_some(i)
    }

    /// The player an object acts for (effect owner or `SpawnedAttack.spawner`).
    pub(crate) fn obj_owner(&self, oi: usize) -> Option<usize> {
        let o = &self.objects[oi];
        match o.kind {
            ObjKind::PlayerEffect { owner } => Some(owner as usize),
            ObjKind::Spawned => o.spawner.map(|s| s as usize),
        }
    }

    /// World position of an object (player effects sit on their player,
    /// including the temporary Empower override position of a block).
    pub(crate) fn obj_pos(&self, oi: usize) -> V2 {
        let o = &self.objects[oi];
        match o.kind {
            ObjKind::PlayerEffect { owner } => self.players[owner as usize].pos,
            ObjKind::Spawned => o.pos,
        }
    }

    /// `transform.localScale.x` of a node.
    fn node_scale(&self, content: &Content, oi: usize, node: NodeIdx) -> f32 {
        let o = &self.objects[oi];
        if node == 0 {
            return o.scale;
        }
        let pf = content.prefab(o.def);
        // A FollowPlayer on the node rescales it with the player.
        if let Some(st) = &o.st {
            for c in pf.comps.iter().filter(|c| c.node == node) {
                if let (Kind::FollowPlayer { .. }, Some(s)) = (&c.kind, c.slot)
                    && let CompState::StartScale {
                        scale,
                        inited: true,
                    } = st.comps[s as usize]
                {
                    let ow = self.obj_owner(oi);
                    let ps = ow.map(|p| self.players[p].scale).unwrap_or(1.0);
                    return scale * ps;
                }
            }
        }
        pf.nodes.get(node as usize).map(|n| n.scale).unwrap_or(1.0)
    }

    /// `transform.root.localScale.x`: the player for player effects.
    fn root_scale(&self, oi: usize) -> f32 {
        let o = &self.objects[oi];
        match o.kind {
            ObjKind::PlayerEffect { owner } => self.players[owner as usize].scale,
            ObjKind::Spawned => o.scale,
        }
    }

    /// `AttackLevel.attackLevel` of the object (root component), else the
    /// `SpawnedAttack` level.
    pub(crate) fn obj_level(&self, content: &Content, oi: usize) -> i32 {
        let o = &self.objects[oi];
        let pf = content.prefab(o.def);
        if let (Some(ci), Some(st)) = (pf.attack_level, &o.st)
            && let Some(s) = pf.comps[ci as usize].slot
            && let CompState::Level { level } = st.comps[s as usize]
        {
            return level;
        }
        o.level.max(1)
    }

    fn level_scale_m(content: &Content, pf: &Prefab) -> f32 {
        let _ = content;
        pf.attack_level
            .and_then(|ci| match pf.comps[ci as usize].kind {
                Kind::AttackLevel { level_scale_m, .. } => Some(level_scale_m),
                _ => None,
            })
            .unwrap_or(0.7)
    }

    pub(crate) fn get_slot(&self, oi: usize, slot: Option<u16>) -> CompState {
        match (slot, &self.objects[oi].st) {
            (Some(s), Some(st)) => st.comps[s as usize],
            _ => CompState::None,
        }
    }

    pub(crate) fn set_slot(&mut self, oi: usize, slot: Option<u16>, v: CompState) {
        if let (Some(s), Some(st)) = (slot, self.objects[oi].st.as_mut()) {
            st.comps[s as usize] = v;
        }
    }

    fn node_active(&self, content: &Content, oi: usize, node: NodeIdx) -> bool {
        let o = &self.objects[oi];
        match &o.st {
            Some(st) => content.prefab(o.def).active_in_hierarchy(st.active, node),
            None => true,
        }
    }

    /// Component enabled and on an active node.
    fn comp_live(&self, content: &Content, oi: usize, ci: usize) -> bool {
        let o = &self.objects[oi];
        if o.dead {
            return false;
        }
        let pf = content.prefab(o.def);
        let c = &pf.comps[ci];
        match &o.st {
            Some(st) => st.comp_enabled(ci as CompIdx) && pf.active_in_hierarchy(st.active, c.node),
            None => c.enabled,
        }
    }

    // ------------------------------------------------------------------
    // Life cycle
    // ------------------------------------------------------------------

    /// `OnEnable` + `Start` of every live component of a new object.
    pub(crate) fn object_start(&mut self, content: &Content, oi: usize) {
        let pf = content.prefab(self.objects[oi].def);
        let n = pf.comps.len();
        for ci in 0..n {
            if self.comp_live(content, oi, ci) {
                self.comp_on_enable(content, oi, ci, true);
            }
        }
        // Mark started nodes first so re-entrant activation does not re-run.
        if let Some(st) = self.objects[oi].st.as_mut() {
            let mut s = 0u64;
            for k in 0..pf.nodes.len().min(64) {
                if pf.active_in_hierarchy(st.active, k as NodeIdx) {
                    s |= 1 << k;
                }
            }
            st.started = s;
        }
        for ci in 0..n {
            if self.comp_live(content, oi, ci) {
                self.comp_start(content, oi, ci);
            }
        }
    }

    /// `OnEnable` / `OnDisable` reactions.
    fn comp_on_enable(&mut self, content: &Content, oi: usize, ci: usize, on: bool) {
        let pf = content.prefab(self.objects[oi].def);
        match &pf.comps[ci].kind {
            Kind::EnableEvent(ev) if on => self.run_ev(content, oi, ev),
            Kind::DisableEvent(ev) if !on => self.run_ev(content, oi, ev),
            Kind::SoundPlayer {
                on_enable: true, ..
            } if on => self.sound_player(content, oi, ci, true),
            Kind::SoundPlayer {
                end_on_disable: true,
                ..
            } if !on => self.sound_player(content, oi, ci, false),
            _ => {}
        }
        if !on {
            // Coroutines of a deactivated object stop.
            let slot = pf.comps[ci].slot;
            match self.get_slot(oi, slot) {
                CompState::Delay { .. } => self.set_slot(oi, slot, CompState::Delay { pending: 0 }),
                CompState::Charge(mut c) => {
                    c.active = false;
                    self.set_slot(oi, slot, CompState::Charge(c));
                }
                CompState::Radar { .. } => self.set_slot(
                    oi,
                    slot,
                    CompState::Radar {
                        shots: 0,
                        ticks: 0,
                        target: 0,
                    },
                ),
                _ => {}
            }
        }
    }

    /// `Start` of one component (runs once, when first live).
    fn comp_start(&mut self, content: &Content, oi: usize, ci: usize) {
        let o = &self.objects[oi];
        let pf = content.prefab(o.def);
        let c = &pf.comps[ci];
        let node = c.node;
        match &c.kind {
            Kind::Explosion(e) if e.auto => self.comp_explode(content, oi, ci),
            Kind::DelayEvent {
                auto: true, time, ..
            } => self.set_slot(
                oi,
                c.slot,
                CompState::Delay {
                    pending: delay_ticks(*time),
                },
            ),
            Kind::StartEvent(ev) => self.run_ev(content, oi, ev),
            Kind::RemoveAfterSeconds {
                seconds,
                scale_with_scale,
            } if node == 0 => {
                // Counts down by game_dt / localScale.x when scaleWithScale.
                self.objects[oi].life = Some(*seconds);
                self.objects[oi].life_scale = *scale_with_scale;
            }
            Kind::LevelScale { on_start, .. } => {
                let scale = self.node_scale(content, oi, node);
                self.set_slot(
                    oi,
                    c.slot,
                    CompState::StartScale {
                        scale,
                        inited: true,
                    },
                );
                if *on_start && node == 0 {
                    let l = self.obj_level(content, oi);
                    let m = Self::level_scale_m(content, pf);
                    self.objects[oi].scale *= 1.0 + (l as f32 - 1.0) * m;
                }
            }
            Kind::FollowPlayer { .. } => {
                let scale = if node == 0 {
                    self.objects[oi].scale
                } else {
                    pf.nodes[node as usize].scale
                };
                self.set_slot(
                    oi,
                    c.slot,
                    CompState::StartScale {
                        scale,
                        inited: true,
                    },
                );
                if node == 0 {
                    self.follow(oi, c.slot);
                }
            }
            Kind::CapScale { min, max } if node == 0 => {
                let s = self.objects[oi].scale;
                self.objects[oi].scale = clamp(s, *min, *max);
            }
            Kind::SoundPlayer { on_start: true, .. } => self.sound_player(content, oi, ci, true),
            Kind::AddShake {
                auto: true,
                shake,
                inherit_scale,
                max,
            } => {
                let s = if *inherit_scale {
                    let s = self.node_scale(content, oi, node);
                    max.map(|m| s.min(m)).unwrap_or(s)
                } else {
                    1.0
                };
                self.events.shake(*shake * s);
            }
            Kind::PlayerInRangeTrigger {
                scale_with_range,
                range,
                ..
            } => {
                if let CompState::InRange {
                    counter,
                    done,
                    in_range,
                    target,
                    started: false,
                    ..
                } = self.get_slot(oi, c.slot)
                {
                    let r = if *scale_with_range {
                        *range * self.node_scale(content, oi, node)
                    } else {
                        *range
                    };
                    self.set_slot(
                        oi,
                        c.slot,
                        CompState::InRange {
                            counter,
                            done,
                            in_range,
                            target,
                            range: r,
                            started: true,
                        },
                    );
                }
            }
            Kind::BlockTrigger {
                delay_other_actions: true,
                ..
            } => {
                if let Some(p) = self.obj_owner(oi)
                    && matches!(self.objects[oi].kind, ObjKind::PlayerEffect { .. })
                {
                    self.players[p].block.stats.delay_other_actions = true;
                }
            }
            Kind::ThereCanOnlyBeOne(_) => self.there_can_only_be_one(content, oi),
            Kind::Thruster { pow: pw, .. } => self.thruster_start(content, oi, ci, *pw),
            Kind::Ensnare(d) => self.ensnare_start(content, oi, ci, d),
            Kind::LineEffect { .. } => self.set_slot(
                oi,
                c.slot,
                CompState::Timer {
                    t: 0.0,
                    done: false,
                },
            ),
            _ => {}
        }
    }

    /// `SetActive` on a node: OnEnable/Start or OnDisable of the components
    /// whose hierarchy state changes.
    fn set_node_active(&mut self, content: &Content, oi: usize, n: NodeIdx, on: bool) {
        if (n as usize) >= 64 {
            return;
        }
        let pf = content.prefab(self.objects[oi].def);
        let Some(before) = self.objects[oi].st.as_ref().map(|s| s.active) else {
            return;
        };
        let after = if on {
            before | (1 << n)
        } else {
            before & !(1 << n)
        };
        if after == before {
            return;
        }
        let changed: Vec<usize> = (0..pf.comps.len())
            .filter(|ci| {
                let nd = pf.comps[*ci].node;
                pf.active_in_hierarchy(before, nd) != pf.active_in_hierarchy(after, nd)
            })
            .collect();
        if let Some(st) = self.objects[oi].st.as_mut() {
            st.active = after;
        }
        self.events.push(EventKind::ObjectNodeActive {
            id: self.objects[oi].id,
            node: n,
            active: on,
        });
        for ci in changed.iter().copied() {
            let enabled = self.objects[oi]
                .st
                .as_ref()
                .is_some_and(|s| s.comp_enabled(ci as CompIdx));
            if enabled {
                self.comp_on_enable(content, oi, ci, on);
            }
        }
        if on {
            for ci in changed {
                let nd = pf.comps[ci].node as usize;
                let started = self.objects[oi]
                    .st
                    .as_ref()
                    .is_some_and(|s| nd < 64 && s.started & (1 << nd) != 0);
                if !started && self.comp_live(content, oi, ci) {
                    self.comp_start(content, oi, ci);
                }
            }
            if let Some(st) = self.objects[oi].st.as_mut() {
                for k in 0..pf.nodes.len().min(64) {
                    if pf.active_in_hierarchy(after, k as NodeIdx) {
                        st.started |= 1 << k;
                    }
                }
            }
        }
    }

    fn set_comp_enabled(&mut self, content: &Content, oi: usize, ci: usize, on: bool) {
        if ci >= 64 {
            return;
        }
        let Some(was) = self.objects[oi]
            .st
            .as_ref()
            .map(|s| s.comp_enabled(ci as CompIdx))
        else {
            return;
        };
        if was == on {
            return;
        }
        if let Some(st) = self.objects[oi].st.as_mut() {
            if on {
                st.enabled |= 1 << ci;
            } else {
                st.enabled &= !(1 << ci);
            }
        }
        let pf = content.prefab(self.objects[oi].def);
        if !self.node_active(content, oi, pf.comps[ci].node) {
            return;
        }
        self.comp_on_enable(content, oi, ci, on);
        if on {
            // First enable runs Start (e.g. Remnant's delayed range trigger).
            self.comp_start(content, oi, ci);
        } else if let Kind::RemoveAfterSeconds { .. } = pf.comps[ci].kind
            && pf.comps[ci].node == 0
        {
            self.objects[oi].life = None;
        }
    }

    /// Destroy an object (deferred removal at the end of the object pass).
    pub(crate) fn destroy_object(&mut self, content: &Content, oi: usize) {
        if self.objects[oi].dead {
            return;
        }
        let pf = content.prefab(self.objects[oi].def);
        for ci in 0..pf.comps.len() {
            if !self.comp_live(content, oi, ci) {
                continue;
            }
            match &pf.comps[ci].kind {
                Kind::DestroyEvent(ev) => self.run_ev(content, oi, ev),
                Kind::SoundPlayer {
                    end_on_destroy: true,
                    ..
                } => self.sound_player(content, oi, ci, false),
                _ => {}
            }
        }
        self.objects[oi].dead = true;
    }

    // ------------------------------------------------------------------
    // UnityEvents
    // ------------------------------------------------------------------

    pub(crate) fn run_ev(&mut self, content: &Content, oi: usize, ev: &[Call]) {
        for c in ev {
            if self.objects[oi].dead && !matches!(c.method, Method::Spawn) {
                // Calls after Destroy still run in Unity (Destroy is deferred),
                // but a destroyed object no longer matters except for spawns.
            }
            self.call(content, oi, *c);
        }
    }

    fn call(&mut self, content: &Content, oi: usize, call: Call) {
        let ci = match call.target {
            Target::Node(n) => {
                if let Method::SetActive(on) = call.method {
                    self.set_node_active(content, oi, n, on);
                }
                return;
            }
            Target::Comp(c) => c as usize,
        };
        let pf = content.prefab(self.objects[oi].def);
        let Some(comp) = pf.comps.get(ci) else { return };
        if let Method::SetEnabled(on) = call.method {
            self.set_comp_enabled(content, oi, ci, on);
            return;
        }
        let slot = comp.slot;
        match (&comp.kind, call.method) {
            (Kind::Explosion(_), Method::Explode) => self.comp_explode(content, oi, ci),
            (Kind::SpawnObjects { .. }, Method::Spawn) => self.comp_spawn(content, oi, ci),
            (Kind::DelayEvent { time, .. }, Method::Go) => self.set_slot(
                oi,
                slot,
                CompState::Delay {
                    pending: delay_ticks(*time),
                },
            ),
            (
                Kind::DealDamageToPlayer {
                    damage,
                    lethal,
                    other,
                },
                Method::Go,
            ) => self.deal_damage_to_player(content, oi, *damage, *lethal, *other),
            (Kind::RadarShot { range }, Method::Go) => self.radar_go(content, oi, ci, *range),
            (Kind::StunPlayer { own, time }, Method::Go) => self.stun_player(oi, *own, *time),
            (Kind::ResetBlock, Method::Go) => {
                if let Some(p) = self.obj_owner(oi) {
                    self.reset_block_cd(p);
                }
            }
            (Kind::SpawnStaticRemnant { source }, Method::Go) => {
                self.spawn_remnant(content, oi, *source)
            }
            (Kind::PlayerDoJump { multiplier }, Method::DoJump) => {
                if let Some(p) = self.obj_owner(oi) {
                    self.jump(content, p, true, *multiplier);
                }
            }
            (Kind::DoPlayerReload, Method::DoReload) => {
                if let Some(p) = self.obj_owner(oi) {
                    self.reload_ammo(content, p);
                }
            }
            (Kind::PlayerDoBlock, Method::DoBlock) => {
                if let Some(p) = self.obj_owner(oi)
                    && !self.players[p].dead
                {
                    self.do_block(content, p, true, true, BlockKind::Default, None, false);
                }
            }
            (Kind::ToggleStats { hp, movement_speed }, Method::TurnOn | Method::TurnOff) => {
                let on = call.method == Method::TurnOn;
                if let CompState::Toggle { on: was } = self.get_slot(oi, slot)
                    && was != on
                    && let Some(p) = self.obj_owner(oi)
                {
                    self.set_slot(oi, slot, CompState::Toggle { on });
                    self.toggle_stats(p, *hp, 1.0, *movement_speed, 1.0, 1.0, on);
                    self.events.push(EventKind::StatBoost {
                        player: p as u8,
                        object: self.objects[oi].id,
                        on,
                    });
                }
            }
            (
                Kind::CooldownWindUp {
                    increase_per_shot,
                    curve,
                },
                Method::Add,
            ) => {
                if let CompState::WindUp { value } = self.get_slot(oi, slot) {
                    let m = curve.eval(value).max(1e-6);
                    self.set_slot(
                        oi,
                        slot,
                        CompState::WindUp {
                            value: clamp(value + increase_per_shot / m, 0.0, 100.0),
                        },
                    );
                }
            }
            (Kind::CooldownWindUp { .. }, Method::Reset) => {
                self.set_slot(oi, slot, CompState::WindUp { value: 0.0 })
            }
            (Kind::CooldownCondition { ev, .. }, Method::TryEvent) => {
                if let CompState::Cooldown {
                    last,
                    cooldown,
                    base,
                } = self.get_slot(oi, slot)
                {
                    let now = self.frame as f32 * DT;
                    if now >= last + cooldown {
                        self.set_slot(
                            oi,
                            slot,
                            CompState::Cooldown {
                                last: now,
                                cooldown,
                                base,
                            },
                        );
                        self.run_ev(content, oi, ev);
                    }
                }
            }
            (
                Kind::StatsAfterDealingDamage {
                    movement_speed,
                    jump,
                    hp,
                    end,
                    ..
                },
                Method::Interrupt,
            ) => {
                if let CompState::Toggle { on: true } = self.get_slot(oi, slot)
                    && let Some(p) = self.obj_owner(oi)
                {
                    self.toggle_stats(p, *hp, *hp, *movement_speed, *jump, 1.0, false);
                    self.set_slot(oi, slot, CompState::Toggle { on: false });
                    self.run_ev(content, oi, end);
                }
            }
            (
                Kind::RemoveAfterSeconds {
                    seconds,
                    scale_with_scale,
                },
                Method::ResetCounter,
            ) => {
                // Counts down by game_dt / localScale.x when scaleWithScale.
                self.objects[oi].life = Some(*seconds);
                self.objects[oi].life_scale = *scale_with_scale;
            }
            (Kind::DisableEvent(ev), Method::OnDisable) => self.run_ev(content, oi, ev),
            (Kind::RotatingShooter { .. }, Method::Attack) => self.rotating_attack(content, oi, ci),
            (Kind::SoundPlayer { .. }, Method::PlayStart) => {
                self.sound_player(content, oi, ci, true)
            }
            (Kind::SoundPlayer { .. }, Method::PlayEnd) => {
                self.sound_player(content, oi, ci, false)
            }
            (
                Kind::AddShake {
                    shake,
                    inherit_scale,
                    max,
                    ..
                },
                Method::Shake,
            ) => {
                let s = if *inherit_scale {
                    let s = self.node_scale(content, oi, comp.node);
                    max.map(|m| s.min(m)).unwrap_or(s)
                } else {
                    1.0
                };
                self.events.shake(*shake * s);
            }
            (_, Method::ParticlePlay | Method::ParticleStop | Method::LinePlay) => {
                self.events.push(EventKind::ObjectCue {
                    id: self.objects[oi].id,
                    node: comp.node,
                    play: call.method != Method::ParticleStop,
                    line: call.method == Method::LinePlay,
                });
            }
            _ => {}
        }
    }

    fn sound_player(&mut self, content: &Content, oi: usize, ci: usize, start: bool) {
        let pf = content.prefab(self.objects[oi].def);
        let Kind::SoundPlayer {
            start: s,
            start_loop,
            end,
            ..
        } = pf.comps[ci].kind
        else {
            return;
        };
        let pos = self.obj_pos(oi);
        let key = self.objects[oi].id.wrapping_mul(64).wrapping_add(ci as u32);
        if start {
            if let Some(c) = s {
                self.events.sound(c, pos, 1.0);
            }
            if let Some(c) = start_loop {
                self.events.push(EventKind::LoopSound {
                    cue: c,
                    key,
                    pos,
                    start: true,
                });
            }
        } else {
            if let Some(c) = start_loop {
                self.events.push(EventKind::LoopSound {
                    cue: c,
                    key,
                    pos,
                    start: false,
                });
            }
            if let Some(c) = end {
                self.events.sound(c, pos, 1.0);
            }
        }
    }

    // ------------------------------------------------------------------
    // Explosion and spawning
    // ------------------------------------------------------------------

    fn comp_explode(&mut self, content: &Content, oi: usize, ci: usize) {
        let o = &self.objects[oi];
        let pf = content.prefab(o.def);
        let Kind::Explosion(e) = &pf.comps[ci].kind else {
            return;
        };
        let node = pf.comps[ci].node;
        let s = self.node_scale(content, oi, node);
        let centre = self.obj_pos(oi);
        let spawner = self.obj_owner(oi).map(|p| p as u8);
        let mut hooks = crate::objects::ExplodeHooks::default();
        for c in pf.comps.iter().filter(|c| c.node == node) {
            match c.kind {
                Kind::ExplosionOverpower { dmg_per_100hp } => hooks.overpower = Some(dmg_per_100hp),
                Kind::Implosion {
                    force,
                    clamp_dist,
                    drag,
                    time,
                } => {
                    hooks.implosion = Some((force * s, clamp_dist * s, drag, time));
                }
                _ => {}
            }
        }
        let def = o.def;
        let key = crate::projectile::name_key(&content.object(def).name);
        self.explode_with(content, centre, e, s, spawner, Some(def), key, hooks);
    }

    fn comp_spawn(&mut self, content: &Content, oi: usize, ci: usize) {
        let pf = content.prefab(self.objects[oi].def);
        let Kind::SpawnObjects {
            objects,
            transform_rotation,
            inherit_scale,
            destroy_object,
            destroy_root,
        } = &pf.comps[ci].kind
        else {
            return;
        };
        let node = pf.comps[ci].node;
        let pos = self.obj_pos(oi);
        let dir = if *transform_rotation {
            self.objects[oi].dir
        } else {
            V2::RIGHT
        };
        let scale = if *inherit_scale {
            self.node_scale(content, oi, node)
        } else {
            1.0
        };
        let spawner = self.obj_owner(oi).map(|p| p as u8);
        let level = self.obj_level(content, oi);
        for d in objects.iter() {
            let id = self.spawn_object(content, *d, pos, dir, scale, spawner, level, None);
            if let Some(st) = self.objects[oi].st.as_mut() {
                st.recent_spawn = Some(id);
            }
        }
        if *destroy_root || (*destroy_object && node == 0) {
            self.destroy_object(content, oi);
        } else if *destroy_object {
            self.set_node_active(content, oi, node, false);
        }
    }

    fn spawn_remnant(&mut self, content: &Content, oi: usize, source: Option<ObjectId>) {
        let (Some(src), Some(owner)) = (source, self.obj_owner(oi)) else {
            return;
        };
        let level = self.obj_level(content, oi);
        let pos = self.obj_pos(oi);
        let scale = 1.0 + (level as f32 - 1.0) * 0.3;
        let id = self.spawn_object(
            content,
            src,
            pos,
            V2::RIGHT,
            scale,
            Some(owner as u8),
            1,
            None,
        );
        self.events.push(EventKind::RemnantSpawned {
            player: owner as u8,
            id,
        });
    }

    // ------------------------------------------------------------------
    // Per-tick update
    // ------------------------------------------------------------------

    fn follow(&mut self, oi: usize, slot: Option<u16>) {
        let Some(owner) = self.obj_owner(oi) else {
            return;
        };
        if self.objects[oi]
            .st
            .as_ref()
            .is_some_and(|s| s.follow_stopped)
        {
            return;
        }
        let (ppos, pscale) = (self.players[owner].pos, self.players[owner].scale);
        if let CompState::StartScale {
            scale,
            inited: true,
        } = self.get_slot(oi, slot)
        {
            self.objects[oi].scale = scale * pscale;
        }
        self.objects[oi].pos = ppos;
    }

    /// One tick of an object's components (Update / FixedUpdate / coroutines).
    pub(crate) fn object_update(&mut self, content: &Content, oi: usize, game_dt: f32) {
        let def = self.objects[oi].def;
        let pf = content.prefab(def);
        if self.objects[oi].st.is_none() {
            return;
        }
        if let ObjKind::PlayerEffect { owner } = self.objects[oi].kind {
            let p = &self.players[owner as usize];
            if p.dead || p.respawning {
                return;
            }
            self.objects[oi].pos = p.pos;
        }
        for ci in 0..pf.comps.len() {
            if self.objects[oi].dead {
                return;
            }
            let c = &pf.comps[ci];
            // Coroutines run while the node is active, enabled or not.
            if let Kind::DelayEvent {
                time,
                repeating,
                ev,
                ..
            } = &c.kind
            {
                if !self.node_active(content, oi, c.node) {
                    continue;
                }
                if let CompState::Delay { pending } = self.get_slot(oi, c.slot)
                    && pending > 0
                {
                    let left = pending - 1;
                    self.set_slot(oi, c.slot, CompState::Delay { pending: left });
                    if left == 0 {
                        let enabled = self.objects[oi]
                            .st
                            .as_ref()
                            .is_some_and(|s| s.comp_enabled(ci as CompIdx));
                        if enabled {
                            self.run_ev(content, oi, ev);
                            if *repeating
                                && !self.objects[oi].dead
                                && self.get_slot(oi, c.slot) == (CompState::Delay { pending: 0 })
                            {
                                self.set_slot(
                                    oi,
                                    c.slot,
                                    CompState::Delay {
                                        pending: delay_ticks(*time),
                                    },
                                );
                            }
                        }
                    }
                }
                continue;
            }
            if !self.comp_live(content, oi, ci) {
                continue;
            }
            match &c.kind {
                Kind::FollowPlayer { other: false, .. } if c.node == 0 => self.follow(oi, c.slot),
                Kind::Saw {
                    range,
                    damage,
                    force,
                    ..
                } => self.saw_tick(content, oi, ci, *range, *damage, *force, game_dt),
                Kind::PlayerInRangeTrigger { .. } => self.in_range_tick(content, oi, ci, game_dt),
                Kind::RegenerateAfterStandStill { heal } => {
                    let node = c.node;
                    let in_range = pf.comps.iter().any(|t| {
                        t.node == node
                            && matches!(t.kind, Kind::PlayerInRangeTrigger { .. })
                            && matches!(
                                self.get_slot(oi, t.slot),
                                CompState::InRange { in_range: true, .. }
                            )
                    });
                    if in_range && let Some(p) = self.obj_owner(oi) {
                        self.heal(p, *heal);
                    }
                }
                Kind::CounterUi { time_to_fill, done } => {
                    if let CompState::Timer { t, done: false } = self.get_slot(oi, c.slot) {
                        let mut t = t + game_dt / time_to_fill;
                        t = clamp(t, -0.1 / time_to_fill, 1.0);
                        let fin = t >= 1.0;
                        self.set_slot(oi, c.slot, CompState::Timer { t, done: fin });
                        if fin {
                            self.run_ev(content, oi, done);
                        }
                    }
                }
                Kind::BlinkStep { interval } => {
                    if let CompState::Timer { t, .. } = self.get_slot(oi, c.slot) {
                        let mut t = t + game_dt;
                        if t > *interval {
                            t = 0.0;
                            self.set_slot(oi, c.slot, CompState::Timer { t, done: false });
                            if let Some(p) = self.obj_owner(oi) {
                                self.do_block(
                                    content,
                                    p,
                                    true,
                                    true,
                                    BlockKind::Default,
                                    None,
                                    false,
                                );
                            }
                        } else {
                            self.set_slot(oi, c.slot, CompState::Timer { t, done: false });
                        }
                    }
                }
                Kind::LineEffect { speed, .. } => {
                    if let CompState::Timer { t, done } = self.get_slot(oi, c.slot) {
                        self.set_slot(
                            oi,
                            c.slot,
                            CompState::Timer {
                                t: t + DT * speed,
                                done,
                            },
                        );
                    }
                }
                Kind::LineRangeEffect { dmg, knockback } => {
                    self.line_range_tick(content, oi, ci, *dmg, *knockback)
                }
                Kind::AbyssalCountdown { .. } => self.abyssal_tick(content, oi, ci, game_dt),
                Kind::StatsAfterDealingDamage {
                    duration,
                    movement_speed,
                    jump,
                    hp,
                    start,
                    end,
                } => {
                    let Some(p) = self.obj_owner(oi) else {
                        continue;
                    };
                    let active = self.players[p].since_dealt_damage < *duration;
                    if let CompState::Toggle { on } = self.get_slot(oi, c.slot)
                        && on != active
                    {
                        self.set_slot(oi, c.slot, CompState::Toggle { on: active });
                        self.toggle_stats(p, *hp, *hp, *movement_speed, *jump, 1.0, active);
                        self.events.push(EventKind::StatBoost {
                            player: p as u8,
                            object: self.objects[oi].id,
                            on: active,
                        });
                        self.run_ev(content, oi, if active { start } else { end });
                    }
                }
                Kind::StatsWhenFullHp {
                    health_multiplier,
                    size_multiplier,
                    threshold,
                } => {
                    let Some(p) = self.obj_owner(oi) else {
                        continue;
                    };
                    let pl = &self.players[p];
                    let on = pl.health / pl.stats.max_health >= *threshold;
                    if let CompState::Toggle { on: was } = self.get_slot(oi, c.slot)
                        && was != on
                    {
                        self.set_slot(oi, c.slot, CompState::Toggle { on });
                        self.toggle_stats(
                            p,
                            *health_multiplier,
                            *health_multiplier,
                            1.0,
                            1.0,
                            *size_multiplier,
                            on,
                        );
                        let pos = self.players[p].pos;
                        self.events.sound(
                            if on {
                                "SFX_Passive_Pristine_Perserverance_Grow_SE"
                            } else {
                                "SFX_Passive_Pristine_Perserverance_Shrink_SE"
                            },
                            pos,
                            1.0,
                        );
                        self.events.push(EventKind::StatBoost {
                            player: p as u8,
                            object: self.objects[oi].id,
                            on,
                        });
                    }
                }
                Kind::Chase { on, off } => {
                    let Some(p) = self.obj_owner(oi) else {
                        continue;
                    };
                    let pos = self.players[p].pos;
                    let dir = self.players[p].controls.direction;
                    let active = !dir.is_zero()
                        && self
                            .closest_enemy(content, pos, p, true)
                            .is_some_and(|t| (self.players[t].pos - pos).angle_deg(dir) <= 70.0);
                    if let CompState::Toggle { on: was } = self.get_slot(oi, c.slot)
                        && was != active
                    {
                        self.set_slot(oi, c.slot, CompState::Toggle { on: active });
                        self.run_ev(content, oi, if active { on } else { off });
                    }
                }
                Kind::DuringReloadTrigger { ev, start, end } => {
                    let Some(p) = self.obj_owner(oi) else {
                        continue;
                    };
                    let reloading = self.players[p].gun.reloading;
                    if let CompState::Reloading { was } = self.get_slot(oi, c.slot) {
                        self.set_slot(oi, c.slot, CompState::Reloading { was: reloading });
                        if reloading {
                            if !was {
                                self.run_ev(content, oi, start);
                            }
                            self.run_ev(content, oi, ev);
                        } else if was {
                            self.run_ev(content, oi, end);
                        }
                    }
                }
                Kind::CooldownWindUp { curve, .. } => {
                    // Update: cooldown = start / multiplier(value).
                    if let CompState::WindUp { value } = self.get_slot(oi, c.slot) {
                        let m = curve.eval(value).max(1e-6);
                        let node = c.node;
                        for t in pf.comps.iter().filter(|t| t.node == node) {
                            if let Kind::CooldownCondition { .. } = t.kind
                                && let CompState::Cooldown { last, base, .. } =
                                    self.get_slot(oi, t.slot)
                            {
                                self.set_slot(
                                    oi,
                                    t.slot,
                                    CompState::Cooldown {
                                        last,
                                        cooldown: base / m,
                                        base,
                                    },
                                );
                            }
                        }
                    }
                }
                Kind::SpawnObjectOnDealDamage { .. } => {
                    if let CompState::Refresh { acc, since } = self.get_slot(oi, c.slot) {
                        self.set_slot(
                            oi,
                            c.slot,
                            CompState::Refresh {
                                acc,
                                since: since + DT,
                            },
                        );
                    }
                }
                Kind::Thruster {
                    physics_object_m, ..
                } => self.thruster_tick(content, oi, ci, *physics_object_m),
                Kind::Ensnare(d) => self.ensnare_tick(oi, ci, d),
                Kind::ShieldCharge(d) => self.charge_tick(content, oi, ci, d),
                Kind::Teleport { .. } => {
                    if let CompState::Teleport { pending, to } = self.get_slot(oi, c.slot)
                        && pending > 0
                    {
                        let left = pending - 1;
                        self.set_slot(oi, c.slot, CompState::Teleport { pending: left, to });
                        if left == 0
                            && let Some(p) = self.obj_owner(oi)
                        {
                            self.teleport_player(p, to);
                        }
                    }
                }
                Kind::RadarShot { .. } => self.radar_tick(content, oi, ci),
                Kind::RotatingShooter { .. } => {
                    if let CompState::Shooter { reps, ticks: t } = self.get_slot(oi, c.slot)
                        && reps > 0
                    {
                        if t <= 1 {
                            self.set_slot(
                                oi,
                                c.slot,
                                CompState::Shooter {
                                    reps: reps - 1,
                                    ticks: super::addons::real_ticks(0.1),
                                },
                            );
                            self.rotating_fire_ring(content, oi, ci);
                        } else {
                            self.set_slot(oi, c.slot, CompState::Shooter { reps, ticks: t - 1 });
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // ------------------------------------------------------------------
    // Player hooks
    // ------------------------------------------------------------------

    /// Dispatch a player hook to the components of the player's effects.
    pub(crate) fn effects_hook(&mut self, content: &Content, p: usize, hook: Hook) {
        match hook {
            Hook::Jump
            | Hook::TouchGround { .. }
            | Hook::TouchWall { .. }
            | Hook::WasDealtDamage { .. }
            | Hook::DelayedRevive
            | Hook::RoundStart
            | Hook::PointEnd
            | Hook::Tick => return,
            _ => {}
        }
        let mut k = 0;
        while k < self.players[p].effects.len() {
            let id = self.players[p].effects[k];
            k += 1;
            let Some(oi) = self.obj_index(id) else {
                continue;
            };
            let pf = content.prefab(self.objects[oi].def);
            for ci in 0..pf.comps.len() {
                if matches!(hook, Hook::Death) {
                    if self.node_active(content, oi, pf.comps[ci].node) {
                        self.comp_on_enable_coroutines_stop(oi, ci, content);
                    }
                    continue;
                }
                if !self.comp_live(content, oi, ci) {
                    continue;
                }
                self.comp_hook(content, oi, ci, p, hook);
            }
        }
    }

    fn comp_on_enable_coroutines_stop(&mut self, oi: usize, ci: usize, content: &Content) {
        let pf = content.prefab(self.objects[oi].def);
        let slot = pf.comps[ci].slot;
        match self.get_slot(oi, slot) {
            CompState::Delay { .. } => self.set_slot(oi, slot, CompState::Delay { pending: 0 }),
            CompState::Charge(mut c) => {
                c.active = false;
                c.wait = 0;
                self.set_slot(oi, slot, CompState::Charge(c));
            }
            CompState::Radar { .. } => self.set_slot(
                oi,
                slot,
                CompState::Radar {
                    shots: 0,
                    ticks: 0,
                    target: 0,
                },
            ),
            CompState::Teleport { to, .. } => {
                self.set_slot(oi, slot, CompState::Teleport { pending: 0, to })
            }
            _ => {}
        }
    }

    fn comp_hook(&mut self, content: &Content, oi: usize, ci: usize, p: usize, hook: Hook) {
        let pf = content.prefab(self.objects[oi].def);
        let c = &pf.comps[ci];
        let slot = c.slot;
        match (&c.kind, hook) {
            (
                Kind::BlockTrigger {
                    blacklisted,
                    super_first,
                    first_delays,
                    early,
                    block,
                    ..
                },
                Hook::Block { phase, kind },
            ) => {
                if block_kind_index(kind) == *blacklisted {
                    return;
                }
                let ev = match phase {
                    BlockPhase::SuperFirst => super_first,
                    BlockPhase::FirstDelaysOthers => first_delays,
                    BlockPhase::Early => early,
                    BlockPhase::Block => block,
                };
                self.run_ev(content, oi, ev);
            }
            (Kind::BlockTrigger { success, .. }, Hook::BlockedProjectile { .. }) => {
                self.run_ev(content, oi, success)
            }
            (Kind::BlockTrigger { recharge, .. }, Hook::BlockRecharged) => {
                self.run_ev(content, oi, recharge)
            }
            (
                Kind::Teleport { distance, mask },
                Hook::Block {
                    phase: BlockPhase::SuperFirst,
                    kind,
                },
            ) => self.teleport_block(content, oi, ci, p, *distance, *mask, kind),
            (
                Kind::ShieldCharge(d),
                Hook::Block {
                    phase: BlockPhase::SuperFirst,
                    kind,
                },
            ) if kind != BlockKind::ShieldCharge => self.charge_start(content, oi, ci, p, d, kind),
            (Kind::ShieldCharge(d), Hook::PlayerCollision { other, .. }) => {
                self.charge_collide(content, oi, ci, p, other as usize, d)
            }
            (Kind::ShieldCharge(_), Hook::Revive { .. }) => {
                if let CompState::Charge(mut s) = self.get_slot(oi, slot) {
                    s.active = false;
                    s.wait = 0;
                    self.set_slot(oi, slot, CompState::Charge(s));
                }
            }
            (
                Kind::Empower { .. },
                Hook::Block {
                    phase: BlockPhase::Block,
                    kind,
                },
            ) => {
                if !matches!(
                    kind,
                    BlockKind::Echo | BlockKind::Empower | BlockKind::ShieldCharge
                ) {
                    self.set_slot(oi, slot, CompState::Empower { armed: true });
                    let pos = self.players[p].pos;
                    self.events.sound("SFX_Block_Empower_Spawn_SE", pos, 1.0);
                    self.events
                        .push(EventKind::EmpowerArmed { player: p as u8 });
                }
            }
            (
                Kind::Empower {
                    dmg_multiplier,
                    speed_multiplier,
                    add_object,
                },
                Hook::ShootProjectile { projectile },
            ) => {
                if let CompState::Empower { armed } = self.get_slot(oi, slot) {
                    if armed {
                        self.empower_projectile(
                            content,
                            p,
                            projectile,
                            *dmg_multiplier,
                            *speed_multiplier,
                            *add_object,
                        );
                    }
                    self.set_slot(oi, slot, CompState::Empower { armed: false });
                }
            }
            (Kind::Empower { .. }, Hook::Revive { .. }) => {
                self.set_slot(oi, slot, CompState::Empower { armed: false })
            }
            (
                Kind::EmpowerStopFollow,
                Hook::Block {
                    phase: BlockPhase::Block,
                    kind: BlockKind::Empower,
                },
            ) => {
                if let Some(r) = self.objects[oi].st.as_ref().and_then(|s| s.recent_spawn)
                    && let Some(ri) = self.obj_index(r)
                    && let Some(st) = self.objects[ri].st.as_mut()
                {
                    st.follow_stopped = true;
                }
            }
            (Kind::AbyssalCountdown { .. }, Hook::Revive { .. }) => {
                self.abyssal_reset(content, oi, ci)
            }
            (Kind::RespawnEvent(ev), Hook::Revive { .. }) => self.run_ev(content, oi, ev),
            (Kind::DealtDamageTrigger(ev), Hook::DealtDamage { .. }) => {
                self.run_ev(content, oi, ev)
            }
            (Kind::ReloadTrigger { out_of_ammo, .. }, Hook::OutOfAmmo { .. }) => {
                self.run_ev(content, oi, out_of_ammo)
            }
            (Kind::ReloadTrigger { reload_done, .. }, Hook::ReloadDone { .. }) => {
                self.run_ev(content, oi, reload_done)
            }
            (
                Kind::SpawnObjectOnDealDamage {
                    damage_needed,
                    cd,
                    allow_self,
                    ev,
                },
                Hook::DealtDamage {
                    damage,
                    self_damage,
                    ..
                },
            ) => {
                if self_damage && !allow_self {
                    return;
                }
                if let CompState::Refresh { acc, since } = self.get_slot(oi, slot) {
                    let acc = acc + damage.len();
                    if acc > *damage_needed && since > *cd {
                        self.set_slot(
                            oi,
                            slot,
                            CompState::Refresh {
                                acc: 0.0,
                                since: 0.0,
                            },
                        );
                        self.run_ev(content, oi, ev);
                    } else {
                        self.set_slot(oi, slot, CompState::Refresh { acc, since });
                    }
                }
            }
            (
                Kind::AttackTrigger {
                    every_shot: false,
                    ev,
                },
                Hook::Attack,
            ) => self.run_ev(content, oi, ev),
            (
                Kind::AttackTrigger {
                    every_shot: true,
                    ev,
                },
                Hook::ShootProjectile { .. },
            ) => self.run_ev(content, oi, ev),
            _ => {}
        }
    }

    // ------------------------------------------------------------------
    // Player effects (AddObjectToPlayer, cards.md 5.2)
    // ------------------------------------------------------------------

    /// Instantiate a card's effect object on a player, or level up the
    /// existing one of the same prefab when it has an `AttackLevel`.
    pub fn add_player_effect(&mut self, content: &Content, p: usize, def: ObjectId) {
        let pf = content.prefab(def);
        if pf.attack_level.is_some() {
            for k in 0..self.players[p].effects.len() {
                let id = self.players[p].effects[k];
                let Some(oi) = self.obj_index(id) else {
                    continue;
                };
                if self.objects[oi].def == def {
                    self.level_up(content, oi);
                    return;
                }
            }
        }
        let pos = self.players[p].pos;
        let scale = 1.0;
        let id = self.alloc_id();
        let d = content.object(def);
        let obj = crate::objects::SpawnedObject {
            id,
            def,
            pos,
            dir: V2::RIGHT,
            scale: pf.nodes[0].scale * scale,
            spawner: Some(p as u8),
            level: 1,
            age: 0.0,
            life: None,
            life_scale: false,
            attach: Attach::None,
            kind: ObjKind::PlayerEffect { owner: p as u8 },
            st: new_state(pf).or_else(|| {
                Some(Box::new(ObjState {
                    active: pf.initial_active(),
                    enabled: pf.initial_enabled(),
                    started: 0,
                    comps: vec![],
                    recent_spawn: None,
                    follow_stopped: false,
                }))
            }),
            dead: false,
        };
        let _ = d;
        self.objects.push(obj);
        self.players[p].effects.push(id);
        self.events.push(EventKind::EffectAdded {
            player: p as u8,
            id,
            def,
        });
        let oi = self.objects.len() - 1;
        self.object_start(content, oi);
    }

    fn level_up(&mut self, content: &Content, oi: usize) {
        let pf = content.prefab(self.objects[oi].def);
        let Some(ali) = pf.attack_level else { return };
        let slot = pf.comps[ali as usize].slot;
        let CompState::Level { level } = self.get_slot(oi, slot) else {
            return;
        };
        let level = level + 1;
        self.set_slot(oi, slot, CompState::Level { level });
        let m = Self::level_scale_m(content, pf);
        for c in pf.comps.iter() {
            if let Kind::LevelScale {
                on_level_up: true, ..
            } = c.kind
                && c.node == 0
                && let CompState::StartScale { scale, .. } = self.get_slot(oi, c.slot)
            {
                self.objects[oi].scale = scale * (1.0 + (level as f32 - 1.0) * m);
            }
        }
        if let Some(p) = self.obj_owner(oi) {
            self.events.push(EventKind::EffectLevelUp {
                player: p as u8,
                id: self.objects[oi].id,
                level,
            });
        }
    }

    /// Remove every effect object of a player (stat reset).
    pub(crate) fn clear_player_effects(&mut self, content: &Content, p: usize) {
        let ids = std::mem::take(&mut self.players[p].effects);
        for id in ids {
            if let Some(oi) = self.obj_index(id) {
                self.destroy_object(content, oi);
            }
        }
        self.players[p].pulls.clear();
    }

    // ------------------------------------------------------------------
    // Stat helpers
    // ------------------------------------------------------------------

    /// Multiply (on) or divide (off) health, max health, movement speed,
    /// jump and size, then `ConfigureMassAndSize` (cards.md 6.9-6.11).
    #[allow(clippy::too_many_arguments)]
    fn toggle_stats(
        &mut self,
        p: usize,
        hp: f32,
        max_hp: f32,
        ms: f32,
        jump: f32,
        size: f32,
        on: bool,
    ) {
        let pl = &mut self.players[p];
        let f = |x: f32| if on { x } else { 1.0 / x };
        pl.health *= f(hp);
        pl.stats.max_health *= f(max_hp);
        pl.stats.movement_speed *= f(ms);
        pl.stats.jump *= f(jump);
        pl.stats.size_multiplier *= f(size);
        pl.configure_mass_and_size();
    }

    fn reset_block_cd(&mut self, p: usize) {
        let b = &mut self.players[p].block;
        b.active = true;
        b.counter = b.stats.cooldown() + 1.0;
        self.events
            .push(EventKind::BlockRecharged { player: p as u8 });
    }

    /// Move a player (teleport rule: zero velocity, ignore walls 2 ticks,
    /// grounded timer reset).
    fn teleport_player(&mut self, p: usize, to: V2) {
        let from = self.players[p].pos;
        self.teleport(p, to);
        self.players[p].since_grounded = 0.0;
        self.events.push(EventKind::Teleported {
            player: p as u8,
            from,
            to,
        });
    }

    /// `PlayerVelocity.AddForce` (no block check): Force adds `0.02 f / m`.
    pub(crate) fn add_player_force(&mut self, p: usize, f: V2, mode: ForceMode) {
        let pl = &mut self.players[p];
        if !pl.playing || pl.dead || pl.respawning || pl.kinematic {
            return;
        }
        match mode {
            ForceMode::Impulse => pl.vel += f / pl.mass,
            ForceMode::Force => pl.vel += f * 0.02 / pl.mass,
        }
    }

    // ------------------------------------------------------------------
    // Behaviours
    // ------------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    fn saw_tick(
        &mut self,
        content: &Content,
        oi: usize,
        ci: usize,
        range: f32,
        damage: f32,
        force: f32,
        game_dt: f32,
    ) {
        let pf = content.prefab(self.objects[oi].def);
        let s = self.node_scale(content, oi, pf.comps[ci].node);
        let pos = self.obj_pos(oi);
        let owner = self.obj_owner(oi);
        // The last player in list order within range (cards.md 6.2).
        let mut target = None;
        for (i, p) in self.players.iter().enumerate() {
            if Some(i) == owner || !p.hittable() || !p.playing {
                continue;
            }
            if (p.pos - pos).len() < range * s {
                target = Some(i);
            }
        }
        let Some(t) = target else { return };
        if !self.can_see(content, pos, t) {
            return;
        }
        let tp = self.players[t].pos;
        let dist = (tp - pos).len();
        let dir = (tp - pos).normalized();
        if damage != 0.0 {
            self.take_damage(
                content,
                t,
                dir * (game_dt * damage),
                tp,
                owner.map(|o| o as u8),
                true,
                false,
            );
        }
        if force != 0.0 {
            let fm = 1.0 / self.players[t].scale;
            let f = clamp(1.0 - dist / range, 0.0, 1.0) * fm;
            let fd = v2(dir.x, dir.y * 0.5);
            self.add_player_force(t, fd * (s * f * game_dt * force), ForceMode::Force);
            self.take_force(
                t,
                fd * (f * 0.0005 * game_dt * force),
                ForceMode::Impulse,
                false,
                false,
                0.0,
            );
        }
        self.events.push(EventKind::SawHit { pos: tp, dir });
        if self.frame.is_multiple_of(8) {
            self.events.sound(crate::events::cue::SAW_DAMAGE, tp, 1.0);
        }
    }

    fn in_range_tick(&mut self, content: &Content, oi: usize, ci: usize, game_dt: f32) {
        let pf = content.prefab(self.objects[oi].def);
        let Kind::PlayerInRangeTrigger {
            any,
            cooldown,
            repeating,
            ev,
            ..
        } = &pf.comps[ci].kind
        else {
            return;
        };
        let slot = pf.comps[ci].slot;
        let CompState::InRange {
            counter,
            done,
            range,
            started,
            ..
        } = self.get_slot(oi, slot)
        else {
            return;
        };
        let counter = counter + game_dt;
        let mut st = CompState::InRange {
            counter,
            done,
            in_range: false,
            target: None,
            range,
            started,
        };
        self.set_slot(oi, slot, st);
        if done {
            return;
        }
        let pos = self.obj_pos(oi);
        let owner = self.obj_owner(oi);
        let target = if *any {
            self.closest_player(content, pos, false)
        } else {
            owner.and_then(|o| self.closest_enemy(content, pos, o, false))
        };
        let Some(t) = target else { return };
        let tp = self.players[t].pos;
        if !self.can_see(content, pos, t)
            || (tp - pos).len() >= range * self.root_scale(oi)
            || counter < *cooldown
        {
            return;
        }
        st = CompState::InRange {
            counter: 0.0,
            done: !repeating,
            in_range: true,
            target: Some(t as u8),
            range,
            started,
        };
        self.set_slot(oi, slot, st);
        if let Some(o) = owner {
            self.events.push(EventKind::RangeTriggered {
                id: self.objects[oi].id,
                player: o as u8,
                target: t as u8,
            });
        }
        self.run_ev(content, oi, ev);
        // PlayerInRangeSlow listens to the trigger (Chilling Presence).
        let level = self.obj_level(content, oi);
        for c in pf.comps.iter() {
            if let Kind::PlayerInRangeSlow { amount, max } = c.kind {
                self.add_slow_additive(
                    t,
                    amount * level as f32,
                    max + (level as f32 - 1.0) * 0.25,
                    false,
                );
            }
        }
    }

    fn deal_damage_to_player(
        &mut self,
        content: &Content,
        oi: usize,
        damage: f32,
        lethal: bool,
        other: bool,
    ) {
        let Some(owner) = self.obj_owner(oi) else {
            return;
        };
        let target = if other {
            // The in-range trigger's current target, else the closest enemy
            // (cards.md 5.5 Clarified).
            let pf = content.prefab(self.objects[oi].def);
            let t = pf
                .comps
                .iter()
                .find_map(|c| match self.get_slot(oi, c.slot) {
                    CompState::InRange {
                        target: Some(t), ..
                    } => Some(t as usize),
                    _ => None,
                });
            t.or_else(|| {
                let pos = self.players[owner].pos;
                self.closest_enemy(content, pos, owner, false)
            })
        } else {
            Some(owner)
        };
        let Some(t) = target else { return };
        let pos = self.obj_pos(oi);
        self.take_damage(
            content,
            t,
            V2::UP * damage,
            pos,
            Some(owner as u8),
            lethal,
            false,
        );
        if other {
            self.events.push(EventKind::LifeDrain {
                from: t as u8,
                to: owner as u8,
                amount: damage,
            });
        }
    }

    fn stun_player(&mut self, oi: usize, own: bool, time: f32) {
        let parent = match self.objects[oi].attach {
            Attach::Player(pl, _) => Some(pl as usize),
            _ => match self.objects[oi].kind {
                ObjKind::PlayerEffect { owner } => Some(owner as usize),
                ObjKind::Spawned => None,
            },
        };
        let Some(pp) = parent else { return };
        let t = if own {
            Some(pp)
        } else {
            let pos = self.players[pp].pos;
            let mut best: Option<(f32, usize)> = None;
            let team = self.players[pp].team;
            for (i, p) in self.players.iter().enumerate() {
                if p.team != team && p.hittable() {
                    let d = (p.pos - pos).len_sq();
                    if best.is_none_or(|b| d < b.0) {
                        best = Some((d, i));
                    }
                }
            }
            best.map(|b| b.1)
        };
        if let Some(t) = t
            && self.players[t].hittable()
        {
            self.add_stun(t, time);
        }
    }

    fn there_can_only_be_one(&mut self, content: &Content, oi: usize) {
        let Attach::Player(pl, _) = self.objects[oi].attach else {
            return;
        };
        let me = self.objects[oi].id;
        let mut found = false;
        for k in 0..self.objects.len() {
            let o = &self.objects[k];
            if o.dead || o.id == me || !matches!(o.attach, Attach::Player(q, _) if q == pl) {
                continue;
            }
            let pf = content.prefab(o.def);
            let poke: Option<&Ev> = pf.comps.iter().find_map(|c| match &c.kind {
                Kind::ThereCanOnlyBeOne(ev) => Some(ev),
                _ => None,
            });
            if let Some(ev) = poke {
                found = true;
                self.run_ev(content, k, ev);
            }
        }
        if found {
            self.destroy_object(content, oi);
        }
    }

    fn line_range_tick(
        &mut self,
        content: &Content,
        oi: usize,
        ci: usize,
        dmg: f32,
        knockback: f32,
    ) {
        let pf = content.prefab(self.objects[oi].def);
        let slot = pf.comps[ci].slot;
        if let CompState::Done { done: true } = self.get_slot(oi, slot) {
            return;
        }
        let Some(owner) = self.obj_owner(oi) else {
            return;
        };
        let pos = self.obj_pos(oi);
        let Some(t) = self.closest_enemy(content, pos, owner, true) else {
            return;
        };
        // Ring radius of the child LineEffect (cards.md 6.14).
        let mut radius = 0.0;
        for c in pf.comps.iter() {
            if let Kind::LineEffect {
                radius: r,
                radius_over_time,
                ..
            } = &c.kind
                && let CompState::Timer { t, .. } = self.get_slot(oi, c.slot)
            {
                radius = r * self.root_scale(oi) * radius_over_time.eval(t);
            }
        }
        let tp = self.players[t].pos;
        let d = (tp - pos).len();
        if d < radius + 2.0 && d > radius - 2.0 {
            self.set_slot(oi, slot, CompState::Done { done: true });
            let s = self.objects[oi].scale;
            let dir = (tp - pos).normalized();
            self.call_take_damage(content, t, dir * (dmg * s), tp, Some(owner as u8), true);
            if !self.players[t].is_blocking() {
                self.take_force(
                    t,
                    dir * (knockback * s),
                    ForceMode::Impulse,
                    false,
                    false,
                    0.0,
                );
            }
        }
    }

    fn thruster_start(&mut self, content: &Content, oi: usize, ci: usize, pw: f32) {
        let pf = content.prefab(self.objects[oi].def);
        let Kind::Thruster { force, .. } = pf.comps[ci].kind else {
            return;
        };
        let s = self.node_scale(content, oi, pf.comps[ci].node);
        let dir = self.objects[oi].dir;
        let factor = match self.objects[oi].attach {
            Attach::Player(..) => 1.0,
            Attach::Prop(..) => 0.5,
            Attach::None => 0.2,
        };
        if let Some(l) = self.objects[oi].life.as_mut() {
            *l *= factor;
        }
        self.set_slot(
            oi,
            pf.comps[ci].slot,
            CompState::Thrust {
                dir,
                force: force * pow(s, pw),
                started: !matches!(self.objects[oi].attach, Attach::None),
            },
        );
    }

    fn thruster_tick(&mut self, content: &Content, oi: usize, ci: usize, physics_m: f32) {
        let pf = content.prefab(self.objects[oi].def);
        let CompState::Thrust {
            dir,
            force,
            started: true,
        } = self.get_slot(oi, pf.comps[ci].slot)
        else {
            return;
        };
        match self.objects[oi].attach {
            Attach::Player(pl, _) => self.take_force(
                pl as usize,
                dir * force,
                ForceMode::Force,
                false,
                false,
                0.0,
            ),
            Attach::Prop(pr, _) => {
                let at = self.objects[oi].pos;
                if (pr as usize) < self.physics.props.len() {
                    self.physics
                        .add_force_at(pr as usize, dir * (force * physics_m), at);
                }
            }
            Attach::None => {}
        }
    }

    fn ensnare_start(&mut self, content: &Content, oi: usize, ci: usize, d: &EnsnareDef) {
        let pf = content.prefab(self.objects[oi].def);
        let s = self.node_scale(content, oi, pf.comps[ci].node);
        let range = d.range * (1.0 + s) * 0.5;
        let pos = self.obj_pos(oi);
        let Some(t) = self.closest_player(content, pos, true) else {
            return;
        };
        if (self.players[t].pos - pos).len() >= range {
            return;
        }
        let start = if d.start_pos_is_my_pos {
            pos
        } else {
            self.players[t].pos
        };
        self.set_slot(
            oi,
            pf.comps[ci].slot,
            CompState::Ensnare {
                target: Some(t as u8),
                start,
                elapsed: 0.0,
                scale: s,
                snapped: false,
            },
        );
        let tp = self.players[t].pos;
        self.events
            .sound("SFX_Character_Effect_Ensnare_SE", tp, 1.0);
        self.events.push(EventKind::Ensnared {
            id: self.objects[oi].id,
            target: t as u8,
            start,
        });
    }

    fn ensnare_tick(&mut self, oi: usize, ci: usize, d: &EnsnareDef) {
        let _ = ci;
        let slot = {
            // Slot lookup without content: the Ensnare state is unique per object.
            let st = self.objects[oi].st.as_ref();
            st.and_then(|s| {
                s.comps
                    .iter()
                    .position(|c| matches!(c, CompState::Ensnare { .. }))
            })
        };
        let Some(slot) = slot.map(|s| s as u16) else {
            return;
        };
        let CompState::Ensnare {
            target: Some(t),
            start,
            elapsed,
            scale,
            snapped,
        } = self.get_slot(oi, Some(slot))
        else {
            return;
        };
        let t = t as usize;
        let mut counter = elapsed;
        if counter >= d.line_end || !self.players[t].hittable() {
            return;
        }
        let tp = self.players[t].pos;
        self.players[t].since_grounded = 0.0;
        self.players[t].vel *= pow(d.drag, scale);
        self.add_player_force(
            t,
            (start - tp).clamp_len(60.0) * (scale * d.force),
            ForceMode::Force,
        );
        let mut snapped = snapped;
        if !snapped && (start - tp).len() > d.break_distance {
            snapped = true;
            counter = counter.max(d.snap_cap_line_time);
            self.events
                .sound("SFX_Character_Jump_Ensnare_Break_SE", tp, 1.0);
            self.events.push(EventKind::EnsnareSnapped {
                id: self.objects[oi].id,
            });
        }
        counter += DT * d.line_speed;
        self.set_slot(
            oi,
            Some(slot),
            CompState::Ensnare {
                target: Some(t as u8),
                start,
                elapsed: counter,
                scale,
                snapped,
            },
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn teleport_block(
        &mut self,
        content: &Content,
        oi: usize,
        ci: usize,
        p: usize,
        distance: f32,
        mask: u32,
        kind: BlockKind,
    ) {
        let pf = content.prefab(self.objects[oi].def);
        let slot = pf.comps[ci].slot;
        let start = self.players[p].pos;
        if kind == BlockKind::Empower {
            // Wait one frame, then go to the empowered bullet's impact point.
            self.set_slot(
                oi,
                slot,
                CompState::Teleport {
                    pending: 1,
                    to: start,
                },
            );
            return;
        }
        let level = self.obj_level(content, oi) as f32;
        let step = distance * level / 10.0;
        let aim = self.players[p].aim;
        let mut last_free = start;
        let mut q = start;
        {
            // `OverlapCircle(q, 0.5)` without a layer mask (the serialized
            // `mask` is never read): statics, animated colliders, props,
            // debris and every player's circle, the teleporter's own
            // included, block a sample (cards.md 11, Clarified 6.3).
            let _ = mask;
            let circles = self.player_circles();
            let scene = self.scene(content, &circles);
            let mut hits = Vec::new();
            for _ in 0..10 {
                q += aim * step;
                scene.overlaps(q, 0.5, !0, None, &mut hits);
                if hits.is_empty() && !self.debris_overlaps(q, 0.5) {
                    last_free = q;
                }
            }
        }
        self.teleport_player(p, last_free);
    }

    /// Whether a debris piece of a destroyed box overlaps the circle.
    fn debris_overlaps(&self, c: V2, r: f32) -> bool {
        self.physics.pieces.iter().any(|pc| {
            let shape = crate::geom::Shape::Obb {
                c: pc.pos,
                axis: v2(cos(pc.angle), sin(pc.angle)),
                half: pc.half * pc.scale(),
            };
            crate::objects::shape_overlaps_circle(&shape, c, r)
        })
    }

    fn charge_start(
        &mut self,
        content: &Content,
        oi: usize,
        ci: usize,
        p: usize,
        d: &ShieldChargeDef,
        kind: BlockKind,
    ) {
        let pf = content.prefab(self.objects[oi].def);
        let slot = pf.comps[ci].slot;
        let l = (self.obj_level(content, oi) - 1).max(0) as f32;
        let used_time = d.time + l * d.time_per_level;
        let mut st = ChargeState {
            active: true,
            c: 0.0,
            used_time,
            block_time: used_time,
            dir: self.players[p].aim,
            force: d.force + l * d.force_per_level,
            damage: d.damage + l * d.damage_per_level,
            knockback: d.knockback + l * d.knockback_per_level,
            hits: 0,
            cancelled: false,
            wait: 0,
            from: self.players[p].pos,
        };
        if kind == BlockKind::Empower {
            st.wait = 1;
        }
        self.set_slot(oi, slot, CompState::Charge(st));
        // Extra charge blocks with levels (cards.md 6.4).
        let lv = l as i32;
        let level = lv + 1;
        for i in 0..lv {
            let di = d.time * 0.1 + l * d.time * 0.15 + (i + 1) as f32 * d.time / level as f32;
            self.players[p].pending_blocks.push(PendingBlock {
                ticks: ticks(di).max(1),
                first: false,
                dont_set_cd: true,
                kind: BlockKind::ShieldCharge,
                only_effects: false,
                continuation: false,
                at: None,
            });
        }
        let pos = self.players[p].pos;
        self.events.sound("SFX_Block_Shield_Charge_SE", pos, 1.0);
        self.events.push(EventKind::ShieldChargeStart {
            player: p as u8,
            dir: st.dir,
        });
    }

    fn charge_tick(&mut self, content: &Content, oi: usize, ci: usize, d: &ShieldChargeDef) {
        let pf = content.prefab(self.objects[oi].def);
        let slot = pf.comps[ci].slot;
        let CompState::Charge(mut st) = self.get_slot(oi, slot) else {
            return;
        };
        if !st.active {
            if st.block_time >= 0.0 {
                st.block_time -= DT * self.time.scale();
                self.set_slot(oi, slot, CompState::Charge(st));
            }
            return;
        }
        let Some(p) = self.obj_owner(oi) else { return };
        if st.wait > 0 {
            st.wait -= 1;
            if st.wait == 0 {
                let now = self.players[p].pos;
                st.dir = (st.from - now).normalized();
            }
            self.set_slot(oi, slot, CompState::Charge(st));
            return;
        }
        let ts = self.time.scale();
        st.block_time -= DT * ts;
        if !st.cancelled {
            let f = d.force_curve.eval(st.c) * st.force;
            self.take_force(p, st.dir * f, ForceMode::Force, true, true, 0.0);
            let v = self.players[p].vel;
            self.take_force(p, -v * (d.drag * 0.014), ForceMode::Force, true, true, 0.0);
            self.players[p].since_grounded = 0.0;
        }
        st.c += 0.014 / st.used_time.max(1e-6);
        if st.c >= 1.0 {
            st.active = false;
            self.set_slot(oi, slot, CompState::Charge(st));
            self.events
                .push(EventKind::ShieldChargeEnd { player: p as u8 });
            self.do_block(
                content,
                p,
                false,
                true,
                BlockKind::ShieldCharge,
                None,
                false,
            );
            return;
        }
        self.set_slot(oi, slot, CompState::Charge(st));
    }

    pub(crate) fn charge_collide(
        &mut self,
        content: &Content,
        oi: usize,
        ci: usize,
        p: usize,
        other: usize,
        d: &ShieldChargeDef,
    ) {
        let pf = content.prefab(self.objects[oi].def);
        let slot = pf.comps[ci].slot;
        let CompState::Charge(mut st) = self.get_slot(oi, slot) else {
            return;
        };
        // Collisions count while `blockTime` (game clock) lasts, which
        // outlives the force loop (real clock) a little.
        if st.wait > 0 || st.block_time < 0.0 || other >= 16 || st.hits & (1 << other) != 0 {
            return;
        }
        st.hits |= 1 << other;
        st.cancelled = true;
        self.set_slot(oi, slot, CompState::Charge(st));
        let pos = self.players[other].pos;
        self.take_damage(
            content,
            other,
            st.dir * st.damage,
            pos,
            Some(p as u8),
            true,
            false,
        );
        self.take_force(
            other,
            st.dir * st.knockback,
            ForceMode::Impulse,
            false,
            false,
            0.0,
        );
        // Recoil: `knockBack` respects mass, only `stopForce` ignores it
        // (cards.md 6.4, Clarified).
        self.take_force(
            p,
            -st.dir * d.knockback,
            ForceMode::Impulse,
            false,
            true,
            0.0,
        );
        self.take_force(
            p,
            -st.dir * d.stop_force,
            ForceMode::Impulse,
            true,
            true,
            0.0,
        );
        self.events.shake(st.dir * d.shake);
        self.events.push(EventKind::ShieldChargeHit {
            player: p as u8,
            target: other as u8,
            pos,
        });
        self.do_block(
            content,
            p,
            false,
            true,
            BlockKind::ShieldCharge,
            None,
            false,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn empower_projectile(
        &mut self,
        content: &Content,
        p: usize,
        projectile: u32,
        dmg: f32,
        speed: f32,
        add: Option<ObjectId>,
    ) {
        let Some(k) = self.projectiles.iter().rposition(|x| x.id == projectile) else {
            return;
        };
        let g = self.players[p].gun.stats;
        let def = &content.projectiles
            [(self.players[p].gun.projectile as usize).min(content.projectiles.len() - 1)];
        let wf = def.world_force * g.gravity;
        let pr = &mut self.projectiles[k];
        pr.set_damage(pr.damage * dmg, g.size);
        pr.vel = (pr.vel - wf) * speed + wf;
        pr.start_speed *= speed;
        if let Some(o) = add {
            for a in content.addons(o) {
                pr.addons.push(super::addons::Addon::new(*a, 1.0));
            }
        }
        let pos = pr.pos;
        self.events.sound("SFX_Block_Empower_Shoot_SE", pos, 1.0);
        self.events.push(EventKind::EmpowerFired {
            player: p as u8,
            projectile,
        });
    }

    fn radar_go(&mut self, content: &Content, oi: usize, ci: usize, range: f32) {
        let Some(p) = self.obj_owner(oi) else { return };
        let pos = self.obj_pos(oi);
        let Some(t) = self.closest_enemy(content, pos, p, true) else {
            return;
        };
        let ppos = self.players[p].pos;
        if (self.players[t].pos - ppos).len() >= range {
            return;
        }
        let level = self.obj_level(content, oi);
        let pf = content.prefab(self.objects[oi].def);
        self.set_slot(
            oi,
            pf.comps[ci].slot,
            CompState::Radar {
                shots: level,
                // Every shot, the first included, waits 0.1 real s
                // (cards.md 11, Clarified 6.7). The object pass of this
                // tick still runs and counts one tick down.
                ticks: super::addons::real_ticks(0.1) + 1,
                target: t as u8,
            },
        );
        let tp = self.players[t].pos;
        self.events.push(EventKind::RadarPing {
            player: p as u8,
            target: t as u8,
            pos: tp,
        });
    }

    fn radar_tick(&mut self, content: &Content, oi: usize, ci: usize) {
        let pf = content.prefab(self.objects[oi].def);
        let slot = pf.comps[ci].slot;
        let CompState::Radar {
            shots,
            ticks: t,
            target,
        } = self.get_slot(oi, slot)
        else {
            return;
        };
        if shots <= 0 {
            return;
        }
        if t > 1 {
            self.set_slot(
                oi,
                slot,
                CompState::Radar {
                    shots,
                    ticks: t - 1,
                    target,
                },
            );
            return;
        }
        self.set_slot(
            oi,
            slot,
            CompState::Radar {
                shots: shots - 1,
                ticks: super::addons::real_ticks(0.1),
                target,
            },
        );
        let Some(p) = self.obj_owner(oi) else { return };
        let ti = target as usize;
        if !self.players[ti].hittable() || self.players[p].dead {
            return;
        }
        let d = self.players[ti].pos - self.players[p].pos;
        let ps = self.players[p].gun.stats.projectile_speed.max(1e-6);
        self.players[p].gun.force_shoot_dir = V2::UP * (d.len_sq() * 0.015 / ps) + d;
        self.gun_attack(content, p, true, 1.0, false);
        self.players[p].gun.force_shoot_dir = V2::ZERO;
    }

    fn rotating_attack(&mut self, content: &Content, oi: usize, ci: usize) {
        let pf = content.prefab(self.objects[oi].def);
        let Some(g) = self.effect_gun(content, oi, ci) else {
            return;
        };
        let shots = round_half_even(0.5 / g.attack_speed.max(1e-6)).clamp(1, 5);
        self.rotating_fire_ring(content, oi, ci);
        if shots > 1 {
            self.set_slot(
                oi,
                pf.comps[ci].slot,
                CompState::Shooter {
                    reps: shots - 1,
                    ticks: super::addons::real_ticks(0.1),
                },
            );
        }
    }

    /// The effect gun of a rotating shooter: the `Gun` on its node, with the
    /// buff gun merged once per level above 1 (`GunLevel`, weapons.md 21.1).
    fn effect_gun(&self, content: &Content, oi: usize, ci: usize) -> Option<GunStats> {
        let pf = content.prefab(self.objects[oi].def);
        let node = pf.comps[ci].node;
        let gi = pf
            .comps
            .iter()
            .position(|c| c.node == node && matches!(c.kind, Kind::Gun(_)))?;
        let Kind::Gun(g) = &pf.comps[gi].kind else {
            return None;
        };
        let mut stats = g.stats;
        let level = self.obj_level(content, oi);
        for c in &pf.comps {
            if let Kind::GunLevel { copy_to: Some(to) } = c.kind
                && to as usize == gi
                && let Some(Kind::Gun(buff)) = pf
                    .comps
                    .iter()
                    .find(|b| b.node == c.node && matches!(b.kind, Kind::Gun(_)))
                    .map(|b| &b.kind)
            {
                for _ in 1..level {
                    stats.merge_card(&buff.stats);
                }
            }
        }
        Some(stats)
    }

    fn rotating_fire_ring(&mut self, content: &Content, oi: usize, ci: usize) {
        let pf = content.prefab(self.objects[oi].def);
        let Kind::RotatingShooter {
            bullets_to_fire, ..
        } = pf.comps[ci].kind
        else {
            return;
        };
        let node = pf.comps[ci].node;
        let Some(gdef) = pf.comps.iter().find_map(|c| match &c.kind {
            Kind::Gun(g) if c.node == node => Some(g),
            _ => None,
        }) else {
            return;
        };
        let Some(stats) = self.effect_gun(content, oi, ci) else {
            return;
        };
        let Some(owner) = self.obj_owner(oi) else {
            return;
        };
        let level = self.obj_level(content, oi);
        let n = (bullets_to_fire * level).max(1);
        let step = 360.0 / n as f32;
        let pos = self.obj_pos(oi);
        for m in (1..=n).rev() {
            let a = (m as f32 * step) * crate::math::DEG2RAD;
            let dir = v2(cos(a), sin(a));
            let bursts = stats.bursts.clamp(1, 100);
            let np = stats.number_of_projectiles.max(0);
            for _ in 0..bursts {
                for _ in 0..np {
                    self.spawn_gun_projectile(
                        content,
                        owner,
                        &stats,
                        &gdef.spawns,
                        gdef.projectile,
                        pos,
                        dir,
                        1.0,
                        np,
                    );
                }
            }
        }
        self.events.push(EventKind::RingFired {
            player: owner as u8,
            id: self.objects[oi].id,
            count: n as u16,
        });
    }

    // ------------------------------------------------------------------
    // Abyssal countdown (cards.md 6.8)
    // ------------------------------------------------------------------

    fn abyssal_tick(&mut self, content: &Content, oi: usize, ci: usize, game_dt: f32) {
        let pf = content.prefab(self.objects[oi].def);
        let Kind::AbyssalCountdown {
            time_to_fill,
            time_to_empty,
            duration,
            hp_multiplier,
            objects,
        } = &pf.comps[ci].kind
        else {
            return;
        };
        let slot = pf.comps[ci].slot;
        let CompState::Abyssal {
            mut counter,
            mut remaining,
            mut form,
            mut start_counter,
        } = self.get_slot(oi, slot)
        else {
            return;
        };
        let Some(p) = self.obj_owner(oi) else { return };
        if !self.players[p].simulated {
            start_counter = 1.0;
            self.set_slot(
                oi,
                slot,
                CompState::Abyssal {
                    counter,
                    remaining,
                    form,
                    start_counter,
                },
            );
            return;
        }
        if start_counter > 0.0 {
            start_counter -= game_dt;
            self.set_slot(
                oi,
                slot,
                CompState::Abyssal {
                    counter,
                    remaining,
                    form,
                    start_counter,
                },
            );
            return;
        }
        if remaining > 0.0 {
            if !form {
                form = true;
                for n in objects.iter() {
                    self.set_node_active(content, oi, *n, true);
                }
                let pl = &mut self.players[p];
                pl.stats.max_health *= hp_multiplier;
                pl.health *= hp_multiplier;
                pl.configure_mass_and_size();
                self.events
                    .push(EventKind::AbyssalEntered { player: p as u8 });
            }
            remaining -= game_dt;
            counter = remaining / duration;
        } else {
            if form {
                form = false;
                for n in objects.iter() {
                    self.set_node_active(content, oi, *n, false);
                }
                let pl = &mut self.players[p];
                pl.stats.max_health /= hp_multiplier;
                pl.health /= hp_multiplier;
                pl.configure_mass_and_size();
                self.events.push(EventKind::AbyssalLeft { player: p as u8 });
            }
            let d = self.players[p].controls.direction;
            if d.is_zero() || d == V2::DOWN {
                counter += game_dt / time_to_fill;
            } else {
                counter -= game_dt / time_to_empty;
            }
            counter = clamp(counter, -0.1 / time_to_fill, 1.0);
            if counter >= 1.0 {
                remaining = *duration;
            }
        }
        self.set_slot(
            oi,
            slot,
            CompState::Abyssal {
                counter,
                remaining,
                form,
                start_counter,
            },
        );
    }

    fn abyssal_reset(&mut self, content: &Content, oi: usize, ci: usize) {
        let pf = content.prefab(self.objects[oi].def);
        let Kind::AbyssalCountdown {
            hp_multiplier,
            objects,
            ..
        } = &pf.comps[ci].kind
        else {
            return;
        };
        let slot = pf.comps[ci].slot;
        if let CompState::Abyssal { form, .. } = self.get_slot(oi, slot) {
            if form {
                for n in objects.iter() {
                    self.set_node_active(content, oi, *n, false);
                }
                if let Some(p) = self.obj_owner(oi) {
                    let pl = &mut self.players[p];
                    pl.stats.max_health /= hp_multiplier;
                    pl.health /= hp_multiplier;
                    pl.configure_mass_and_size();
                    self.events.push(EventKind::AbyssalLeft { player: p as u8 });
                }
            }
            self.set_slot(
                oi,
                slot,
                CompState::Abyssal {
                    counter: 0.0,
                    remaining: 0.0,
                    form: false,
                    start_counter: 0.0,
                },
            );
        }
    }

    // ------------------------------------------------------------------
    // Implosion pulls (weapons.md 19.4), FixedUpdate
    // ------------------------------------------------------------------

    pub(crate) fn pulls_tick(&mut self, game_dt: f32) {
        let ts = self.time.scale();
        for i in 0..self.players.len() {
            if self.players[i].pulls.is_empty() {
                continue;
            }
            let mut k = 0;
            while k < self.players[i].pulls.len() {
                let mut pl: Pull = self.players[i].pulls[k];
                let (pos, vel) = (self.players[i].pos, self.players[i].vel);
                let v = (pl.point - pos).clamp_len(pl.clamp);
                // `fixed_dt` is `TimeHandler.fixedDeltaTime` = 0.014 * ts
                // (weapons.md 19.4, Clarified).
                let f = v * pl.force + (-vel * pl.drag * (DT * ts)) * pl.drag * ts;
                self.take_force(i, f, ForceMode::Force, false, false, 0.0);
                pl.left -= game_dt;
                if pl.left <= 0.0 {
                    self.players[i].pulls.remove(k);
                } else {
                    self.players[i].pulls[k] = pl;
                    k += 1;
                }
            }
        }
    }

    /// Hit effects of explosions (Overpower damage, Implosion pull request).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn explosion_player_hooks(
        &mut self,
        content: &Content,
        i: usize,
        centre: V2,
        s: f32,
        spawner: Option<u8>,
        hooks: &crate::objects::ExplodeHooks,
        phase: u8,
    ) {
        if phase == 0
            && let (Some(dmg), Some(sp)) = (hooks.overpower, spawner)
        {
            let sp = sp as usize;
            let amount = self.players[sp].stats.max_health * dmg * 0.01 * s;
            let dir = (self.players[i].pos - self.players[sp].pos).normalized();
            let pos = self.players[i].pos;
            self.call_take_damage(content, i, dir * amount, pos, Some(sp as u8), true);
        }
        if phase == 1
            && let Some((force, clamp_dist, drag, time)) = hooks.implosion
        {
            self.players[i].pulls.push(Pull {
                point: centre,
                force,
                clamp: clamp_dist,
                drag,
                left: time,
            });
        }
    }
}
