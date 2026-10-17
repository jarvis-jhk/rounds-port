//! Card effect hook points (docs/sim-design.md "Entities"; player.md 16;
//! cards.md 6). The sim calls [`World::fire_hook`] at every hook point listed
//! here; the hook is dispatched to the components of the player's card
//! effect objects (`crate::script`).

use crate::content::Content;
use crate::events::BlockKind;
use crate::math::V2;
use crate::world::World;

/// Block hook phases (player.md 12.4 step 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockPhase {
    SuperFirst,
    FirstDelaysOthers,
    Early,
    Block,
}

/// A hook invocation with its data.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hook {
    Jump,
    TouchGround {
        since_grounded: f32,
        point: V2,
        normal: V2,
    },
    TouchWall {
        since_wall_grab: f32,
        point: V2,
        normal: V2,
    },
    DealtDamage {
        damage: V2,
        self_damage: bool,
        victim: u8,
    },
    WasDealtDamage {
        damage: V2,
        self_damage: bool,
    },
    Block {
        phase: BlockPhase,
        kind: BlockKind,
    },
    BlockedProjectile {
        projectile: u32,
        forward: V2,
        point: V2,
    },
    BlockRecharged,
    PlayerCollision {
        point: V2,
        push: V2,
        other: u8,
    },
    ReloadDone {
        rounds: i32,
    },
    OutOfAmmo {
        max_ammo: i32,
    },
    Revive {
        full: bool,
    },
    DelayedRevive,
    Death,
    /// One trigger pull (`Gun.attack actions`).
    Attack,
    /// Per projectile, before the ammo decrement (`ShootProjectileAction`).
    ShootProjectile {
        projectile: u32,
    },
    RoundStart,
    PointEnd,
    Tick,
}

impl World {
    /// Dispatch a hook to the player's active card effects.
    #[inline]
    pub(crate) fn fire_hook(&mut self, content: &Content, player: usize, hook: Hook) {
        if self.players[player].effects.is_empty() {
            return;
        }
        self.effects_hook(content, player, hook);
    }
}
