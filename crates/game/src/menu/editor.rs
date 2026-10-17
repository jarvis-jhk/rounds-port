//! Face editor (character creator, ui.md 2.6): four categories (eyes,
//! mouths, two accessory slots), a grid of items 7 per row, a big preview
//! on which items can be dragged (mouse: press within 2 units of an item;
//! controller: right stick moves the active category's item at 3 units/s,
//! offsets clamped to a radius of 4.5). Edits a face preset in place; the
//! settings are saved when the editor closes.

use glam::Vec2;

use super::{Cx, Shared};
use crate::faces::{CATEGORIES, CATEGORY_KEYS};
use crate::input::DeviceId;
use crate::settings::Face;
use crate::ui::{Font, Rect, style};

const COLS: usize = 7;
const DRAG_RADIUS: f32 = 4.5;

pub struct FaceEditor {
    /// Device that edits (`None`: whoever, e.g. the mouse).
    pub owner: Option<DeviceId>,
    pub preset: usize,
    /// Skin colour index for the preview body.
    pub skin: usize,
    cat: usize,
    sel: usize,
    scroll: f32,
    drag: Option<usize>,
    /// Focus on the Save button (controller).
    on_save: bool,
}

/// Preview geometry: centre and body radius (reference pixels).
const PREVIEW_C: Vec2 = Vec2::new(-560.0, 520.0);
const PREVIEW_R: f32 = 250.0;

impl FaceEditor {
    pub fn new(owner: Option<DeviceId>, preset: usize, skin: usize) -> FaceEditor {
        FaceEditor {
            owner,
            preset,
            skin,
            cat: 0,
            sel: 0,
            scroll: 0.0,
            drag: None,
            on_save: false,
        }
    }

    fn face(&self, sh: &Shared) -> Face {
        sh.preset(self.preset)
    }

    fn set_face(&self, sh: &mut Shared, f: Face) {
        if let Some(x) = sh.settings.faces.get_mut(self.preset) {
            *x = f;
        }
    }

    /// Face units -> preview pixels (one face unit = 0.32 body radius).
    fn unit() -> f32 {
        PREVIEW_R * 0.32
    }

    /// Returns true when the editor closes.
    pub fn tick(&mut self, cx: &mut Cx<'_>) -> bool {
        let w = cx.width;
        cx.frame.rect(
            Rect::new(Vec2::new(0.0, 540.0), Vec2::new(w + 10.0, 1090.0)),
            0.0,
            style::PANEL,
        );
        let nav = match self.owner {
            Some(d) => cx.input.dev(d),
            None => cx.input.nav,
        };
        let state = self
            .owner
            .and_then(|d| cx.input.states.iter().find(|(id, _)| *id == d).map(|x| x.1));
        let mut face = self.face(cx.shared);

        // Category tabs.
        let tab_w = 300.0;
        let tabs: Vec<Rect> = (0..CATEGORIES)
            .map(|i| {
                Rect::new(
                    Vec2::new(200.0 + (i as f32 - 1.5) * tab_w, 110.0),
                    Vec2::new(tab_w - 10.0, 70.0),
                )
            })
            .collect();
        let mut cat = self.cat;
        if nav.prev {
            cat = (cat + CATEGORIES - 1) % CATEGORIES;
        }
        if nav.next {
            cat = (cat + 1) % CATEGORIES;
        }
        if cx.input.click
            && let Some(p) = cx.input.pointer
            && let Some(i) = tabs.iter().position(|r| r.contains(p))
        {
            cat = i;
        }
        if cat != self.cat {
            self.cat = cat;
            self.sel = face.item(cat).0 as usize;
            self.scroll = 0.0;
            cx.shared.sfx(cx.audio, "UI_Button_Hover");
        }
        // Grid.
        let n = cx.shared.catalog.list(self.cat).len();
        let cell = 150.0;
        let gx0 = 200.0 - cell * COLS as f32 * 0.5;
        let gy0 = 190.0;
        let rows_visible = 5usize;
        let row_of = |i: usize| i / COLS;
        let mut sel = self.sel.min(n.saturating_sub(1));
        if !self.on_save {
            if nav.left && !sel.is_multiple_of(COLS) {
                sel -= 1;
            }
            if nav.right && sel % COLS + 1 < COLS && sel + 1 < n {
                sel += 1;
            }
            if nav.up && sel >= COLS {
                sel -= COLS;
            }
            if nav.down {
                if sel + COLS < n {
                    sel += COLS;
                } else {
                    self.on_save = true;
                }
            }
        } else if nav.up {
            self.on_save = false;
        }
        if sel != self.sel {
            cx.shared.sfx(cx.audio, "UI_Button_Hover");
        }
        self.sel = sel;
        // Scroll to keep the selection visible; the wheel scrolls too.
        let total_rows = n.div_ceil(COLS);
        let max_scroll = total_rows.saturating_sub(rows_visible) as f32;
        self.scroll = (self.scroll - cx.input.wheel).clamp(0.0, max_scroll);
        if !cx.input.mouse_mode {
            let r = row_of(self.sel) as f32;
            if r < self.scroll {
                self.scroll = r;
            }
            if r >= self.scroll + rows_visible as f32 {
                self.scroll = r - rows_visible as f32 + 1.0;
            }
        }
        let first_row = self.scroll.floor() as usize;
        let mut cells: Vec<(usize, Rect)> = Vec::new();
        for i in first_row * COLS..((first_row + rows_visible) * COLS).min(n) {
            let r = row_of(i) - first_row;
            let c = i % COLS;
            cells.push((
                i,
                Rect::new(
                    Vec2::new(gx0 + cell * (c as f32 + 0.5), gy0 + cell * (r as f32 + 0.5)),
                    Vec2::splat(cell - 14.0),
                ),
            ));
        }
        let mut equip = None;
        if nav.confirm && !self.on_save {
            equip = Some(self.sel);
        }
        if cx.input.click
            && let Some(p) = cx.input.pointer
            && let Some((i, _)) = cells.iter().find(|(_, r)| r.contains(p))
        {
            equip = Some(*i);
            self.sel = *i;
        }
        if let Some(i) = equip {
            face.set_item(self.cat, i as u16);
            cx.shared.sfx(cx.audio, "UI_Button_Click");
        }
        // Dragging on the preview.
        let unit = Self::unit();
        let item_pos = |f: &Face, c: usize, sh: &Shared| -> Vec2 {
            let (id, off) = f.item(c);
            let def = sh
                .catalog
                .list(c)
                .get(id as usize)
                .map(|d| d.offset)
                .unwrap_or_default();
            def + Vec2::new(off[0], off[1])
        };
        if let Some(p) = cx.input.pointer {
            let local = (p - PREVIEW_C) / unit * Vec2::new(1.0, -1.0);
            if cx.input.click && self.drag.is_none() && (p - PREVIEW_C).length() < PREVIEW_R * 1.6 {
                self.drag = (0..CATEGORIES)
                    .filter(|c| *c < 2 || face.item(*c).0 > 0)
                    .map(|c| (c, (item_pos(&face, c, cx.shared) - local).length()))
                    .filter(|(_, d)| *d < 2.0)
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|x| x.0);
            }
            if let Some(c) = self.drag {
                if cx.input.mouse_held {
                    let def = item_pos(&face, c, cx.shared) - {
                        let o = face.item(c).1;
                        Vec2::new(o[0], o[1])
                    };
                    let off = (local - def).clamp_length_max(DRAG_RADIUS);
                    face.set_offset(c, [off.x, off.y]);
                } else {
                    self.drag = None;
                }
            }
        }
        if let Some(s) = state
            && s.aim_axis.length() > 0.2
        {
            let o = face.item(self.cat).1;
            let off = (Vec2::new(o[0], o[1]) + s.aim_axis * 3.0 * 0.014 * 4.0)
                .clamp_length_max(DRAG_RADIUS);
            face.set_offset(self.cat, [off.x, off.y]);
        }
        self.set_face(cx.shared, face);

        // Draw: preview, tabs, grid, save.
        let sh = &*cx.shared;
        cx.frame.text(
            format!("FACE {}", self.preset + 1),
            Vec2::new(PREVIEW_C.x, 150.0),
            70.0,
            Vec2::splat(0.5),
            Font::Light,
            style::TEXT,
        );
        let body = sh.colour(self.skin);
        cx.frame
            .face(sh.face_items(&face), PREVIEW_C, PREVIEW_R, body, 1.0);
        for (i, r) in tabs.iter().enumerate() {
            let (k, f) = CATEGORY_KEYS[i];
            let on = i == self.cat;
            if on {
                cx.frame.rect(*r, 10.0, style::BAR);
            }
            cx.frame.text(
                sh.up(k, f),
                r.c,
                34.0,
                Vec2::splat(0.5),
                if on { Font::Bold } else { Font::Light },
                if on { style::TEXT } else { style::GREY },
            );
        }
        let equipped = face.item(self.cat).0 as usize;
        for (i, r) in &cells {
            let focused = *i == self.sel && !self.on_save;
            cx.frame.rect(
                *r,
                16.0,
                if focused {
                    style::BAR
                } else {
                    style::PANEL_LIGHT
                },
            );
            // The item alone on a small body.
            let items = cell_items(sh, self.cat, *i);
            cx.frame.face(items, r.c, r.size.x * 0.36, body, 1.0);
            if *i == equipped {
                cx.frame.circle(
                    r.c + Vec2::new(r.size.x * 0.38, -r.size.y * 0.38),
                    9.0,
                    style::TEXT,
                );
            }
        }
        if total_rows > rows_visible {
            cx.frame.text(
                format!(
                    "{}/{}",
                    (self.scroll as usize + rows_visible).min(total_rows),
                    total_rows
                ),
                Vec2::new(
                    gx0 + cell * COLS as f32 + 20.0,
                    gy0 + cell * rows_visible as f32 - 20.0,
                ),
                26.0,
                Vec2::new(0.0, 0.5),
                Font::Regular,
                style::GREY,
            );
        }
        let save_r = Rect::new(Vec2::new(200.0, 1000.0), Vec2::new(420.0, 80.0));
        if self.on_save {
            cx.frame.rect(save_r, 10.0, style::BAR);
        }
        cx.frame.text(
            sh.up("BUTTON_SAVE", "Save"),
            save_r.c,
            54.0,
            Vec2::splat(0.5),
            if self.on_save {
                Font::Bold
            } else {
                Font::Light
            },
            style::TEXT,
        );
        cx.frame.text(
            if self.owner.is_some_and(|d| !d.is_keyboard()) {
                "LB / RB: CATEGORY   A: PICK   RIGHT STICK: MOVE   START: SAVE"
            } else {
                "Q / E: CATEGORY   CLICK: PICK   DRAG ON THE FACE: MOVE   ENTER: SAVE"
            },
            Vec2::new(0.0, 1055.0),
            22.0,
            Vec2::splat(0.5),
            Font::Regular,
            style::GREY,
        );
        let clicked_save = cx.input.click && cx.input.pointer.is_some_and(|p| save_r.contains(p));
        let done = clicked_save
            || nav.start
            || nav.back
            || (self.on_save && nav.confirm)
            || (self.owner.is_none_or(|d| d.is_keyboard())
                && cx.input.dev(DeviceId::KeyboardMouse).back);
        if done {
            cx.shared.settings.save();
            cx.shared.sfx(cx.audio, "UI_Button_Click");
        }
        done
    }
}

/// The face items of one grid cell: only the item `index` of `cat`.
fn cell_items(sh: &Shared, cat: usize, index: usize) -> Vec<crate::play::assets::FaceItem> {
    let mut f = Face::default();
    let slot = if cat >= 2 { 2 } else { cat };
    f.set_item(slot, index as u16);
    let all = sh.face_items(&f);
    match cat {
        0 => all.into_iter().take(1).collect(),
        1 => all.into_iter().skip(1).take(1).collect(),
        _ => all.into_iter().skip(2).collect(),
    }
}
