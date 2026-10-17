//! Per-player, per-tick input (player.md 3.3) and the derived edge state.
//!
//! [`PlayerInput`] is what travels over the network (GGRS) and what local
//! devices produce: 8 bytes, `bytemuck::Pod`, all-zero = "no input".
//! Dead zones, the "force 8 dir" aim options and mouse -> aim-vector
//! conversion are done by the platform layer *before* quantising; the sim
//! applies the 16-direction move snap, the aim gravity bias and everything else.

use crate::math::{V2, snap16, v2};
use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Pod, Zeroable)]
pub struct PlayerInput {
    /// Move stick, -127..=127 each axis (y up).
    pub move_x: i8,
    pub move_y: i8,
    /// Held buttons, see the `BTN_*` constants.
    pub buttons: u16,
    /// Aim vector, -32767..=32767 per axis = -1..1 (y up). (0,0) = no aim
    /// (stick centred). Mouse players send the unit vector from the player
    /// to the cursor.
    pub aim_x: i16,
    pub aim_y: i16,
}

impl PlayerInput {
    pub const BTN_JUMP: u16 = 1 << 0;
    pub const BTN_FIRE: u16 = 1 << 1;
    pub const BTN_BLOCK: u16 = 1 << 2;
    /// Menu navigation (card pick, rematch prompt). The move stick (|x| > 0.7)
    /// works too, as in the original.
    pub const BTN_MENU_LEFT: u16 = 1 << 3;
    pub const BTN_MENU_RIGHT: u16 = 1 << 4;
    /// Confirm (jump confirms as well).
    pub const BTN_MENU_OK: u16 = 1 << 5;
    pub const BTN_MENU_BACK: u16 = 1 << 6;
    /// Forfeit: the player leaves the match for good (device lost, left
    /// the room). The first tick this bit is held in the player's input
    /// calls [`crate::World::set_disconnected`] for the slot; the rest of
    /// the input is ignored from then on. See docs/spec/match-flow.md 13.
    pub const BTN_FORFEIT: u16 = 1 << 15;

    /// Build from float axes (already dead-zoned) and buttons.
    pub fn from_axes(mv: V2, aim: V2, buttons: u16) -> PlayerInput {
        let q8 = |f: f32| (f.clamp(-1.0, 1.0) * 127.0).round() as i8;
        let q16 = |f: f32| (f.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        PlayerInput {
            move_x: q8(mv.x),
            move_y: q8(mv.y),
            buttons,
            aim_x: q16(aim.x),
            aim_y: q16(aim.y),
        }
    }

    pub fn move_vec(&self) -> V2 {
        v2(self.move_x as f32 / 127.0, self.move_y as f32 / 127.0)
    }

    pub fn aim_vec(&self) -> V2 {
        v2(self.aim_x as f32 / 32767.0, self.aim_y as f32 / 32767.0)
    }

    #[inline]
    pub fn held(&self, b: u16) -> bool {
        self.buttons & b != 0
    }
}

/// Edge-detected view of one tick's input (player.md 3.5).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Controls {
    /// Snapped unit move direction (or zero).
    pub direction: V2,
    /// Raw (quantised) move axes, used by menus.
    pub raw_move: V2,
    /// Aim vector including the gravity-compensation bias (zero if centred).
    pub aim: V2,
    pub jump_held: bool,
    pub jump_pressed: bool,
    pub shoot_held: bool,
    pub shoot_pressed: bool,
    pub shoot_released: bool,
    pub block_pressed: bool,
}

/// Derive controls from this and last tick's raw inputs.
/// `locked`: stunned / not playing -> everything zero. `silenced` removes fire
/// and block. `projectile_speed` feeds the aim bias (player.md 3.5 step 2).
pub fn derive_controls(
    cur: &PlayerInput,
    prev: &PlayerInput,
    locked: bool,
    silenced: bool,
    projectile_speed: f32,
) -> Controls {
    if locked {
        return Controls::default();
    }
    let raw_move = cur.move_vec();
    let direction = snap16(raw_move);
    let mut aim = cur.aim_vec();
    if !aim.is_zero() {
        aim.y += 0.13 / projectile_speed.clamp(1.0, 100.0);
    }
    let pressed = |b: u16| cur.held(b) && !prev.held(b);
    let released = |b: u16| !cur.held(b) && prev.held(b);
    let mut c = Controls {
        direction,
        raw_move,
        aim,
        jump_held: cur.held(PlayerInput::BTN_JUMP),
        jump_pressed: pressed(PlayerInput::BTN_JUMP),
        ..Default::default()
    };
    if !silenced {
        c.shoot_held = cur.held(PlayerInput::BTN_FIRE);
        c.shoot_pressed = pressed(PlayerInput::BTN_FIRE);
        c.shoot_released = released(PlayerInput::BTN_FIRE);
        c.block_pressed = pressed(PlayerInput::BTN_BLOCK);
    }
    c
}

/// Menu navigation edges: -1 left, +1 right, 0 none; plus confirm.
/// The stick counts when |x| > 0.7 and triggers on change (match-flow.md 9.3).
pub fn menu_nav(cur: &PlayerInput, prev: &PlayerInput) -> (i32, bool, bool) {
    let dir = |i: &PlayerInput| -> i32 {
        let x = i.move_vec().x;
        if x > 0.7 || i.held(PlayerInput::BTN_MENU_RIGHT) {
            1
        } else if x < -0.7 || i.held(PlayerInput::BTN_MENU_LEFT) {
            -1
        } else {
            0
        }
    };
    let (d, pd) = (dir(cur), dir(prev));
    let nav = if d != pd { d } else { 0 };
    let ok_b = PlayerInput::BTN_JUMP | PlayerInput::BTN_MENU_OK;
    let ok = (cur.buttons & ok_b) != 0 && (prev.buttons & ok_b) == 0;
    let back = cur.held(PlayerInput::BTN_MENU_BACK) && !prev.held(PlayerInput::BTN_MENU_BACK);
    (nav, ok, back)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_and_roundtrip() {
        assert_eq!(std::mem::size_of::<PlayerInput>(), 8);
        let i = PlayerInput::from_axes(v2(1.0, -0.5), v2(0.0, 1.0), PlayerInput::BTN_FIRE);
        assert_eq!(i.move_x, 127);
        assert!((i.aim_vec().y - 1.0).abs() < 1e-6);
    }

    #[test]
    fn edges() {
        let a = PlayerInput {
            buttons: PlayerInput::BTN_JUMP,
            ..Default::default()
        };
        let z = PlayerInput::default();
        let c = derive_controls(&a, &z, false, false, 1.0);
        assert!(c.jump_pressed && c.jump_held);
        let c = derive_controls(&a, &a, false, false, 1.0);
        assert!(!c.jump_pressed && c.jump_held);
        let f = PlayerInput {
            buttons: PlayerInput::BTN_FIRE,
            ..Default::default()
        };
        let c = derive_controls(&f, &z, false, true, 1.0);
        assert!(!c.shoot_pressed, "silence removes fire");
    }
}
