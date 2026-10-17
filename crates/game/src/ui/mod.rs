//! Menu UI toolkit in the original's style (ui.md 1-2): a 1920x1080
//! reference canvas scaled to the target height, big uppercase Gravity
//! texts, a highlight bar for keyboard/controller navigation that hides in
//! mouse mode, bold selected items, `Nope` flashes.
//!
//! Immediate mode: during `tick` a screen builds a [`Ui`] (layout,
//! interaction against the pointer and every device's navigation) which
//! records draw commands; `draw` replays them scaled to the target.
//! Coordinates are reference pixels, x from the screen centre (the canvas
//! is as wide as the aspect ratio makes it), y down from the top.

pub mod background;
pub mod router;
pub mod strings;

use glam::Vec2;
use rounds_render::{Color, DrawList, FontId, Layer, Text};

use crate::play::assets::{FaceItem, PlayAssets};
pub use router::{Router, UiInput};

/// Reference height.
pub const REF_H: f32 = 1080.0;

/// Palette (ui.md 1.2).
pub mod style {
    use rounds_render::Color;
    pub const TEXT: Color = Color::rgb(0.902, 0.902, 0.902);
    pub const GREY: Color = Color::rgb(0.553, 0.553, 0.553);
    pub const DIM: Color = Color::rgb(0.33, 0.33, 0.33);
    pub const PINK: Color = Color::rgba(1.0, 0.204, 0.477, 0.96);
    pub const DARK_TEXT: Color = Color::rgb(0.1415, 0.1415, 0.1415);
    pub const BAR: Color = Color::rgba(1.0, 1.0, 1.0, 0.09);
    pub const PANEL: Color = Color::rgba(0.0, 0.0, 0.0, 0.45);
    pub const PANEL_LIGHT: Color = Color::rgba(1.0, 1.0, 1.0, 0.05);
    pub const NOPE: Color = Color::rgb(0.85, 0.2, 0.2);
    pub const GOOD: Color = Color::rgb(0.465, 0.604, 0.390);
}

/// Which font family a text uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Font {
    Light,
    Regular,
    Bold,
    UltraLight,
    /// Embedded Noto Sans (glyphs the Gravity atlases lack).
    Fallback,
}

/// Maps ref coordinates to the target.
#[derive(Clone, Copy, Debug)]
pub struct Canvas {
    pub size: Vec2,
    pub k: f32,
}

impl Canvas {
    pub fn new(size: (u32, u32)) -> Canvas {
        let size = Vec2::new(size.0 as f32, size.1 as f32);
        Canvas {
            size,
            k: (size.y / REF_H).max(1e-3),
        }
    }

    /// Canvas width in reference pixels.
    pub fn width(&self) -> f32 {
        self.size.x / self.k
    }

    pub fn p(&self, v: Vec2) -> Vec2 {
        Vec2::new(self.size.x * 0.5 + v.x * self.k, v.y * self.k)
    }

    pub fn to_ref(self, screen: Vec2) -> Vec2 {
        Vec2::new((screen.x - self.size.x * 0.5) / self.k, screen.y / self.k)
    }
}

/// Axis-aligned rectangle in reference pixels (centre + size).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub c: Vec2,
    pub size: Vec2,
}

impl Rect {
    pub fn new(c: Vec2, size: Vec2) -> Rect {
        Rect { c, size }
    }

    pub fn contains(&self, p: Vec2) -> bool {
        let d = (p - self.c).abs();
        d.x <= self.size.x * 0.5 && d.y <= self.size.y * 0.5
    }

    pub fn left(&self) -> f32 {
        self.c.x - self.size.x * 0.5
    }

    pub fn top(&self) -> f32 {
        self.c.y - self.size.y * 0.5
    }

    pub fn right(&self) -> f32 {
        self.c.x + self.size.x * 0.5
    }

    pub fn bottom(&self) -> f32 {
        self.c.y + self.size.y * 0.5
    }
}

/// One recorded draw command (reference coordinates).
#[derive(Clone, Debug)]
pub enum Cmd {
    Rect {
        r: Rect,
        radius: f32,
        color: Color,
    },
    Circle {
        c: Vec2,
        radius: f32,
        color: Color,
    },
    Ring {
        c: Vec2,
        radius: f32,
        thickness: f32,
        color: Color,
    },
    Text {
        text: String,
        pos: Vec2,
        size: f32,
        anchor: Vec2,
        font: Font,
        color: Color,
    },
    /// An imported sprite filling `size` (reference px) around `c`.
    Sprite {
        sprite: crate::play::assets::SpriteRef,
        c: Vec2,
        size: Vec2,
        color: Color,
    },
    /// A player body with a face.
    Face {
        items: Vec<FaceItem>,
        c: Vec2,
        radius: f32,
        body: Color,
        facing: f32,
    },
}

/// A screen's recorded UI.
#[derive(Clone, Debug, Default)]
pub struct Frame {
    pub cmds: Vec<Cmd>,
}

impl Frame {
    pub fn clear(&mut self) {
        self.cmds.clear();
    }

    pub fn rect(&mut self, r: Rect, radius: f32, color: Color) {
        self.cmds.push(Cmd::Rect { r, radius, color });
    }

    pub fn circle(&mut self, c: Vec2, radius: f32, color: Color) {
        self.cmds.push(Cmd::Circle { c, radius, color });
    }

    pub fn ring(&mut self, c: Vec2, radius: f32, thickness: f32, color: Color) {
        self.cmds.push(Cmd::Ring {
            c,
            radius,
            thickness,
            color,
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub fn text(
        &mut self,
        text: impl Into<String>,
        pos: Vec2,
        size: f32,
        anchor: Vec2,
        font: Font,
        color: Color,
    ) {
        self.cmds.push(Cmd::Text {
            text: text.into(),
            pos,
            size,
            anchor,
            font,
            color,
        });
    }

    pub fn sprite(
        &mut self,
        sprite: crate::play::assets::SpriteRef,
        c: Vec2,
        size: Vec2,
        color: Color,
    ) {
        self.cmds.push(Cmd::Sprite {
            sprite,
            c,
            size,
            color,
        });
    }

    pub fn face(&mut self, items: Vec<FaceItem>, c: Vec2, radius: f32, body: Color, facing: f32) {
        self.cmds.push(Cmd::Face {
            items,
            c,
            radius,
            body,
            facing,
        });
    }

    /// Replays the commands on the screen layer from `sort` up.
    pub fn draw(
        &self,
        list: &mut DrawList,
        canvas: &Canvas,
        assets: &PlayAssets,
        fallback_fonts: bool,
        sort: i32,
    ) {
        let k = canvas.k;
        let fonts = assets.fonts;
        for (i, c) in self.cmds.iter().enumerate() {
            let z = sort + i as i32 * 2;
            match c {
                Cmd::Rect { r, radius, color } => {
                    list.rounded_rect(
                        Layer::Screen,
                        z,
                        canvas.p(r.c),
                        r.size * k,
                        radius * k,
                        *color,
                    );
                }
                Cmd::Circle { c, radius, color } => {
                    list.circle(Layer::Screen, z, canvas.p(*c), radius * k, *color)
                }
                Cmd::Ring {
                    c,
                    radius,
                    thickness,
                    color,
                } => list.ring(
                    Layer::Screen,
                    z,
                    canvas.p(*c),
                    radius * k,
                    thickness * k,
                    0.0,
                    std::f32::consts::TAU,
                    *color,
                ),
                Cmd::Text {
                    text,
                    pos,
                    size,
                    anchor,
                    font,
                    color,
                } => {
                    let f = if fallback_fonts || !gravity_covers(text) {
                        FontId::DEFAULT
                    } else {
                        match font {
                            Font::Light => fonts.light,
                            Font::Regular => fonts.regular,
                            Font::Bold => fonts.bold,
                            Font::UltraLight => fonts.ultralight,
                            Font::Fallback => FontId::DEFAULT,
                        }
                    };
                    list.push(
                        Layer::Screen,
                        z,
                        Text {
                            text: text.clone(),
                            font: f,
                            size: size * k,
                            position: canvas.p(*pos),
                            rotation: 0.0,
                            anchor: *anchor,
                            color: *color,
                        },
                    );
                }
                Cmd::Sprite {
                    sprite,
                    c,
                    size,
                    color,
                } => {
                    let s = sprite.fill(
                        glam::Affine2::from_translation(canvas.p(*c)),
                        *size * k,
                        *color,
                    );
                    list.sprite(Layer::Screen, z, s);
                }
                Cmd::Face {
                    items,
                    c,
                    radius,
                    body,
                    facing,
                } => crate::faces::draw_face_screen(
                    list,
                    z,
                    assets,
                    items,
                    canvas.p(*c),
                    radius * k,
                    *body,
                    *facing,
                ),
            }
        }
    }
}

/// True if the Gravity atlases (Latin) can show `text`.
fn gravity_covers(text: &str) -> bool {
    text.chars()
        .all(|c| (c as u32) < 0x250 || c == '…' || c == '•')
}

/// A vertical list of menu items with keyboard/controller focus and mouse
/// hover (ui.md 2.2). Returns what happened this tick.
#[derive(Clone, Debug, Default)]
pub struct ListNav {
    pub focus: usize,
    /// Nope flash: (item, ticks left).
    pub nope: Option<(usize, u32)>,
    /// Tick counter for animations.
    pub t: u32,
}

/// Result of [`ListNav::update`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ListEvent {
    /// Item activated (confirm / click).
    pub activate: Option<usize>,
    /// Value change on an item: -1 / +1 (left/right, or click on arrows).
    pub change: Option<(usize, i32)>,
    pub back: bool,
    /// Focus moved (play the hover sound).
    pub moved: bool,
}

impl ListNav {
    /// `rects`: hit areas of the items; `enabled[i]` false = greyed (Nope).
    pub fn update(&mut self, input: &UiInput, rects: &[Rect], enabled: &[bool]) -> ListEvent {
        self.t = self.t.wrapping_add(1);
        if let Some((i, n)) = self.nope {
            self.nope = if n > 1 { Some((i, n - 1)) } else { None };
        }
        let mut ev = ListEvent::default();
        let n = rects.len();
        if n == 0 {
            ev.back = input.nav.back;
            return ev;
        }
        self.focus = self.focus.min(n - 1);
        let nav = &input.nav;
        let before = self.focus;
        if nav.up {
            self.focus = (self.focus + n - 1) % n;
        }
        if nav.down {
            self.focus = (self.focus + 1) % n;
        }
        if input.mouse_mode
            && let Some(p) = input.pointer
            && let Some(i) = rects.iter().position(|r| r.contains(p))
        {
            self.focus = i;
        }
        ev.moved = self.focus != before;
        let ok = |i: usize| enabled.get(i).copied().unwrap_or(true);
        if nav.left {
            ev.change = Some((self.focus, -1));
        }
        if nav.right {
            ev.change = Some((self.focus, 1));
        }
        if nav.confirm {
            ev.activate = Some(self.focus);
        }
        if input.click
            && let Some(p) = input.pointer
            && let Some(i) = rects.iter().position(|r| r.contains(p))
        {
            ev.activate = Some(i);
        }
        if let Some(i) = ev.activate
            && !ok(i)
        {
            self.nope = Some((i, 11));
            ev.activate = None;
        }
        ev.back = nav.back;
        ev
    }

    /// Text colour and font of item `i` (bold when focused, red flash).
    pub fn style(&self, i: usize, enabled: bool) -> (Color, Font) {
        let focused = i == self.focus;
        let mut c = if enabled { style::TEXT } else { style::GREY };
        if let Some((n, _)) = self.nope
            && n == i
        {
            c = style::NOPE;
        }
        (c, if focused { Font::Bold } else { Font::Light })
    }

    /// Horizontal shake of the Nope animation (15 units, 0.2 s).
    pub fn shake(&self, i: usize) -> f32 {
        match self.nope {
            Some((n, left)) if n == i => (left as f32 * 2.1).sin() * 15.0 * (left as f32 / 14.0),
            _ => 0.0,
        }
    }
}

/// Draws the highlight bar behind a focused row (controller mode only).
pub fn bar(frame: &mut Frame, input: &UiInput, r: Rect) {
    if !input.mouse_mode {
        frame.rect(r, 6.0, style::BAR);
    }
}

/// A one-line text field.
#[derive(Clone, Debug, Default)]
pub struct TextField {
    pub value: String,
    pub max: usize,
}

impl TextField {
    pub fn new(max: usize) -> TextField {
        TextField {
            value: String::new(),
            max,
        }
    }

    /// Applies typed text and backspace. `filter` keeps allowed chars.
    pub fn edit(&mut self, input: &UiInput, filter: impl Fn(char) -> bool) -> bool {
        let before = self.value.clone();
        for ch in input.text.chars() {
            if filter(ch) && self.value.chars().count() < self.max {
                self.value.push(ch);
            }
        }
        if input.backspace {
            self.value.pop();
        }
        before != self.value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canvas_roundtrip() {
        let c = Canvas::new((2560, 1080));
        assert!((c.width() - 2560.0).abs() < 1e-3);
        let p = Vec2::new(-300.0, 200.0);
        assert!((c.to_ref(c.p(p)) - p).length() < 1e-3);
        let c = Canvas::new((1280, 720));
        assert!((c.p(Vec2::new(0.0, 1080.0)) - Vec2::new(640.0, 720.0)).length() < 1e-3);
    }

    #[test]
    fn list_nav_wraps_and_nopes() {
        let rects: Vec<Rect> = (0..3)
            .map(|i| Rect::new(Vec2::new(0.0, i as f32 * 100.0), Vec2::new(200.0, 90.0)))
            .collect();
        let mut nav = ListNav::default();
        let mut input = UiInput::default();
        input.nav.up = true;
        let ev = nav.update(&input, &rects, &[true, true, false]);
        assert_eq!(nav.focus, 2);
        assert!(ev.moved);
        input.nav.up = false;
        input.nav.confirm = true;
        let ev = nav.update(&input, &rects, &[true, true, false]);
        assert_eq!(ev.activate, None, "disabled item");
        assert!(nav.nope.is_some());
        // Mouse hover + click.
        let input = UiInput {
            mouse_mode: true,
            click: true,
            pointer: Some(Vec2::new(10.0, 100.0)),
            ..Default::default()
        };
        let ev = nav.update(&input, &rects, &[true; 3]);
        assert_eq!(ev.activate, Some(1));
    }
}

/// Controller buttons shown in prompts (the imported button atlases'
/// order: south, east, west, north, start/menu, left/right trigger).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)] // the atlas order; not every button is used in prompts yet
pub enum Glyph {
    South,
    East,
    West,
    North,
    Start,
    LeftTrigger,
    RightTrigger,
}

impl Glyph {
    fn index(self) -> usize {
        self as usize
    }

    /// Xbox letter and colour for the drawn fallback.
    fn fallback(self) -> (&'static str, Color) {
        match self {
            Glyph::South => ("A", Color::rgb(0.38, 0.71, 0.27)),
            Glyph::East => ("B", Color::rgb(0.86, 0.26, 0.25)),
            Glyph::West => ("X", Color::rgb(0.24, 0.47, 0.86)),
            Glyph::North => ("Y", Color::rgb(0.93, 0.72, 0.18)),
            Glyph::Start => ("≡", Color::rgb(0.6, 0.6, 0.6)),
            Glyph::LeftTrigger => ("LT", Color::rgb(0.6, 0.6, 0.6)),
            Glyph::RightTrigger => ("RT", Color::rgb(0.6, 0.6, 0.6)),
        }
    }
}

/// Button glyph sprites from the import (`XboxSeriesButtonAtlas`,
/// `Playstation_4_Buttons_Atlas`), if present.
#[derive(Clone, Debug, Default)]
pub struct Glyphs {
    pub xbox: Vec<Option<crate::play::assets::SpriteRef>>,
    pub playstation: Vec<Option<crate::play::assets::SpriteRef>>,
}

impl Glyphs {
    pub fn load(
        gpu: &rounds_render::Gpu,
        renderer: &mut rounds_render::Renderer,
        assets: &mut crate::play::assets::PlayAssets,
    ) -> Glyphs {
        let mut set = |prefix: &str| -> Vec<Option<crate::play::assets::SpriteRef>> {
            (0..7)
                .map(|i| assets.sprite_named(gpu, renderer, &format!("{prefix}_128x128_{i}")))
                .collect()
        };
        Glyphs {
            xbox: set("XboxSeriesButtonAtlas"),
            playstation: set("Playstation_4_Buttons_Atlas"),
        }
    }
}

/// PlayStation-style controller (by its SDL name).
pub fn is_playstation(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    [
        "ps3",
        "ps4",
        "ps5",
        "dualshock",
        "dualsense",
        "playstation",
        "sony",
    ]
    .iter()
    .any(|k| n.contains(k))
}

/// A piece of a prompt line.
#[derive(Clone, Debug)]
pub enum Part {
    Text(String),
    Glyph(Glyph),
}

impl Frame {
    /// A controller button glyph (the imported sprite, or a drawn circle
    /// with the Xbox letter) centred at `c`, `size` reference px tall.
    pub fn glyph(
        &mut self,
        glyphs: &Glyphs,
        playstation: bool,
        g: Glyph,
        c: Vec2,
        size: f32,
        alpha: f32,
    ) {
        let set = if playstation && glyphs.playstation.iter().any(|s| s.is_some()) {
            &glyphs.playstation
        } else {
            &glyphs.xbox
        };
        if let Some(Some(s)) = set.get(g.index()) {
            self.sprite(*s, c, Vec2::splat(size), Color::rgba(1.0, 1.0, 1.0, alpha));
            return;
        }
        let (letter, col) = g.fallback();
        self.circle(c, size * 0.45, col.with_alpha(alpha));
        self.text(
            letter,
            c,
            size * 0.55,
            Vec2::splat(0.5),
            Font::Bold,
            Color::rgba(1.0, 1.0, 1.0, alpha),
        );
    }

    /// A centred prompt line mixing text and glyphs (text widths are
    /// estimated from the size; the Gravity fonts average ~0.52 em).
    #[allow(clippy::too_many_arguments)]
    pub fn prompt(
        &mut self,
        glyphs: &Glyphs,
        playstation: bool,
        parts: &[Part],
        c: Vec2,
        size: f32,
        font: Font,
        color: Color,
    ) {
        let em = |c: char| match c {
            ' ' => 0.27,
            '·' | '.' | ',' | ':' | '/' | '|' | '\'' | '-' | 'I' | 'i' | 'l' => 0.3,
            'M' | 'W' | 'm' | 'w' => 0.85,
            c if c.is_uppercase() || c.is_ascii_digit() => 0.64,
            _ => 0.5,
        };
        let w = |p: &Part| match p {
            Part::Text(t) => t.chars().map(em).sum::<f32>() * size,
            Part::Glyph(_) => size * 1.35,
        };
        let total: f32 =
            parts.iter().map(w).sum::<f32>() + size * 0.25 * parts.len().saturating_sub(1) as f32;
        let mut x = c.x - total * 0.5;
        for p in parts {
            let pw = w(p);
            match p {
                Part::Text(t) => self.text(
                    t.clone(),
                    Vec2::new(x + pw * 0.5, c.y),
                    size,
                    Vec2::splat(0.5),
                    font,
                    color,
                ),
                Part::Glyph(g) => self.glyph(
                    glyphs,
                    playstation,
                    *g,
                    Vec2::new(x + pw * 0.5, c.y),
                    size * 1.25,
                    color.a,
                ),
            }
            x += pw + size * 0.25;
        }
    }
}
