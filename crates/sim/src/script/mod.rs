//! Card effect scripts (cards.md 5-6, weapons.md 16-21): a small component
//! runtime for prefabs.
//!
//! Every prefab ([`crate::content::ObjectDef`]) is compiled once, when the
//! [`crate::Content`] is built, into a [`Prefab`]: its node tree and a list
//! of [`Comp`]onents whose parameters come from the imported Unity fields
//! (`EffectDesc::params`). UnityEvent wiring (`DelayEvent.delayedEvent`,
//! `BlockTrigger.triggerEvent`, ...) becomes [`Call`] lists that name a target
//! component or node and a [`Method`]. At run time a spawned object keeps a
//! small per-instance [`ObjState`] (node active flags, component enabled
//! flags, and one [`CompState`] per stateful component).
//!
//! Effect objects added to a player by a card (`AddObjectToPlayer`, the
//! "A_..." prefabs) are ordinary spawned objects of kind
//! [`crate::objects::ObjKind::PlayerEffect`]; the player's hook points
//! ([`crate::effects::Hook`]) dispatch to their components.
//!
//! Projectile add-ons (`ObjectsToSpawn.AddToProjectile`: homing, drill,
//! poison, ...) are compiled separately into [`addons::AddonDef`] lists and
//! run inside the projectile update (`addons.rs`).

pub mod addons;
pub mod compile;
pub mod run;
pub mod sounds;

use crate::content::{ExplosionDef, ObjectId, SpawnEntry};
use crate::math::{Curve, V2};
use crate::stats::GunStats;
use std::sync::Arc;

pub type CompIdx = u16;
pub type NodeIdx = u16;

/// Method named by a UnityEvent persistent call (only those the shipped
/// prefabs use; anything else is [`Method::Other`] and ignored).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    /// `Explosion.Explode`.
    Explode,
    /// `SpawnObjects.Spawn`.
    Spawn,
    /// `Go` of DealDamageToPlayer / RadarShot / StunPlayer / ResetBlock /
    /// SpawnStaticRemnant; `DelayEvent.Go` starts the delay.
    Go,
    /// `Behaviour.set_enabled(bool)`.
    SetEnabled(bool),
    /// `GameObject.SetActive(bool)`.
    SetActive(bool),
    DoJump,
    DoReload,
    DoBlock,
    TurnOn,
    TurnOff,
    /// `CooldownWindUp.Add`.
    Add,
    /// `CooldownWindUp.Reset`.
    Reset,
    /// `CooldownCondition.TryEvent`.
    TryEvent,
    /// `StatsAfterDealingDamage.Interupt`.
    Interrupt,
    /// `RemoveAfterSeconds.ResetCounter`.
    ResetCounter,
    /// `DisableEvent.OnDisable` called directly.
    OnDisable,
    /// `RotatingShooter.Attack`.
    Attack,
    /// Sound player start / end.
    PlayStart,
    PlayEnd,
    /// `AddShake.DoShake`.
    Shake,
    /// `ParticleSystem.Play` / `Stop` / `Clear` (presentation).
    ParticlePlay,
    ParticleStop,
    /// `LineEffect.Play` and the line animations (presentation).
    LinePlay,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Comp(CompIdx),
    Node(NodeIdx),
}

/// One persistent call of a UnityEvent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Call {
    pub target: Target,
    pub method: Method,
}

/// A UnityEvent: calls in order.
pub type Ev = Box<[Call]>;

/// Effect gun of a rotating shooter (weapons.md 21.1).
#[derive(Clone, Debug, PartialEq)]
pub struct GunDef {
    pub stats: GunStats,
    pub spawns: Arc<[SpawnEntry]>,
    /// Index into `Content::projectiles`.
    pub projectile: u16,
}

/// Compiled component kinds with their imported parameters. Field names
/// follow the Unity fields (cards.md 6, weapons.md 19-21).
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Explosion(ExplosionDef),
    ExplosionOverpower {
        dmg_per_100hp: f32,
    },
    Implosion {
        force: f32,
        clamp_dist: f32,
        drag: f32,
        time: f32,
    },
    RemoveAfterSeconds {
        seconds: f32,
        scale_with_scale: bool,
    },
    DelayEvent {
        time: f32,
        auto: bool,
        repeating: bool,
        ev: Ev,
    },
    StartEvent(Ev),
    EnableEvent(Ev),
    DisableEvent(Ev),
    RespawnEvent(Ev),
    DestroyEvent(Ev),
    SpawnObjects {
        objects: Box<[ObjectId]>,
        transform_rotation: bool,
        inherit_scale: bool,
        destroy_object: bool,
        destroy_root: bool,
    },
    FollowPlayer {
        other: bool,
        inherit_scale: bool,
    },
    AttackLevel {
        level: i32,
        level_scale_m: f32,
    },
    LevelScale {
        on_start: bool,
        on_level_up: bool,
    },
    BlockTrigger {
        /// `blackListedType` (1 = None = react to every block type).
        blacklisted: u8,
        delay_other_actions: bool,
        super_first: Ev,
        first_delays: Ev,
        early: Ev,
        block: Ev,
        success: Ev,
        recharge: Ev,
    },
    EmpowerStopFollow,
    Saw {
        range: f32,
        damage: f32,
        force: f32,
        shake: f32,
    },
    Teleport {
        distance: f32,
        mask: u32,
    },
    ShieldCharge(Box<ShieldChargeDef>),
    Empower {
        dmg_multiplier: f32,
        speed_multiplier: f32,
        add_object: Option<ObjectId>,
    },
    RotatingShooter {
        bullets_to_fire: i32,
        destroy_after_attack: bool,
    },
    Gun(Box<GunDef>),
    GunLevel {
        /// The gun this buff gun is merged into (same prefab).
        copy_to: Option<CompIdx>,
    },
    PlayerDoJump {
        multiplier: f32,
    },
    RadarShot {
        range: f32,
    },
    AbyssalCountdown {
        time_to_fill: f32,
        time_to_empty: f32,
        duration: f32,
        hp_multiplier: f32,
        objects: Box<[NodeIdx]>,
    },
    BlinkStep {
        interval: f32,
    },
    StatsAfterDealingDamage {
        duration: f32,
        movement_speed: f32,
        jump: f32,
        hp: f32,
        start: Ev,
        end: Ev,
    },
    StatsWhenFullHp {
        health_multiplier: f32,
        size_multiplier: f32,
        threshold: f32,
    },
    Chase {
        on: Ev,
        off: Ev,
    },
    ToggleStats {
        hp: f32,
        movement_speed: f32,
    },
    PlayerInRangeTrigger {
        any: bool,
        range: f32,
        cooldown: f32,
        repeating: bool,
        scale_with_range: bool,
        ev: Ev,
    },
    PlayerInRangeSlow {
        amount: f32,
        max: f32,
    },
    DealDamageToPlayer {
        damage: f32,
        lethal: bool,
        other: bool,
    },
    RegenerateAfterStandStill {
        heal: f32,
    },
    DealtDamageTrigger(Ev),
    DoPlayerReload,
    ReloadTrigger {
        out_of_ammo: Ev,
        reload_done: Ev,
    },
    PlayerDoBlock,
    SpawnObjectOnDealDamage {
        damage_needed: f32,
        cd: f32,
        allow_self: bool,
        ev: Ev,
    },
    ResetBlock,
    AttackTrigger {
        every_shot: bool,
        ev: Ev,
    },
    DuringReloadTrigger {
        ev: Ev,
        start: Ev,
        end: Ev,
    },
    CooldownCondition {
        cooldown: f32,
        ev: Ev,
    },
    CooldownWindUp {
        increase_per_shot: f32,
        curve: Curve,
    },
    LineRangeEffect {
        dmg: f32,
        knockback: f32,
    },
    /// `LineEffect` (only its ring radius matters to gameplay: Radiance).
    LineEffect {
        radius: f32,
        radius_over_time: Curve,
        speed: f32,
    },
    SpawnStaticRemnant {
        source: Option<ObjectId>,
    },
    CounterUi {
        time_to_fill: f32,
        done: Ev,
    },
    StunPlayer {
        own: bool,
        time: f32,
    },
    ThereCanOnlyBeOne(Ev),
    CapScale {
        min: f32,
        max: f32,
    },
    Thruster {
        force: f32,
        pow: f32,
        physics_object_m: f32,
        drag: f32,
    },
    Ensnare(Box<EnsnareDef>),
    AddShake {
        shake: V2,
        auto: bool,
        inherit_scale: bool,
        max: Option<f32>,
    },
    SoundPlayer {
        start: Option<&'static str>,
        start_loop: Option<&'static str>,
        end: Option<&'static str>,
        on_start: bool,
        on_enable: bool,
        end_on_disable: bool,
        end_on_destroy: bool,
    },
    /// A `ParticleSystem` (call target only, presentation).
    Particles,
    /// Anything else (visual helpers, unknown scripts).
    Cosmetic,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ShieldChargeDef {
    pub time: f32,
    pub force: f32,
    pub drag: f32,
    pub force_curve: Curve,
    pub damage: f32,
    pub knockback: f32,
    pub stop_force: f32,
    pub shake: f32,
    pub damage_per_level: f32,
    pub knockback_per_level: f32,
    pub force_per_level: f32,
    pub time_per_level: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EnsnareDef {
    pub range: f32,
    pub force: f32,
    pub break_distance: f32,
    pub drag: f32,
    pub snap_cap_line_time: f32,
    pub start_pos_is_my_pos: bool,
    /// The rope `LineEffect`'s `globalTimeSpeed` (its counter advances by
    /// `real_dt * line_speed`).
    pub line_speed: f32,
    /// Counter value where the rope's width curve reaches zero: the pull
    /// runs while `counter < line_end` (cards.md 6.15).
    pub line_end: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Comp {
    pub node: NodeIdx,
    /// Initial `enabled`.
    pub enabled: bool,
    pub kind: Kind,
    /// Index into `ObjState::comps` for stateful kinds.
    pub slot: Option<u16>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub name: String,
    pub parent: Option<NodeIdx>,
    pub active: bool,
    pub scale: f32,
}

/// A compiled prefab.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Prefab {
    pub nodes: Vec<Node>,
    pub comps: Vec<Comp>,
    /// Number of state slots.
    pub slots: u16,
    /// Index of the root `AttackLevel` (level merging, cards.md 5.2).
    pub attack_level: Option<CompIdx>,
    /// Root `FollowPlayer` (Self target): the object follows its spawner.
    pub follows: bool,
    /// Has no behaviour beyond an auto explosion/lifetime (no state needed).
    pub trivial: bool,
}

impl Prefab {
    /// `activeInHierarchy` of a node given the per-instance active bits.
    pub fn active_in_hierarchy(&self, active: u64, mut n: NodeIdx) -> bool {
        loop {
            if !node_bit(active, n) {
                return false;
            }
            match self.nodes.get(n as usize).and_then(|x| x.parent) {
                Some(p) => n = p,
                None => return true,
            }
        }
    }
    pub fn initial_active(&self) -> u64 {
        let mut a = 0u64;
        for (i, n) in self.nodes.iter().enumerate() {
            if n.active || i >= 64 {
                a |= bit(i);
            }
        }
        a
    }
    pub fn initial_enabled(&self) -> u64 {
        let mut a = 0u64;
        for (i, c) in self.comps.iter().enumerate() {
            if c.enabled || i >= 64 {
                a |= bit(i);
            }
        }
        a
    }
}

#[inline]
pub fn bit(i: usize) -> u64 {
    if i < 64 { 1u64 << i } else { 0 }
}
/// Bits beyond 64 are treated as always set.
#[inline]
pub fn node_bit(mask: u64, i: NodeIdx) -> bool {
    (i as usize) >= 64 || mask & (1u64 << i) != 0
}

/// Per-instance state of one stateful component.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CompState {
    None,
    /// DelayEvent: real ticks until it fires (0 = idle).
    Delay {
        pending: u32,
    },
    /// RemoveAfterSeconds: remaining game seconds.
    Remove {
        left: f32,
    },
    /// AttackLevel.
    Level {
        level: i32,
    },
    /// LevelScale / FollowPlayer / CapScale: captured start scale.
    StartScale {
        scale: f32,
        inited: bool,
    },
    /// PlayerInRangeTrigger.
    InRange {
        counter: f32,
        done: bool,
        in_range: bool,
        target: Option<u8>,
        range: f32,
        started: bool,
    },
    /// Generic timer (CounterUI fill, BlinkStep counter, LineEffect counter).
    Timer {
        t: f32,
        done: bool,
    },
    /// Stat toggles (StatsAfterDealingDamage, StatsWhenFullHP, ToggleStats, Chase).
    Toggle {
        on: bool,
    },
    Abyssal {
        counter: f32,
        remaining: f32,
        form: bool,
        start_counter: f32,
    },
    Charge(ChargeState),
    Empower {
        armed: bool,
    },
    Teleport {
        pending: u8,
        to: V2,
    },
    Radar {
        shots: i32,
        ticks: u32,
        target: u8,
    },
    Refresh {
        acc: f32,
        since: f32,
    },
    Cooldown {
        last: f32,
        cooldown: f32,
        base: f32,
    },
    WindUp {
        value: f32,
    },
    Reloading {
        was: bool,
    },
    Ensnare {
        target: Option<u8>,
        start: V2,
        elapsed: f32,
        scale: f32,
        snapped: bool,
    },
    Thrust {
        dir: V2,
        force: f32,
        started: bool,
    },
    Shooter {
        reps: i32,
        ticks: u32,
    },
    /// LineRangeEffect: hit done.
    Done {
        done: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChargeState {
    pub active: bool,
    pub c: f32,
    pub used_time: f32,
    pub block_time: f32,
    pub dir: V2,
    pub force: f32,
    pub damage: f32,
    pub knockback: f32,
    /// Bit per player already hit this charge.
    pub hits: u16,
    pub cancelled: bool,
    /// Empower trigger: wait one tick, then charge from the old position.
    pub wait: u8,
    pub from: V2,
}

/// Per-instance runtime state of a spawned object.
#[derive(Clone, Debug, PartialEq)]
pub struct ObjState {
    pub active: u64,
    pub enabled: u64,
    /// Nodes whose components already ran `Start`.
    pub started: u64,
    pub comps: Vec<CompState>,
    /// `SpawnObjects.mostRecentlySpawnedObject` (any SpawnObjects of this object).
    pub recent_spawn: Option<u32>,
    /// `EmpowerStopBlockObjectFollow` turned following off.
    pub follow_stopped: bool,
}

impl ObjState {
    pub fn comp_enabled(&self, c: CompIdx) -> bool {
        (c as usize) >= 64 || self.enabled & (1u64 << c) != 0
    }
}
