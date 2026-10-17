//! Events produced by a step (docs/sim-design.md "Events"; weapons.md 23;
//! cards.md 9). The presentation keys them by `(frame, seq)` and fires each
//! once; events of rolled-back frames that are not re-produced can be
//! cancelled. Sound cue names are SoundEvent asset names (audio.md, player.md
//! 19.2) without the `_SE` suffix being optional: they are given in full.
//! Looping sounds (stun, reload, saw, platform, rope) are driven by state or
//! by start/stop events, not by `Sound`.

use crate::content::{CardId, MapId, ObjectId};
use crate::math::V2;

#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub frame: u32,
    /// Order within the frame.
    pub seq: u32,
    pub kind: EventKind,
}

/// What a projectile hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitTarget {
    Nothing,
    Map,
    Prop(u32),
    Player(u8),
    Bullet(u32),
}

/// Block trigger types (player.md 12.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockKind {
    Default,
    None,
    ShieldCharge,
    Echo,
    Empower,
}

#[derive(Clone, Debug, PartialEq)]
pub enum EventKind {
    /// One-shot positional sound cue.
    Sound {
        cue: &'static str,
        pos: V2,
        intensity: f32,
    },
    /// World "game feel" vector (screen shake), maps.md 9.1.
    Shake {
        vec: V2,
    },

    // ---- players ----
    PlayerSpawnMoveStarted {
        player: u8,
        from: V2,
        to: V2,
    },
    PlayerSpawned {
        player: u8,
        pos: V2,
    },
    Jumped {
        player: u8,
        pos: V2,
        dir: V2,
        wall: bool,
    },
    Landed {
        player: u8,
        pos: V2,
        normal: V2,
        impact: f32,
    },
    WallStick {
        player: u8,
        pos: V2,
        normal: V2,
    },
    Damaged {
        player: u8,
        amount: f32,
        dir: V2,
        pos: V2,
        attacker: Option<u8>,
        lethal: bool,
    },
    Healed {
        player: u8,
        amount: f32,
    },
    Died {
        player: u8,
        dir: V2,
        killer: Option<u8>,
    },
    PhoenixDied {
        player: u8,
        dir: V2,
    },
    Revived {
        player: u8,
        full: bool,
    },
    BlockStarted {
        player: u8,
        kind: BlockKind,
    },
    BlockRecharged {
        player: u8,
    },
    StatusEffectBlocked {
        player: u8,
    },
    Stunned {
        player: u8,
        start: bool,
    },
    Silenced {
        player: u8,
        start: bool,
    },
    Slowed {
        player: u8,
        amount: f32,
        particles: u32,
    },
    Bounced {
        player: u8,
        dir: V2,
    },
    OutOfBounds {
        player: u8,
        pos: V2,
        inward: V2,
        shielded: bool,
    },
    OutOfBoundsWarning {
        player: u8,
        pos: V2,
        inward: V2,
    },

    // ---- weapons ----
    ShotFired {
        player: u8,
        projectiles: u16,
        pos: V2,
        dir: V2,
    },
    ProjectileSpawned {
        id: u32,
        player: Option<u8>,
        pos: V2,
        vel: V2,
        size: f32,
    },
    ProjectileHit {
        id: u32,
        point: V2,
        normal: V2,
        damage: f32,
        target: HitTarget,
        blocked: bool,
    },
    ProjectileBlocked {
        id: u32,
        player: u8,
        point: V2,
    },
    ProjectileBounced {
        id: u32,
        point: V2,
        normal: V2,
    },
    /// Removed by bullet-vs-bullet collision.
    ProjectileKilled {
        id: u32,
        pos: V2,
    },
    ProjectileRemoved {
        id: u32,
    },
    ReloadStarted {
        player: u8,
        duration: f32,
    },
    ReloadDone {
        player: u8,
        rounds: i32,
    },
    OutOfAmmo {
        player: u8,
    },
    CantShoot {
        player: u8,
    },

    // ---- objects / map ----
    ObjectSpawned {
        id: u32,
        def: ObjectId,
        pos: V2,
        dir: V2,
        scale: f32,
    },
    ObjectRemoved {
        id: u32,
    },
    Explosion {
        pos: V2,
        radius: f32,
        def: Option<ObjectId>,
    },
    PropDamaged {
        prop: u32,
    },
    PropDestroyed {
        prop: u32,
        pos: V2,
    },
    PropImpact {
        prop: u32,
        pos: V2,
        impulse: f32,
    },
    SawHit {
        pos: V2,
        dir: V2,
    },
    SmasherSound {
        index: u16,
        impact: bool,
    },

    // ---- card effects (M5; cards.md 9) ----
    /// A looping sound of an effect object starts/stops (`key` identifies
    /// the instance: object id * 64 + component index).
    LoopSound {
        cue: &'static str,
        key: u32,
        pos: V2,
        start: bool,
    },
    /// A node of an effect object was (de)activated (`SetActive`, e.g. the
    /// abyssal form's saw/pull field, the healing field's counter canvas).
    ObjectNodeActive {
        id: u32,
        node: u16,
        active: bool,
    },
    /// An effect object's particle system / line effect on `node` plays
    /// (`play`) or stops; `line` = a `LineEffect` animation.
    ObjectCue {
        id: u32,
        node: u16,
        play: bool,
        line: bool,
    },
    /// A card effect object was added to a player (object `id`, prefab `def`).
    EffectAdded {
        player: u8,
        id: u32,
        def: ObjectId,
    },
    /// Picking the same effect card again raised its level.
    EffectLevelUp {
        player: u8,
        id: u32,
        level: i32,
    },
    /// Teleport block (cards.md 6.3): cue `SFX_Block_Teleport_SE`.
    Teleported {
        player: u8,
        from: V2,
        to: V2,
    },
    /// Shield Charge (6.4): cue `SFX_Block_Shield_Charge_SE` at start.
    ShieldChargeStart {
        player: u8,
        dir: V2,
    },
    ShieldChargeHit {
        player: u8,
        target: u8,
        pos: V2,
    },
    ShieldChargeEnd {
        player: u8,
    },
    /// Empower (6.5): armed by a block (cue `SFX_Block_Empower_Spawn_SE`),
    /// fired with the next shot (`SFX_Block_Empower_Shoot_SE`), triggered at
    /// the impact point.
    EmpowerArmed {
        player: u8,
    },
    EmpowerFired {
        player: u8,
        projectile: u32,
    },
    EmpowerTriggered {
        player: u8,
        pos: V2,
    },
    /// Abyssal Countdown form entered/left (6.8); the fill ring is
    /// `World::abyssal_counter`.
    AbyssalEntered {
        player: u8,
    },
    AbyssalLeft {
        player: u8,
    },
    /// Radar Shot found a target (6.7).
    RadarPing {
        player: u8,
        target: u8,
        pos: V2,
    },
    /// A stat-toggling passive switched (Brawler, Taste of Blood, Chase,
    /// Pristine Perseverance; 6.9-6.11).
    StatBoost {
        player: u8,
        object: u32,
        on: bool,
    },
    /// A range trigger fired on `target` (Chilling Presence pulse,
    /// Lifestealer beam, Remnant; 6.12).
    RangeTriggered {
        id: u32,
        player: u8,
        target: u8,
    },
    /// Lifestealer drained `amount` from `from` to `to`.
    LifeDrain {
        from: u8,
        to: u8,
        amount: f32,
    },
    /// Remnant: a static copy of `player`'s body was spawned as object `id`.
    RemnantSpawned {
        player: u8,
        id: u32,
    },
    /// Ensnare rope attached (cue `SFX_Character_Effect_Ensnare_SE`) /
    /// snapped (`SFX_Character_Jump_Ensnare_Break_SE`).
    Ensnared {
        id: u32,
        target: u8,
        start: V2,
    },
    EnsnareSnapped {
        id: u32,
    },
    /// EMP / Bombs Away ring of `count` projectiles (weapons.md 21.1).
    RingFired {
        player: u8,
        id: u32,
        count: u16,
    },
    /// Drill ammo entered / left a wall (weapons.md 16.2).
    DrillEnter {
        id: u32,
        pos: V2,
    },
    DrillExit {
        id: u32,
        pos: V2,
    },
    /// Homing locked on (cue `SFX_Bullet_Effect_Homing_Found_SE`).
    HomingLocked {
        id: u32,
        target: u8,
    },
    /// Target bounce retargeted a bullet (cue
    /// `SFX_Bullet_Effect_Target_Bounce_Target_Player_SE`).
    TargetBounce {
        id: u32,
        target: u8,
    },

    // ---- match flow ----
    MatchStarted,
    /// A new map was chosen and placed off-screen right; `seed` drives the
    /// per-child slide delays of the presentation.
    MapLoaded {
        map: MapId,
        seed: u32,
    },
    /// The map slides in (match-flow.md 7.1); the old one slides out.
    MapCallIn {
        map: MapId,
    },
    MapEntered {
        map: MapId,
    },
    MapReady {
        map: MapId,
    },
    BattleStarted,
    PointWon {
        team: u8,
        points: Vec<u8>,
        rounds: Vec<u8>,
    },
    RoundWon {
        team: u8,
        rounds: Vec<u8>,
    },
    GameWon {
        team: u8,
    },
    /// Point counters reset (round over / rematch).
    ScoreReset,
    PickStarted {
        player: u8,
    },
    CardOffered {
        player: u8,
        slot: u8,
        card: CardId,
    },
    CardSelected {
        player: u8,
        slot: u8,
    },
    CardPicked {
        player: u8,
        card: CardId,
        slot: u8,
    },
    PickEnded {
        player: u8,
    },
    RematchPrompt,
    RematchSelected {
        yes: bool,
    },
    Rematch,
    /// "No" on the rematch prompt: the game returns to the menu.
    MatchEnded,
    /// Time-scale curve started (`slow` = slow-down, else speed-up).
    TimeCurve {
        slow: bool,
    },
    /// A player forfeited / disconnected and left the match for good
    /// ([`crate::World::set_disconnected`]).
    PlayerDisconnected {
        player: u8,
    },
}

/// Event sink owned by the world for the current frame.
#[derive(Clone, Debug, Default)]
pub struct Events {
    pub(crate) list: Vec<Event>,
    pub(crate) frame: u32,
}

impl Events {
    pub fn push(&mut self, kind: EventKind) {
        let seq = self.list.len() as u32;
        self.list.push(Event {
            frame: self.frame,
            seq,
            kind,
        });
    }
    pub fn sound(&mut self, cue: &'static str, pos: V2, intensity: f32) {
        self.push(EventKind::Sound {
            cue,
            pos,
            intensity,
        });
    }
    pub fn shake(&mut self, vec: V2) {
        self.push(EventKind::Shake { vec });
    }
    pub(crate) fn begin(&mut self, frame: u32) {
        self.list.clear();
        self.frame = frame;
    }
}

/// Sound cue names used by the sim (audio.md section 6, player.md 19.2).
pub mod cue {
    pub const JUMP: &str = "SFX_Character_Jump_Ground_SE";
    pub const JUMP_BIG: &str = "SFX_Character_Jump_Ground_Big_SE";
    pub const LAND: &str = "SFX_Character_Land_SE";
    pub const LAND_BIG: &str = "SFX_Character_Land_Big_SE";
    pub const STICK_WALL: &str = "SFX_Character_Stick_Wall_SE";
    pub const STICK_WALL_BIG: &str = "SFX_Character_Stick_Wall_Big_SE";
    pub const BOUNCE: &str = "SFX_Character_Bounce_SE";
    pub const DIE: &str = "SFX_Character_Die_SE";
    pub const BLOCK_START: &str = "SFX_Character_Block_Start_SE";
    pub const BLOCK_BLOCKED: &str = "SFX_Character_Block_Blocked_SE";
    pub const BLOCK_RECHARGED: &str = "SFX_Character_Block_Recharged_SE";
    pub const BLOCK_STATUS: &str = "SFX_Character_Block_Status_Effect_SE";
    pub const CANT_SHOOT: &str = "SFX_Character_Cant_Shoot_SE";
    pub const RELOAD_COMPLETE: &str = "SFX_Character_Reload_Complete_SE";
    pub const RELOAD_START: &str = "SFX_Character_Reload_Start_SE";
    pub const HEAL: &str = "SFX_Character_Effect_Heal_SE";
    pub const DAMAGE_PASSIVE: &str = "SFX_Character_Effect_Damage_Passive_SE";
    pub const DAMAGE_POISON: &str = "SFX_Character_Effect_Damage_Poison_SE";
    pub const HOMING_FOUND: &str = "SFX_Bullet_Effect_Homing_Found_SE";
    pub const TARGET_BOUNCE: &str = "SFX_Bullet_Effect_Target_Bounce_Target_Player_SE";
    pub const DAMAGE_LIFE_STEAL: &str = "SFX_Character_Effect_Damage_Life_Steal_SE";
    pub const DAMAGE_SCREEN_EDGE: &str = "SFX_Character_Effect_Damage_Screen_Edge_SE";
    pub const SLOW_FREEZE: &str = "SFX_Character_Effect_Slow_Freeze_SE";
    pub const SPAWN: [&str; 4] = [
        "SFX_Character_Spawn_A_SE",
        "SFX_Character_Spawn_B_SE",
        "SFX_Character_Spawn_C_SE",
        "SFX_Character_Spawn_D_SE",
    ];
    pub const IMPACT_BULLET_ON_BULLET: &str = "SFX_Impact_Bullet_On_Bullet_SE";
    pub const IMPACT_BOUNCE: &str = "SFX_Impact_Environment_Bounce_SE";
    pub const BOX_IMPACT: &str = "SFX_Environment_Box_Impact_SE";
    pub const BOX_DESTRUCTION: &str = "SFX_Environment_Box_Destruction_SE";
    pub const SAW_DAMAGE: &str = "SFX_Environment_Saw_Damage_SE";
    pub const SMASHER_MOVE: &str = "SFX_Environment_Smasher_Move_SE";
    pub const SMASHER_IMPACT: &str = "SFX_Environment_Smasher_Impact_Ground_SE";
    pub const TEXT_APPEAR: &str = "UI_Text_Appear_SE";
    pub const TEXT_DISAPPEAR: &str = "UI_Text_Disappear_SE";
    pub const LEVEL_IN: &str = "UI_Level_Transition_In_SE";
    pub const LEVEL_OUT: &str = "UI_Level_Transition_Out_SE";
    pub const BALL_APPEAR: &str = "UI_Player_Ball_Appear_SE";
    pub const BALL_DISAPPEAR: &str = "UI_Player_Ball_Disappear_SE";
    pub const CARD_SWITCH: &str = "UI_Card_Switch_SE";
    pub const CARD_PICK: &str = "UI_Card_Pick_SE";
    pub const CARDS_APPEAR: &str = "UI_Cards_Appear_SE";
    pub const WIN_ROUND: &str = "UI_Win_Round_SE";
}
