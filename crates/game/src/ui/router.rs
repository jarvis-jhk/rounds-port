//! Device inputs -> menu navigation (ui.md 2.2): per device edges with
//! key repeat (first repeat after 0.35 s, then every 0.2 s / 0.15 s for
//! grids), sticks count past 0.5, mouse vs controller mode, text input.

use std::collections::BTreeMap;

use glam::Vec2;

use super::Canvas;
use crate::input::{Buttons, DeviceId, InputState};

/// One device's navigation this tick.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DevNav {
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
    pub confirm: bool,
    pub back: bool,
    pub start: bool,
    pub edit: bool,
    pub tab: bool,
    pub prev: bool,
    pub next: bool,
    /// Jump / A / Space went down (join, ready).
    pub jump: bool,
    /// Any button went down.
    pub any: bool,
}

impl DevNav {
    fn or(&mut self, o: &DevNav) {
        self.up |= o.up;
        self.down |= o.down;
        self.left |= o.left;
        self.right |= o.right;
        self.confirm |= o.confirm;
        self.back |= o.back;
        self.start |= o.start;
        self.edit |= o.edit;
        self.tab |= o.tab;
        self.prev |= o.prev;
        self.next |= o.next;
        self.jump |= o.jump;
        self.any |= o.any;
    }
}

/// Everything a menu needs from the input this tick.
#[derive(Clone, Debug, Default)]
pub struct UiInput {
    /// Mouse pointer in reference coordinates.
    pub pointer: Option<Vec2>,
    pub click: bool,
    pub right_click: bool,
    pub mouse_held: bool,
    pub right_held: bool,
    pub wheel: f32,
    /// Last input was the mouse (hide the bar, hover selects).
    pub mouse_mode: bool,
    /// Per device (in device order).
    pub devs: Vec<(DeviceId, DevNav)>,
    /// All devices merged.
    pub nav: DevNav,
    pub text: String,
    pub backspace: bool,
    /// Raw device states (lobby).
    pub states: Vec<(DeviceId, InputState)>,
}

impl UiInput {
    pub fn dev(&self, d: DeviceId) -> DevNav {
        self.devs
            .iter()
            .find(|(id, _)| *id == d)
            .map(|x| x.1)
            .unwrap_or_default()
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Repeat {
    dir: (i8, i8),
    /// Ticks until the next repeat.
    wait: u32,
}

/// Keeps per-device repeat state and the mouse mode.
#[derive(Default)]
pub struct Router {
    repeat: BTreeMap<DeviceId, Repeat>,
    last_pointer: Option<Vec2>,
    mouse_mode: bool,
    backspace_wait: u32,
    /// Repeat interval in ticks (0.2 s; grids use 0.15 s).
    pub interval: u32,
}

const FIRST_REPEAT: u32 = 25; // 0.35 s
const REPEAT: u32 = 14; // 0.2 s

impl Router {
    pub fn new() -> Router {
        Router {
            mouse_mode: true,
            interval: REPEAT,
            ..Default::default()
        }
    }

    /// Builds the menu input. `text_mode`: a text field has focus, so
    /// keyboards only navigate with arrows/Enter/Esc/Tab.
    pub fn update(
        &mut self,
        inputs: &[(DeviceId, InputState)],
        text: &str,
        canvas: &Canvas,
        text_mode: bool,
    ) -> UiInput {
        let mut out = UiInput {
            text: if text_mode {
                text.to_owned()
            } else {
                String::new()
            },
            states: inputs.to_vec(),
            ..Default::default()
        };
        let mut nav_input = false;
        for (id, s) in inputs {
            let pressed = |b: Buttons| s.pressed.contains(b);
            let held = |b: Buttons| s.held.contains(b);
            if *id == DeviceId::KeyboardMouse {
                out.pointer = s.pointer.map(|p| canvas.to_ref(p));
                out.click = pressed(Buttons::FIRE);
                out.right_click = pressed(Buttons::BLOCK);
                out.mouse_held = held(Buttons::FIRE);
                out.right_held = held(Buttons::BLOCK);
                out.wheel = s.wheel;
                let moved = s.pointer.is_some()
                    && self
                        .last_pointer
                        .is_some_and(|l| (l - s.pointer.unwrap()).length() > 2.0);
                if self.last_pointer.is_none() && s.pointer.is_some() {
                    self.last_pointer = s.pointer;
                }
                if moved || out.click || out.right_click || s.wheel != 0.0 {
                    self.mouse_mode = true;
                    self.last_pointer = s.pointer;
                }
                if text_mode {
                    out.backspace = pressed(Buttons::BACKSPACE);
                    if held(Buttons::BACKSPACE) {
                        if self.backspace_wait == 0 {
                            out.backspace = true;
                            self.backspace_wait = 3;
                        } else {
                            self.backspace_wait -= 1;
                        }
                    } else {
                        self.backspace_wait = FIRST_REPEAT;
                    }
                    if pressed(Buttons::BACKSPACE) {
                        self.backspace_wait = FIRST_REPEAT;
                    }
                }
            }
            // Directions: buttons (WASD, arrows, D-pad) or the stick.
            let keyboard = id.is_keyboard();
            let mut dx = 0i8;
            let mut dy = 0i8;
            let use_dirs =
                !(text_mode && matches!(id, DeviceId::KeyboardMouse | DeviceId::KeyboardLeft));
            if use_dirs {
                if held(Buttons::LEFT) {
                    dx -= 1;
                }
                if held(Buttons::RIGHT) {
                    dx += 1;
                }
                if held(Buttons::UP) {
                    dy -= 1;
                }
                if held(Buttons::DOWN) {
                    dy += 1;
                }
                if !keyboard {
                    let m = s.move_axis;
                    if m.x.abs() > 0.5 && m.x.abs() >= m.y.abs() {
                        dx = m.x.signum() as i8;
                    }
                    if m.y.abs() > 0.5 && m.y.abs() > m.x.abs() {
                        dy = -m.y.signum() as i8;
                    }
                }
            }
            let r = self.repeat.entry(*id).or_default();
            let mut fire = (0i8, 0i8);
            if (dx, dy) != r.dir {
                // New direction: fire the changed axes now.
                fire = (
                    if dx != r.dir.0 { dx } else { 0 },
                    if dy != r.dir.1 { dy } else { 0 },
                );
                r.dir = (dx, dy);
                r.wait = FIRST_REPEAT;
            } else if (dx, dy) != (0, 0) {
                if r.wait == 0 {
                    fire = (dx, dy);
                    r.wait = self.interval;
                } else {
                    r.wait -= 1;
                }
            }
            if use_dirs {
                // Taps shorter than a tick (fast typing, slow frames, injected
                // input) are only visible as press edges.
                let tap = |b: Buttons| pressed(b) && !held(b);
                if fire.0 == 0 {
                    fire.0 = tap(Buttons::RIGHT) as i8 - tap(Buttons::LEFT) as i8;
                }
                if fire.1 == 0 {
                    fire.1 = tap(Buttons::DOWN) as i8 - tap(Buttons::UP) as i8;
                }
            }
            let text_kb = text_mode && keyboard;
            let mut n = DevNav {
                left: fire.0 < 0,
                right: fire.0 > 0,
                up: fire.1 < 0,
                down: fire.1 > 0,
                confirm: pressed(Buttons::CONFIRM)
                    || (!keyboard && pressed(Buttons::JUMP))
                    || (!text_kb && *id == DeviceId::KeyboardMouse && pressed(Buttons::JUMP)),
                back: pressed(Buttons::BACK) && !(text_kb && *id == DeviceId::KeyboardRight),
                start: pressed(Buttons::START) && !keyboard
                    || (keyboard && pressed(Buttons::CONFIRM)),
                edit: !text_kb && pressed(Buttons::EDIT),
                tab: pressed(Buttons::TAB),
                prev: !text_kb && pressed(Buttons::PREV),
                next: !text_kb && pressed(Buttons::NEXT),
                jump: !text_kb && pressed(Buttons::JUMP),
                any: s.pressed.0 != 0,
            };
            if keyboard && text_mode && *id != DeviceId::KeyboardRight {
                // Letters type; only Enter/Esc/Tab act.
                n.left = false;
                n.right = false;
                n.up = false;
                n.down = false;
            }
            if fire != (0, 0) || (n.any && !(*id == DeviceId::KeyboardMouse && out.click)) {
                nav_input |= fire != (0, 0) || n.confirm || n.back || n.start;
            }
            out.nav.or(&n);
            out.devs.push((*id, n));
        }
        if nav_input {
            self.mouse_mode = false;
        }
        out.mouse_mode = self.mouse_mode;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::Buttons;

    fn state(held: Buttons, pressed: Buttons) -> InputState {
        InputState {
            held,
            pressed,
            ..Default::default()
        }
    }

    #[test]
    fn repeat_and_merge() {
        let mut r = Router::new();
        let c = Canvas::new((1920, 1080));
        let pad = DeviceId::Controller(3);
        let down = |p: bool| {
            vec![(
                pad,
                state(
                    Buttons::DOWN,
                    if p { Buttons::DOWN } else { Buttons::default() },
                ),
            )]
        };
        let first = r.update(&down(true), "", &c, false);
        assert!(first.nav.down && first.dev(pad).down);
        assert!(!first.mouse_mode, "controller input leaves mouse mode");
        // Held: no event until the first repeat, then every interval.
        let mut events = 0;
        for _ in 0..(FIRST_REPEAT + 1 + (REPEAT + 1) * 2) {
            if r.update(&down(false), "", &c, false).nav.down {
                events += 1;
            }
        }
        assert_eq!(events, 3);
    }

    #[test]
    fn text_mode_letters_do_not_navigate() {
        let mut r = Router::new();
        let c = Canvas::new((1920, 1080));
        let kb = vec![(
            DeviceId::KeyboardMouse,
            state(Buttons::UP | Buttons::JUMP, Buttons::UP | Buttons::JUMP),
        )];
        let i = r.update(&kb, "w ", &c, true);
        assert!(!i.nav.up && !i.nav.confirm);
        assert_eq!(i.text, "w ");
        let i = r.update(&kb, "x", &c, false);
        assert!(i.text.is_empty());
    }
}
