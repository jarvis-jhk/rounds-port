//! Local devices -> player slots -> sim inputs (player.md 3).
//!
//! Each slot is driven by a device, a bot, or (online, later) a remote
//! machine. Device states are already dead-zoned by the input hub (radial
//! 0.2, rescaled); here the upper dead zone (0.9 = full deflection) is
//! applied, the Aim script's 0.2 threshold, and mouse aim is turned into
//! the unit vector from the player (as drawn on screen) to the cursor.

use glam::Vec2;
use rounds_render::Camera2D;
use rounds_sim::{PlayerInput, v2};

use crate::input::{Buttons, DeviceId, InputState};

/// Who controls a slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotControl {
    Device(DeviceId),
    /// Bound to the first controller that is not bound yet (a bot until
    /// one is connected).
    AnyController,
    Bot,
    /// Its controller was unplugged: the player stands still until a
    /// controller that is not bound yet appears (replugging gives the pad
    /// a new SDL instance id), which then takes the slot over.
    Orphan,
    /// Simulated on another machine (online driver).
    #[allow(dead_code)]
    Remote,
}

/// Upper stick dead zone (InControl default 0.9): above it reads as full.
const UPPER_DEADZONE: f32 = 0.9;
/// The Aim script ignores smaller aim vectors (player.md 4.1).
const AIM_THRESHOLD: f32 = 0.2;

fn upper(v: Vec2) -> Vec2 {
    (v / UPPER_DEADZONE).clamp_length_max(1.0)
}

/// Where the view is, for mouse aim: the camera and target size of the
/// last drawn frame.
#[derive(Clone, Copy, Debug)]
pub struct ViewInfo {
    pub camera: Camera2D,
    pub size: (u32, u32),
}

/// Sim input for one slot from its device state. `player_pos` is the
/// player's world position (for mouse aim).
pub fn device_input(
    device: DeviceId,
    s: &InputState,
    player_pos: Vec2,
    view: Option<&ViewInfo>,
) -> PlayerInput {
    let mut buttons = 0u16;
    // A tap shorter than a tick is only a press edge; it still counts for
    // this tick (player.md 3: inputs are sampled per fixed step).
    let held = |b: Buttons| s.held.contains(b) || s.pressed.contains(b);
    if held(Buttons::JUMP) {
        buttons |= PlayerInput::BTN_JUMP;
    }
    if held(Buttons::FIRE) {
        buttons |= PlayerInput::BTN_FIRE;
    }
    if held(Buttons::BLOCK) {
        buttons |= PlayerInput::BTN_BLOCK;
    }
    if held(Buttons::LEFT) {
        buttons |= PlayerInput::BTN_MENU_LEFT;
    }
    if held(Buttons::RIGHT) {
        buttons |= PlayerInput::BTN_MENU_RIGHT;
    }
    if held(Buttons::CONFIRM) {
        buttons |= PlayerInput::BTN_MENU_OK;
    }
    if held(Buttons::BACK) {
        buttons |= PlayerInput::BTN_MENU_BACK;
    }
    let mv = upper(s.move_axis);
    let aim = match device {
        DeviceId::KeyboardMouse => match (s.pointer, view) {
            (Some(px), Some(v)) => {
                let world = v.camera.screen_to_world(px, v.size);
                (world - player_pos).normalize_or_zero()
            }
            _ => Vec2::ZERO,
        },
        _ => {
            let a = upper(s.aim_axis);
            if a.length() > AIM_THRESHOLD {
                a
            } else {
                Vec2::ZERO
            }
        }
    };
    PlayerInput::from_axes(v2(mv.x, mv.y), v2(aim.x, aim.y), buttons)
}

/// [`device_input`] with the player options: aim snapped to 16
/// directions (ui.md 2.7 "8 dir" options), controllers aiming with the
/// left stick while the right one is centred (`OPTION_LEFTSTICKAIMING`).
pub fn device_input_with(
    device: DeviceId,
    s: &InputState,
    player_pos: Vec2,
    view: Option<&ViewInfo>,
    opts: &super::PlayOptions,
) -> PlayerInput {
    let mut st = *s;
    if let DeviceId::Controller(_) = device
        && opts.left_stick_aim
        && st.aim_axis.length() <= AIM_THRESHOLD
    {
        st.aim_axis = st.move_axis;
    }
    let mut i = device_input(device, &st, player_pos, view);
    let snap = match device {
        DeviceId::KeyboardMouse => opts.mouse_aim_snap,
        _ => opts.controller_aim_snap,
    };
    if snap {
        let a = i.aim_vec();
        let (x, y) = (a.x, a.y);
        if x != 0.0 || y != 0.0 {
            let step = std::f32::consts::TAU / 16.0;
            let ang = (y.atan2(x) / step).round() * step;
            let len = (x * x + y * y).sqrt().min(1.0);
            let q = PlayerInput::from_axes(v2(0.0, 0.0), v2(ang.cos() * len, ang.sin() * len), 0);
            i.aim_x = q.aim_x;
            i.aim_y = q.aim_y;
        }
    }
    i
}

/// Default slot assignment for `--play`: keyboard+mouse = player 1, the
/// keyboard-only scheme next if asked, every other slot takes the next
/// controller that is connected (a bot until then).
pub fn default_controls(players: usize, keyboard_only: bool) -> Vec<SlotControl> {
    let mut v = vec![SlotControl::Device(DeviceId::KeyboardMouse)];
    if keyboard_only {
        v.push(SlotControl::Device(DeviceId::KeyboardRight));
    }
    v.resize(players.max(v.len()), SlotControl::AnyController);
    v.truncate(players);
    v
}

/// Binds `AnyController` slots to connected controllers not bound yet.
pub fn bind_controllers(controls: &mut [SlotControl], devices: &[DeviceId]) {
    // Unplugged controllers orphan their slots.
    for c in controls.iter_mut() {
        if let SlotControl::Device(d @ DeviceId::Controller(_)) = *c
            && !devices.contains(&d)
        {
            log::info!("controller {d:?} lost; its player waits for a controller");
            *c = SlotControl::Orphan;
        }
    }
    for d in devices {
        if !matches!(d, DeviceId::Controller(_)) || controls.contains(&SlotControl::Device(*d)) {
            continue;
        }
        // Orphaned players first, then slots waiting for any controller.
        let slot = controls
            .iter()
            .position(|c| *c == SlotControl::Orphan)
            .or_else(|| {
                controls
                    .iter()
                    .position(|c| *c == SlotControl::AnyController)
            });
        if let Some(i) = slot {
            controls[i] = SlotControl::Device(*d);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unplugged_controller_slot_rebinds() {
        let (a, b) = (DeviceId::Controller(1), DeviceId::Controller(7));
        let mut c = vec![
            SlotControl::Device(DeviceId::KeyboardMouse),
            SlotControl::Device(a),
        ];
        bind_controllers(&mut c, &[DeviceId::KeyboardMouse]);
        assert_eq!(c[1], SlotControl::Orphan);
        bind_controllers(&mut c, &[DeviceId::KeyboardMouse, b]);
        assert_eq!(c[1], SlotControl::Device(b));
        assert_eq!(c[0], SlotControl::Device(DeviceId::KeyboardMouse));
    }

    #[test]
    fn mouse_aim_points_from_player_to_cursor() {
        let view = ViewInfo {
            camera: Camera2D::new(Vec2::ZERO, 20.0),
            size: (1920, 1080),
        };
        let s = InputState {
            pointer: Some(Vec2::new(1920.0, 540.0)), // right edge, centre height
            ..Default::default()
        };
        let i = device_input(DeviceId::KeyboardMouse, &s, Vec2::ZERO, Some(&view));
        let a = i.aim_vec();
        assert!(a.x > 0.99 && a.y.abs() < 0.01, "{a:?}");
    }

    #[test]
    fn stick_dead_zones() {
        let s = InputState {
            aim_axis: Vec2::new(0.1, 0.0),
            move_axis: Vec2::new(0.95, 0.0),
            ..Default::default()
        };
        let i = device_input(DeviceId::Controller(0), &s, Vec2::ZERO, None);
        assert_eq!(i.aim_vec().x, 0.0);
        assert_eq!(i.move_x, 127);
    }

    #[test]
    fn slots() {
        let mut c = default_controls(3, false);
        assert_eq!(c[0], SlotControl::Device(DeviceId::KeyboardMouse));
        assert_eq!(c[1], SlotControl::AnyController);
        bind_controllers(&mut c, &[DeviceId::KeyboardMouse, DeviceId::Controller(7)]);
        assert_eq!(c[1], SlotControl::Device(DeviceId::Controller(7)));
        assert_eq!(c[2], SlotControl::AnyController);
    }
}
