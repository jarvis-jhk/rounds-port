//! Input devices: keyboard + mouse, two keyboard-only halves and any number
//! of SDL game controllers (hot-plugged), each turned into a normalised
//! [`InputState`] per tick.
//!
//! Bindings follow player.md 3.2: keyboard+mouse = A/D/W/S move, Space
//! jump, left mouse fire, right mouse block, Return start; controllers =
//! left stick move, right stick aim, A/LB/RB jump, X/RT fire, B/LT block,
//! Y edit, Start start, Back back. Port schemes for two players sharing a
//! keyboard (docs/platform.md "Input"): [`DeviceId::KeyboardLeft`] = WASD
//! move/aim, Space jump, F fire, G block (the same keys as keyboard+mouse,
//! so only one of the two plays); [`DeviceId::KeyboardRight`] = arrows
//! move, I/J/K/L aim, Right Shift or `/` jump, `.` or Right Ctrl fire, `,`
//! or Right Alt block.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use glam::Vec2;
use sdl2::controller::{Axis, Button, GameController};
use sdl2::event::Event;
use sdl2::keyboard::Scancode;
use sdl2::mouse::MouseButton;

/// A local input device.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DeviceId {
    KeyboardMouse,
    /// Keyboard-only, left half (WASD, Space, F, G); excludes
    /// `KeyboardMouse` (same keys).
    KeyboardLeft,
    /// Keyboard-only, right half (arrows, I/J/K/L aim, right-hand action
    /// keys); see `docs/platform.md`.
    KeyboardRight,
    /// SDL joystick instance id (stable while the controller is plugged in).
    Controller(u32),
}

impl DeviceId {
    pub fn is_keyboard(self) -> bool {
        !matches!(self, DeviceId::Controller(_))
    }

    /// Short label for the lobby.
    pub fn label(self) -> &'static str {
        match self {
            DeviceId::KeyboardMouse => "Keyboard + mouse",
            DeviceId::KeyboardLeft => "Keyboard (left)",
            DeviceId::KeyboardRight => "Keyboard (right)",
            DeviceId::Controller(_) => "Controller",
        }
    }
}

/// A remappable action (Options, Controls; persisted in `settings.json`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Action {
    Jump,
    Fire,
    Block,
    Left,
    Right,
    Up,
    Down,
    Edit,
}

pub const ACTIONS: [Action; 8] = [
    Action::Jump,
    Action::Fire,
    Action::Block,
    Action::Left,
    Action::Right,
    Action::Up,
    Action::Down,
    Action::Edit,
];

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Action::Jump => "JUMP",
            Action::Fire => "SHOOT",
            Action::Block => "BLOCK",
            Action::Left => "MOVE LEFT",
            Action::Right => "MOVE RIGHT",
            Action::Up => "MOVE UP",
            Action::Down => "MOVE DOWN",
            Action::Edit => "EDIT FACE",
        }
    }

    /// Controllers move with the left stick (not remappable).
    pub fn is_move(self) -> bool {
        matches!(
            self,
            Action::Left | Action::Right | Action::Up | Action::Down
        )
    }
}

/// An input scheme with its own bindings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Scheme {
    KeyboardMouse,
    KeyboardLeft,
    KeyboardRight,
    Controller,
}

pub const SCHEMES: [Scheme; 4] = [
    Scheme::KeyboardMouse,
    Scheme::KeyboardLeft,
    Scheme::KeyboardRight,
    Scheme::Controller,
];

impl Scheme {
    pub fn label(self) -> &'static str {
        match self {
            Scheme::KeyboardMouse => "KEYBOARD + MOUSE",
            Scheme::KeyboardLeft => "KEYBOARD LEFT",
            Scheme::KeyboardRight => "KEYBOARD RIGHT",
            Scheme::Controller => "CONTROLLER",
        }
    }
}

/// Key / button names per scheme and action: SDL scancode names
/// (`Space`, `Right Shift`, ...) and `Mouse1`..`Mouse3` for keyboards, SDL
/// game controller button names (`a`, `leftshoulder`, ...) plus
/// `lefttrigger` / `righttrigger` for controllers. Missing entries use the
/// defaults (docs/platform.md "Input devices and schemes").
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Bindings {
    pub map: BTreeMap<Scheme, BTreeMap<Action, Vec<String>>>,
}

impl Default for Bindings {
    fn default() -> Self {
        use Action::*;
        let mut map = BTreeMap::new();
        let table = |rows: &[(Action, &[&str])]| -> BTreeMap<Action, Vec<String>> {
            rows.iter()
                .map(|(a, ks)| (*a, ks.iter().map(|k| k.to_string()).collect()))
                .collect()
        };
        map.insert(
            Scheme::KeyboardMouse,
            table(&[
                (Jump, &["Space"]),
                (Fire, &["Mouse1"]),
                (Block, &["Mouse2"]),
                (Left, &["A"]),
                (Right, &["D"]),
                (Up, &["W"]),
                (Down, &["S"]),
                (Edit, &["E"]),
            ]),
        );
        map.insert(
            Scheme::KeyboardLeft,
            table(&[
                (Jump, &["Space"]),
                (Fire, &["F"]),
                (Block, &["G"]),
                (Left, &["A"]),
                (Right, &["D"]),
                (Up, &["W"]),
                (Down, &["S"]),
                (Edit, &["E"]),
            ]),
        );
        map.insert(
            Scheme::KeyboardRight,
            table(&[
                (Jump, &["Right Shift", "/", "Keypad 0"]),
                (Fire, &[".", "Right Ctrl"]),
                (Block, &[",", "Right Alt"]),
                (Left, &["Left"]),
                (Right, &["Right"]),
                (Up, &["Up"]),
                (Down, &["Down"]),
                (Edit, &["'", "Keypad +"]),
            ]),
        );
        map.insert(
            Scheme::Controller,
            table(&[
                (Jump, &["a", "leftshoulder", "rightshoulder"]),
                (Fire, &["x", "righttrigger"]),
                (Block, &["b", "lefttrigger"]),
                (Edit, &["y"]),
            ]),
        );
        Bindings { map }
    }
}

impl Bindings {
    /// The names bound to `a` in scheme `s` (defaults when unset).
    pub fn get(&self, s: Scheme, a: Action) -> Vec<String> {
        self.map
            .get(&s)
            .and_then(|m| m.get(&a))
            .cloned()
            .unwrap_or_else(|| {
                Bindings::default()
                    .map
                    .get(&s)
                    .and_then(|m| m.get(&a))
                    .cloned()
                    .unwrap_or_default()
            })
    }

    pub fn set(&mut self, s: Scheme, a: Action, names: Vec<String>) {
        self.map.entry(s).or_default().insert(a, names);
    }

    /// Display text of a binding (`SPACE / RIGHT SHIFT`).
    pub fn describe(&self, s: Scheme, a: Action) -> String {
        if s == Scheme::Controller && a.is_move() {
            return "LEFT STICK".into();
        }
        let v = self.get(s, a);
        if v.is_empty() {
            return "-".into();
        }
        v.iter()
            .map(|n| match n.as_str() {
                "Mouse1" => "LEFT MOUSE".to_string(),
                "Mouse2" => "RIGHT MOUSE".to_string(),
                "Mouse3" => "MIDDLE MOUSE".to_string(),
                "leftshoulder" => "LB".into(),
                "rightshoulder" => "RB".into(),
                "lefttrigger" => "LT".into(),
                "righttrigger" => "RT".into(),
                "leftstick" => "L3".into(),
                "rightstick" => "R3".into(),
                "/" => "SLASH".into(),
                "." => "PERIOD".into(),
                "," => "COMMA".into(),
                "'" => "APOSTROPHE".into(),
                ";" => "SEMICOLON".into(),
                "-" => "MINUS".into(),
                "=" => "EQUALS".into(),
                o => o.to_uppercase(),
            })
            .collect::<Vec<_>>()
            .join(" / ")
    }
}

/// A key, mouse button, or controller button that went down (for binding
/// capture in the Controls page).
#[derive(Clone, Debug, PartialEq)]
pub struct RawPress {
    /// `KeyboardMouse` for keys and mouse buttons, else the controller.
    pub device: DeviceId,
    /// Binding name (see [`Bindings`]).
    pub name: String,
}

/// A resolved keyboard binding.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Key {
    Scan(Scancode),
    Mouse(usize),
}

fn keys_of(b: &Bindings, s: Scheme, a: Action) -> Vec<Key> {
    b.get(s, a)
        .iter()
        .filter_map(|n| match n.as_str() {
            "Mouse1" => Some(Key::Mouse(0)),
            "Mouse2" => Some(Key::Mouse(1)),
            "Mouse3" => Some(Key::Mouse(2)),
            n => Scancode::from_name(n).map(Key::Scan),
        })
        .collect()
}

/// Logical buttons, as a bit set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Buttons(pub u32);

impl Buttons {
    pub const JUMP: Buttons = Buttons(1 << 0);
    pub const FIRE: Buttons = Buttons(1 << 1);
    pub const BLOCK: Buttons = Buttons(1 << 2);
    pub const CONFIRM: Buttons = Buttons(1 << 3);
    pub const BACK: Buttons = Buttons(1 << 4);
    pub const START: Buttons = Buttons(1 << 5);
    pub const UP: Buttons = Buttons(1 << 6);
    pub const DOWN: Buttons = Buttons(1 << 7);
    pub const LEFT: Buttons = Buttons(1 << 8);
    pub const RIGHT: Buttons = Buttons(1 << 9);
    /// Edit (controller Y, keyboard E / right-hand `'`): face editor.
    pub const EDIT: Buttons = Buttons(1 << 10);
    /// Keyboard: Tab (switch keyboard scheme, next field).
    pub const TAB: Buttons = Buttons(1 << 11);
    /// Keyboard: Backspace (text fields).
    pub const BACKSPACE: Buttons = Buttons(1 << 12);
    /// Controller shoulders / keyboard Q and E: previous / next tab.
    pub const PREV: Buttons = Buttons(1 << 13);
    pub const NEXT: Buttons = Buttons(1 << 14);

    pub fn contains(self, b: Buttons) -> bool {
        self.0 & b.0 == b.0
    }

    pub fn set(&mut self, b: Buttons, on: bool) {
        if on {
            self.0 |= b.0;
        } else {
            self.0 &= !b.0;
        }
    }
}

impl std::ops::BitOr for Buttons {
    type Output = Buttons;
    fn bitor(self, o: Buttons) -> Buttons {
        Buttons(self.0 | o.0)
    }
}

/// One device's input for one simulation tick. Axes are y-up and clamped
/// to the unit disk after the dead zone.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct InputState {
    pub move_axis: Vec2,
    /// Stick aim direction (controllers); zero when centred.
    pub aim_axis: Vec2,
    /// Mouse position in window pixels (keyboard+mouse only).
    pub pointer: Option<Vec2>,
    pub held: Buttons,
    /// Went down since the previous tick (taps shorter than a tick count).
    pub pressed: Buttons,
    pub released: Buttons,
    /// Analogue triggers 0..1 (left, right).
    pub triggers: [f32; 2],
    /// Mouse wheel steps since the previous tick (keyboard+mouse only).
    pub wheel: f32,
}

/// Radial dead zone with rescaling, then clamp to the unit disk.
pub fn deadzone(v: Vec2, dz: f32) -> Vec2 {
    let len = v.length();
    if len <= dz {
        return Vec2::ZERO;
    }
    let scaled = (len - dz) / (1.0 - dz);
    v / len * scaled.min(1.0)
}

const STICK_DEADZONE: f32 = 0.2;
const TRIGGER_THRESHOLD: f32 = 0.35;

fn axis_value(raw: i16) -> f32 {
    (raw as f32 / 32767.0).clamp(-1.0, 1.0)
}

struct DeviceSlot {
    held: Buttons,
    latched_press: Buttons,
    latched_release: Buttons,
    last_held: Buttons,
}

impl DeviceSlot {
    fn new() -> Self {
        Self {
            held: Buttons::default(),
            latched_press: Buttons::default(),
            latched_release: Buttons::default(),
            last_held: Buttons::default(),
        }
    }

    fn set(&mut self, b: Buttons, down: bool) {
        if down && !self.held.contains(b) {
            self.latched_press.set(b, true);
        }
        if !down && self.held.contains(b) {
            self.latched_release.set(b, true);
        }
        self.held.set(b, down);
    }

    fn take_edges(&mut self) -> (Buttons, Buttons) {
        let pressed = Buttons(self.latched_press.0 | (self.held.0 & !self.last_held.0));
        let released = Buttons(self.latched_release.0 | (!self.held.0 & self.last_held.0));
        self.latched_press = Buttons::default();
        self.latched_release = Buttons::default();
        self.last_held = self.held;
        (pressed, released)
    }
}

/// Info about a connected controller, for menus and `--list-devices`.
#[derive(Clone, Debug)]
pub struct ControllerInfo {
    pub id: DeviceId,
    pub name: String,
    pub has_rumble: bool,
}

/// Tracks every local device.
pub struct InputHub {
    subsystem: sdl2::GameControllerSubsystem,
    controllers: BTreeMap<u32, (GameController, DeviceSlot)>,
    keyboard: DeviceSlot,
    keyboard_left: DeviceSlot,
    keyboard_right: DeviceSlot,
    keys_down: std::collections::HashSet<Scancode>,
    mouse_buttons: [bool; 3],
    pointer: Option<Vec2>,
    wheel: f32,
    /// Text typed since the last [`InputHub::take_text`].
    text: String,
    bindings: Bindings,
    /// Resolved keyboard bindings per scheme (KeyboardMouse, Left, Right).
    keys: [BTreeMap<Action, Vec<Key>>; 3],
    /// Presses since the last [`InputHub::take_raw`].
    raw: Vec<RawPress>,
    /// Trigger state per controller (for raw trigger presses).
    trig_down: BTreeMap<u32, [bool; 2]>,
}

#[allow(dead_code)] // `rumble` is not used yet.
impl InputHub {
    pub fn new(subsystem: sdl2::GameControllerSubsystem) -> Self {
        // Optional extra mappings next to the executable (SDL's built-in
        // database covers common pads).
        if let Some(path) = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("gamecontrollerdb.txt")))
            .filter(|p| p.exists())
        {
            match subsystem.load_mappings(&path) {
                Ok(n) => log::info!("loaded {n} controller mappings from {}", path.display()),
                Err(e) => log::warn!("controller mappings {}: {e}", path.display()),
            }
        }
        Self {
            subsystem,
            controllers: BTreeMap::new(),
            keyboard: DeviceSlot::new(),
            keyboard_left: DeviceSlot::new(),
            keyboard_right: DeviceSlot::new(),
            keys_down: Default::default(),
            mouse_buttons: [false; 3],
            pointer: None,
            wheel: 0.0,
            text: String::new(),
            bindings: Bindings::default(),
            keys: Default::default(),
            raw: Vec::new(),
            trig_down: BTreeMap::new(),
        }
        .with_bindings(Bindings::default())
    }

    fn with_bindings(mut self, b: Bindings) -> Self {
        self.set_bindings(b);
        self
    }

    /// Applies remapped bindings (Options, Controls).
    pub fn set_bindings(&mut self, b: Bindings) {
        for (i, s) in [
            Scheme::KeyboardMouse,
            Scheme::KeyboardLeft,
            Scheme::KeyboardRight,
        ]
        .into_iter()
        .enumerate()
        {
            self.keys[i] = ACTIONS.iter().map(|&a| (a, keys_of(&b, s, a))).collect();
        }
        self.bindings = b;
        self.update_keyboard();
    }

    /// Key / button presses since the last call.
    pub fn take_raw(&mut self) -> Vec<RawPress> {
        std::mem::take(&mut self.raw)
    }

    /// Feeds one SDL event. Returns true if it was an input event.
    pub fn handle_event(&mut self, event: &Event) -> bool {
        match event {
            Event::ControllerDeviceAdded { which, .. } => match self.subsystem.open(*which) {
                Ok(c) => {
                    let id = c.instance_id();
                    log::info!("controller connected: #{id} {}", c.name());
                    self.controllers.entry(id).or_insert((c, DeviceSlot::new()));
                }
                Err(e) => log::warn!("cannot open controller {which}: {e}"),
            },
            Event::ControllerDeviceRemoved { which, .. } => {
                if let Some((c, _)) = self.controllers.remove(which) {
                    log::info!("controller disconnected: #{which} {}", c.name());
                }
            }
            Event::ControllerButtonDown { which, button, .. }
            | Event::ControllerButtonUp { which, button, .. } => {
                let down = matches!(event, Event::ControllerButtonDown { .. });
                if down {
                    self.raw.push(RawPress {
                        device: DeviceId::Controller(*which),
                        name: button.string(),
                    });
                }
                if let Some((_, slot)) = self.controllers.get_mut(which) {
                    for b in controller_button(*button) {
                        slot.set(b, down);
                    }
                }
            }
            Event::KeyDown {
                scancode: Some(sc),
                repeat: false,
                ..
            } => {
                self.keys_down.insert(*sc);
                self.raw.push(RawPress {
                    device: DeviceId::KeyboardMouse,
                    name: sc.name().to_string(),
                });
                self.update_keyboard();
            }
            Event::KeyUp {
                scancode: Some(sc), ..
            } => {
                self.keys_down.remove(sc);
                self.update_keyboard();
            }
            Event::MouseButtonDown { mouse_btn, .. } | Event::MouseButtonUp { mouse_btn, .. } => {
                let down = matches!(event, Event::MouseButtonDown { .. });
                let i = match mouse_btn {
                    MouseButton::Left => 0,
                    MouseButton::Right => 1,
                    MouseButton::Middle => 2,
                    _ => return true,
                };
                self.mouse_buttons[i] = down;
                if down {
                    self.raw.push(RawPress {
                        device: DeviceId::KeyboardMouse,
                        name: format!("Mouse{}", i + 1),
                    });
                }
                self.update_keyboard();
            }
            Event::MouseMotion { x, y, .. } => self.pointer = Some(Vec2::new(*x as f32, *y as f32)),
            Event::MouseWheel { y, .. } => self.wheel += *y as f32,
            Event::TextInput { text, .. } => self.text.push_str(text),
            _ => return false,
        }
        true
    }

    fn update_keyboard(&mut self) {
        let k = |s: &[Scancode]| s.iter().any(|sc| self.keys_down.contains(sc));
        let bound = |scheme: usize, a: Action| {
            self.keys[scheme].get(&a).is_some_and(|ks| {
                ks.iter().any(|key| match key {
                    Key::Scan(sc) => self.keys_down.contains(sc),
                    Key::Mouse(m) => self.mouse_buttons[*m],
                })
            })
        };
        let actions = |scheme: usize| -> [bool; 8] { ACTIONS.map(|a| bound(scheme, a)) };
        let (km, kl, kr) = (actions(0), actions(1), actions(2));
        let set_actions = |slot: &mut DeviceSlot, v: [bool; 8]| {
            slot.set(Buttons::JUMP, v[0]);
            slot.set(Buttons::FIRE, v[1]);
            slot.set(Buttons::BLOCK, v[2]);
            slot.set(Buttons::LEFT, v[3]);
            slot.set(Buttons::RIGHT, v[4]);
            slot.set(Buttons::UP, v[5]);
            slot.set(Buttons::DOWN, v[6]);
            slot.set(Buttons::EDIT, v[7]);
        };
        let slot = &mut self.keyboard;
        set_actions(slot, km);
        slot.set(Buttons::CONFIRM, k(&[Scancode::Return, Scancode::KpEnter]));
        slot.set(Buttons::BACK, k(&[Scancode::Escape]));
        slot.set(Buttons::START, k(&[Scancode::Escape, Scancode::Return]));
        slot.set(Buttons::TAB, k(&[Scancode::Tab]));
        slot.set(Buttons::BACKSPACE, k(&[Scancode::Backspace]));
        slot.set(Buttons::PREV, k(&[Scancode::Q]));
        slot.set(Buttons::NEXT, k(&[Scancode::E]));
        let slot = &mut self.keyboard_left;
        set_actions(slot, kl);
        slot.set(Buttons::CONFIRM, k(&[Scancode::Return]));
        slot.set(Buttons::BACK, k(&[Scancode::Escape]));
        slot.set(Buttons::START, k(&[Scancode::Escape, Scancode::Return]));
        slot.set(Buttons::TAB, k(&[Scancode::Tab]));
        slot.set(Buttons::PREV, k(&[Scancode::Q]));
        slot.set(Buttons::NEXT, k(&[Scancode::E]));
        let slot = &mut self.keyboard_right;
        set_actions(slot, kr);
        slot.set(Buttons::CONFIRM, kr[0] || k(&[Scancode::KpEnter]));
        slot.set(Buttons::BACK, k(&[Scancode::Delete, Scancode::Backspace]));
        slot.set(Buttons::START, k(&[Scancode::KpEnter]) || kr[0]);
    }

    /// Text typed since the last call (SDL text input events).
    pub fn take_text(&mut self) -> String {
        std::mem::take(&mut self.text)
    }

    /// Connected devices: keyboard+mouse first, then controllers by id.
    pub fn devices(&self) -> Vec<DeviceId> {
        std::iter::once(DeviceId::KeyboardMouse)
            .chain(self.controllers.keys().map(|&i| DeviceId::Controller(i)))
            .collect()
    }

    pub fn controllers(&self) -> Vec<ControllerInfo> {
        self.controllers
            .iter()
            .map(|(&i, (c, _))| ControllerInfo {
                id: DeviceId::Controller(i),
                name: c.name(),
                has_rumble: c.has_rumble(),
            })
            .collect()
    }

    /// Rumble a controller (0..1 strengths); ignored when unsupported.
    pub fn rumble(&mut self, id: DeviceId, low: f32, high: f32, ms: u32) {
        if let DeviceId::Controller(i) = id
            && let Some((c, _)) = self.controllers.get_mut(&i)
        {
            let q = |x: f32| (x.clamp(0.0, 1.0) * 65535.0) as u16;
            let _ = c.set_rumble(q(low), q(high), ms);
        }
    }

    /// Samples every device for one tick (consumes press/release edges).
    pub fn snapshot(&mut self, out: &mut Vec<(DeviceId, InputState)>) {
        out.clear();
        // Keyboard + mouse.
        let k = |s: &[Scancode]| s.iter().any(|sc| self.keys_down.contains(sc)) as i32 as f32;
        let axis = |slot: &DeviceSlot| {
            let h = |b: Buttons| slot.held.contains(b) as i32 as f32;
            Vec2::new(
                h(Buttons::RIGHT) - h(Buttons::LEFT),
                h(Buttons::UP) - h(Buttons::DOWN),
            )
        };
        let mv = axis(&self.keyboard);
        let mv_left = axis(&self.keyboard_left);
        let (pressed, released) = self.keyboard.take_edges();
        out.push((
            DeviceId::KeyboardMouse,
            InputState {
                move_axis: mv.clamp_length_max(1.0),
                aim_axis: Vec2::ZERO,
                pointer: self.pointer,
                held: self.keyboard.held,
                pressed,
                released,
                triggers: [
                    self.mouse_buttons[1] as i32 as f32,
                    self.mouse_buttons[0] as i32 as f32,
                ],
                wheel: std::mem::take(&mut self.wheel),
            },
        ));
        // Keyboard only, left half: aims with the move keys.
        let (pressed, released) = self.keyboard_left.take_edges();
        out.push((
            DeviceId::KeyboardLeft,
            InputState {
                move_axis: mv_left.clamp_length_max(1.0),
                aim_axis: mv_left.normalize_or_zero(),
                pointer: None,
                held: self.keyboard_left.held,
                pressed,
                released,
                triggers: [0.0; 2],
                wheel: 0.0,
            },
        ));
        // Keyboard only, right half.
        let mv = axis(&self.keyboard_right);
        let aim = Vec2::new(
            k(&[Scancode::L]) - k(&[Scancode::J]),
            k(&[Scancode::I]) - k(&[Scancode::K]),
        );
        let aim = if aim == Vec2::ZERO { mv } else { aim };
        let (pressed, released) = self.keyboard_right.take_edges();
        out.push((
            DeviceId::KeyboardRight,
            InputState {
                move_axis: mv.clamp_length_max(1.0),
                aim_axis: aim.normalize_or_zero(),
                pointer: None,
                held: self.keyboard_right.held,
                pressed,
                released,
                triggers: [0.0; 2],
                wheel: 0.0,
            },
        ));
        // Controllers: axes are polled, buttons come from events.
        for (&id, (c, slot)) in &mut self.controllers {
            let stick = |x, y| {
                deadzone(
                    Vec2::new(axis_value(c.axis(x)), -axis_value(c.axis(y))),
                    STICK_DEADZONE,
                )
            };
            let lt = axis_value(c.axis(Axis::TriggerLeft)).max(0.0);
            let rt = axis_value(c.axis(Axis::TriggerRight)).max(0.0);
            let trig = [lt > TRIGGER_THRESHOLD, rt > TRIGGER_THRESHOLD];
            let was = self.trig_down.entry(id).or_default();
            for (i, name) in ["lefttrigger", "righttrigger"].iter().enumerate() {
                if trig[i] && !was[i] {
                    self.raw.push(RawPress {
                        device: DeviceId::Controller(id),
                        name: (*name).into(),
                    });
                }
            }
            *was = trig;
            let held = |names: &[String]| {
                names.iter().any(|n| match n.as_str() {
                    "lefttrigger" => trig[0],
                    "righttrigger" => trig[1],
                    n => Button::from_string(n).is_some_and(|b| c.button(b)),
                })
            };
            let bd = &self.bindings;
            slot.set(
                Buttons::FIRE,
                held(&bd.get(Scheme::Controller, Action::Fire)),
            );
            slot.set(
                Buttons::BLOCK,
                held(&bd.get(Scheme::Controller, Action::Block)),
            );
            slot.set(
                Buttons::JUMP,
                held(&bd.get(Scheme::Controller, Action::Jump)),
            );
            slot.set(
                Buttons::EDIT,
                held(&bd.get(Scheme::Controller, Action::Edit)),
            );
            let (pressed, released) = slot.take_edges();
            out.push((
                DeviceId::Controller(id),
                InputState {
                    move_axis: stick(Axis::LeftX, Axis::LeftY),
                    aim_axis: stick(Axis::RightX, Axis::RightY),
                    pointer: None,
                    held: slot.held,
                    pressed,
                    released,
                    triggers: [lt, rt],
                    wheel: 0.0,
                },
            ));
        }
    }
}

/// Controller buttons that arrive as events (fire/block/jump are polled in
/// `snapshot` so taps on triggers and shoulders count too).
fn controller_button(b: Button) -> Vec<Buttons> {
    match b {
        // Jump/fire/block/edit are polled from the bindings in `snapshot`.
        Button::A => vec![Buttons::CONFIRM],
        Button::LeftShoulder => vec![Buttons::PREV],
        Button::RightShoulder => vec![Buttons::NEXT],
        Button::B => vec![Buttons::BACK],
        Button::Start => vec![Buttons::START],
        Button::Back => vec![Buttons::BACK],
        Button::Guide => vec![Buttons::START],
        Button::DPadUp => vec![Buttons::UP],
        Button::DPadDown => vec![Buttons::DOWN],
        Button::DPadLeft => vec![Buttons::LEFT],
        Button::DPadRight => vec![Buttons::RIGHT],
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deadzone_rescales() {
        assert_eq!(deadzone(Vec2::new(0.1, 0.1), 0.2), Vec2::ZERO);
        let v = deadzone(Vec2::new(0.6, 0.0), 0.2);
        assert!((v.x - 0.5).abs() < 1e-6);
        let v = deadzone(Vec2::new(1.0, 1.0), 0.2);
        assert!((v.length() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn short_taps_are_latched() {
        let mut s = DeviceSlot::new();
        s.set(Buttons::JUMP, true);
        s.set(Buttons::JUMP, false);
        let (p, r) = s.take_edges();
        assert!(p.contains(Buttons::JUMP) && r.contains(Buttons::JUMP));
        let (p, r) = s.take_edges();
        assert_eq!((p, r), (Buttons::default(), Buttons::default()));
        s.set(Buttons::FIRE, true);
        let (p, _) = s.take_edges();
        assert!(p.contains(Buttons::FIRE));
        let (p, _) = s.take_edges();
        assert!(!p.contains(Buttons::FIRE), "held is not pressed again");
    }
}
