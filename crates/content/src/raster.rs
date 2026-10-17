//! Debug rasteriser of the sim's collision state (tiny-skia): static
//! colliders, props, debris, animated groups and kill boxes, ropes, spawn
//! points, players, bullets and spawned objects, plus the arena bounds.
//!
//! Colours: static geometry grey (triggers outlined), damage boxes red,
//! props brown (destructibles orange, moving platforms blue, layer-18
//! background props dark), animated groups purple (armed kill box bright
//! red), ropes yellow, spawn points green crosses, players by slot colour
//! (hollow = dead), bullets white with a velocity tick, spawned objects cyan.

use rounds_sim::World;
use rounds_sim::content::{Content, MapId};
use rounds_sim::geom::{Shape, layers};
use rounds_sim::math::{V2, v2};
use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Rect, Stroke, Transform};

/// World window of a tile: centred on the origin, 16:9, slightly larger
/// than the arena (+-35.56 x +-20).
pub const VIEW_HALF: V2 = V2 { x: 40.0, y: 22.5 };

const SLOT_COLOURS: [[u8; 3]; 12] = [
    [255, 120, 40],
    [60, 160, 255],
    [250, 220, 60],
    [120, 230, 90],
    [235, 80, 200],
    [80, 230, 220],
    [255, 255, 255],
    [180, 120, 255],
    [255, 80, 80],
    [150, 200, 120],
    [255, 170, 170],
    [160, 160, 255],
];

pub struct Canvas {
    pub pixmap: Pixmap,
    /// World -> pixel.
    tf: Transform,
    scale: f32,
}

fn paint(rgb: [u8; 3], a: u8) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(Color::from_rgba8(rgb[0], rgb[1], rgb[2], a));
    p.anti_alias = true;
    p
}

impl Canvas {
    pub fn new(w: u32, h: u32) -> Canvas {
        let mut pixmap = Pixmap::new(w, h).expect("pixmap size");
        pixmap.fill(Color::from_rgba8(14, 14, 17, 255));
        let sx = w as f32 / (2.0 * VIEW_HALF.x);
        let sy = h as f32 / (2.0 * VIEW_HALF.y);
        let s = sx.min(sy);
        let tf = Transform::from_row(s, 0.0, 0.0, -s, w as f32 * 0.5, h as f32 * 0.5);
        Canvas {
            pixmap,
            tf,
            scale: s,
        }
    }

    fn path_of(shape: &Shape) -> Option<tiny_skia::Path> {
        let mut pb = PathBuilder::new();
        match shape {
            Shape::Circle { c, r } => pb.push_circle(c.x, c.y, r.max(0.01)),
            Shape::Obb { c, axis, half } => {
                let ax = *axis * half.x;
                let ay = axis.perp_left() * half.y;
                let pts = [*c - ax - ay, *c + ax - ay, *c + ax + ay, *c - ax + ay];
                pb.move_to(pts[0].x, pts[0].y);
                for p in &pts[1..] {
                    pb.line_to(p.x, p.y);
                }
                pb.close();
            }
            Shape::Poly { pts } => {
                let first = pts.first()?;
                pb.move_to(first.x, first.y);
                for p in &pts[1..] {
                    pb.line_to(p.x, p.y);
                }
                pb.close();
            }
        }
        pb.finish()
    }

    pub fn shape(&mut self, shape: &Shape, fill: Option<([u8; 3], u8)>, stroke: Option<[u8; 3]>) {
        let Some(path) = Self::path_of(shape) else {
            return;
        };
        if let Some((rgb, a)) = fill {
            self.pixmap
                .fill_path(&path, &paint(rgb, a), FillRule::Winding, self.tf, None);
        }
        if let Some(rgb) = stroke {
            let st = Stroke {
                width: 1.2 / self.scale,
                ..Stroke::default()
            };
            self.pixmap
                .stroke_path(&path, &paint(rgb, 255), &st, self.tf, None);
        }
    }

    pub fn line(&mut self, a: V2, b: V2, rgb: [u8; 3], width_px: f32) {
        let mut pb = PathBuilder::new();
        pb.move_to(a.x, a.y);
        pb.line_to(b.x, b.y);
        if let Some(path) = pb.finish() {
            let st = Stroke {
                width: width_px / self.scale,
                ..Stroke::default()
            };
            self.pixmap
                .stroke_path(&path, &paint(rgb, 255), &st, self.tf, None);
        }
    }

    pub fn rect_outline(&mut self, min: V2, max: V2, rgb: [u8; 3]) {
        if let Some(r) = Rect::from_ltrb(min.x, min.y, max.x, max.y) {
            let path = PathBuilder::from_rect(r);
            let st = Stroke {
                width: 1.0 / self.scale,
                dash: tiny_skia::StrokeDash::new(vec![6.0 / self.scale, 4.0 / self.scale], 0.0),
                ..Stroke::default()
            };
            self.pixmap
                .stroke_path(&path, &paint(rgb, 255), &st, self.tf, None);
        }
    }

    /// Tiny 3x5 pixel digits/letters (top-left at pixel `x, y`, `px` pixels per dot).
    pub fn text(&mut self, x: f32, y: f32, px: f32, s: &str, rgb: [u8; 3]) {
        let mut cx = x;
        for ch in s.chars() {
            let g = glyph(ch);
            for (row, bits) in g.iter().enumerate() {
                for col in 0..3 {
                    if bits & (4 >> col) != 0
                        && let Some(r) =
                            Rect::from_xywh(cx + col as f32 * px, y + row as f32 * px, px, px)
                    {
                        self.pixmap
                            .fill_rect(r, &paint(rgb, 255), Transform::identity(), None);
                    }
                }
            }
            cx += 4.0 * px;
        }
    }
}

fn glyph(c: char) -> [u8; 5] {
    match c.to_ascii_uppercase() {
        '0' => [7, 5, 5, 5, 7],
        '1' => [2, 6, 2, 2, 7],
        '2' => [7, 1, 7, 4, 7],
        '3' => [7, 1, 7, 1, 7],
        '4' => [5, 5, 7, 1, 1],
        '5' => [7, 4, 7, 1, 7],
        '6' => [7, 4, 7, 5, 7],
        '7' => [7, 1, 1, 1, 1],
        '8' => [7, 5, 7, 5, 7],
        '9' => [7, 5, 7, 1, 7],
        'A' => [2, 5, 7, 5, 5],
        'B' => [6, 5, 6, 5, 6],
        'C' => [7, 4, 4, 4, 7],
        'D' => [6, 5, 5, 5, 6],
        'E' => [7, 4, 6, 4, 7],
        'F' => [7, 4, 6, 4, 4],
        'G' => [7, 4, 5, 5, 7],
        'H' => [5, 5, 7, 5, 5],
        'I' => [7, 2, 2, 2, 7],
        'J' => [1, 1, 1, 5, 7],
        'K' => [5, 5, 6, 5, 5],
        'L' => [4, 4, 4, 4, 7],
        'M' => [5, 7, 7, 5, 5],
        'N' => [6, 5, 5, 5, 5],
        'O' => [7, 5, 5, 5, 7],
        'P' => [7, 5, 7, 4, 4],
        'Q' => [7, 5, 5, 7, 1],
        'R' => [7, 5, 6, 5, 5],
        'S' => [7, 4, 7, 1, 7],
        'T' => [7, 2, 2, 2, 2],
        'U' => [5, 5, 5, 5, 7],
        'V' => [5, 5, 5, 5, 2],
        'W' => [5, 5, 7, 7, 5],
        'X' => [5, 5, 2, 5, 5],
        'Y' => [5, 5, 2, 2, 2],
        'Z' => [7, 1, 2, 4, 7],
        '_' => [0, 0, 0, 0, 7],
        '-' => [0, 0, 7, 0, 0],
        ':' => [0, 2, 0, 2, 0],
        '.' => [0, 0, 0, 0, 2],
        _ => [0, 0, 0, 0, 0],
    }
}

/// Animated group shapes at the current scale (same formula as the sim).
fn anim_shapes(content: &Content, w: &World, id: MapId) -> Vec<(Shape, bool)> {
    let def = content.map(id);
    let mut out = vec![];
    for (k, a) in def.animated.iter().enumerate() {
        let (sy, armed) = w
            .map_anims()
            .get(k)
            .map(|s| (s.scale, s.armed > 0))
            .unwrap_or((1.0, false));
        let (c, s) = (
            rounds_sim::math::cos(a.angle),
            rounds_sim::math::sin(a.angle),
        );
        for col in &a.colliders {
            if let Shape::Obb { c: lc, axis, half } = &col.shape {
                out.push((
                    Shape::Obb {
                        c: a.pivot + v2(lc.x, lc.y * sy).rotate(c, s),
                        axis: axis.rotate(c, s),
                        half: v2(half.x, half.y * sy),
                    },
                    false,
                ));
            }
        }
        if let Some(kb) = a.kill_box {
            out.push((
                Shape::Obb {
                    c: a.pivot + v2(kb.center.x, kb.center.y * sy).rotate(c, s),
                    axis: v2(c, s),
                    half: v2(kb.half.x, kb.half.y * sy),
                },
                armed,
            ));
        }
    }
    out
}

/// Draws the world's current map state. Before the map's props exist in
/// the physics world they are drawn at their authored pose.
pub fn draw(canvas: &mut Canvas, content: &Content, w: &World) {
    let half = content.tuning.bounds_half;
    canvas.rect_outline(-half, half, [90, 90, 110]);
    let Some(id) = w.map_view().current else {
        return;
    };
    let def = content.map(id);
    for s in &content.prepared(id).statics {
        let (fill, stroke) = if s.damage_box.is_some() {
            (Some(([200, 50, 50], 230)), Some([255, 120, 120]))
        } else if s.trigger {
            (None, Some([200, 200, 90]))
        } else if s.layer == layers::IGNORE_MAP {
            (Some(([140, 70, 70], 220)), None)
        } else {
            (Some(([105, 105, 112], 255)), None)
        };
        canvas.shape(&s.shape, fill, stroke);
    }
    for (shape, armed) in anim_shapes(content, w, id) {
        if armed {
            canvas.shape(&shape, Some(([255, 30, 30], 255)), None);
        } else {
            canvas.shape(&shape, Some(([150, 90, 200], 230)), Some([210, 170, 255]));
        }
    }
    let prop_colour = |pi: usize| -> ([u8; 3], u8) {
        let p = &def.props[pi];
        if p.damage_box.is_some() {
            ([210, 40, 40], 240)
        } else if p.destructible.is_some() {
            ([215, 140, 60], 235)
        } else if p.move_sequence.is_some() {
            ([70, 120, 220], 235)
        } else if p
            .colliders
            .first()
            .is_some_and(|c| c.layer == layers::BACKGROUND_OBJECT)
        {
            ([80, 66, 54], 200)
        } else {
            ([150, 112, 80], 235)
        }
    };
    if w.props().is_empty() {
        for (pi, p) in def.props.iter().enumerate() {
            let (c, s) = (
                rounds_sim::math::cos(p.angle),
                rounds_sim::math::sin(p.angle),
            );
            for col in &p.colliders {
                canvas.shape(
                    &col.shape.transformed(p.pos, c, s),
                    Some(prop_colour(pi)),
                    Some([40, 30, 20]),
                );
            }
        }
    } else {
        for p in w.props() {
            if !p.alive {
                continue;
            }
            for s in &p.shapes {
                canvas.shape(s, Some(prop_colour(p.def as usize)), Some([40, 30, 20]));
            }
        }
    }
    for pc in w.debris() {
        let s = pc.scale();
        let shape = Shape::Obb {
            c: pc.pos,
            axis: V2::from_angle(pc.angle),
            half: pc.half * s,
        };
        canvas.shape(&shape, Some(([215, 140, 60], 160)), None);
    }
    for r in &def.ropes {
        canvas.line(r.start, r.end, [230, 210, 80], 1.0);
    }
    for sp in &def.spawn_points {
        let d = 0.8;
        canvas.line(sp.pos - v2(d, d), sp.pos + v2(d, d), [80, 230, 80], 1.5);
        canvas.line(sp.pos - v2(d, -d), sp.pos + v2(d, -d), [80, 230, 80], 1.5);
    }
    for o in w.objects() {
        canvas.shape(
            &Shape::Circle {
                c: o.pos,
                r: 0.6 * o.scale.max(0.2),
            },
            Some(([60, 220, 230], 90)),
            Some([60, 220, 230]),
        );
    }
    for b in w.projectiles() {
        canvas.shape(
            &Shape::Circle {
                c: b.pos,
                r: b.size.clamp(0.15, 2.0),
            },
            Some(([255, 255, 255], 255)),
            None,
        );
        canvas.line(b.pos, b.pos - b.vel * 0.03, [255, 255, 200], 1.0);
    }
    for (i, p) in w.players().iter().enumerate() {
        let col = SLOT_COLOURS[i % SLOT_COLOURS.len()];
        let shape = Shape::Circle {
            c: p.pos,
            r: p.radius(),
        };
        if p.dead {
            canvas.shape(&shape, None, Some(col));
        } else {
            canvas.shape(&shape, Some((col, 255)), Some([0, 0, 0]));
            canvas.line(
                p.pos,
                p.pos + p.aim.normalized() * (p.radius() + 0.6),
                [255, 255, 255],
                1.5,
            );
        }
    }
}

/// One tile: the world plus a label in the top-left corner.
pub fn render(content: &Content, w: &World, width: u32, height: u32, label: &str) -> Pixmap {
    let mut c = Canvas::new(width, height);
    draw(&mut c, content, w);
    let px = (width as f32 / 240.0).max(1.0).round();
    c.text(4.0, 4.0, px, label, [230, 230, 230]);
    c.pixmap
}

/// Contact sheet of tiles (row-major).
pub fn sheet(tiles: &[Pixmap], cols: usize) -> Option<Pixmap> {
    let first = tiles.first()?;
    let (tw, th) = (first.width(), first.height());
    let rows = tiles.len().div_ceil(cols);
    let gap = 4;
    let mut out = Pixmap::new(
        cols as u32 * (tw + gap) + gap,
        rows as u32 * (th + gap) + gap,
    )?;
    out.fill(Color::from_rgba8(40, 40, 46, 255));
    for (k, t) in tiles.iter().enumerate() {
        let x = (k % cols) as i32 * (tw + gap) as i32 + gap as i32;
        let y = (k / cols) as i32 * (th + gap) as i32 + gap as i32;
        out.draw_pixmap(
            x,
            y,
            t.as_ref(),
            &tiny_skia::PixmapPaint::default(),
            Transform::identity(),
            None,
        );
    }
    Some(out)
}
