//! Map presentation: turns an imported map into renderer draw items.
//!
//! What is drawn follows the original (see `docs/spec/maps.md` 7.9, 10.5):
//!
//! - Pieces with a `SpriteMasking.SpriteMask` (all static ground and most
//!   prop roots) draw **no colour**: they are stencil writers
//!   ([`Mask::Write`]); the foreground art particles show inside them.
//! - Other active, enabled SpriteRenderers with a material draw normally
//!   (prop "Color"/"Lines" overlays, background boxes, decorations), in
//!   Unity's order: sorting layer, order in layer, then farther z first.
//! - `SFPolygon`s become shadow casters; ropes (`LineRenderer`) are lines.
//!
//! Positions come from a [`TransformProvider`]: by default the imported
//! initial poses; the simulation will supply the current pose of each
//! rigid body, and everything parented under a body follows it.

use std::collections::HashMap;

use glam::{Affine2, Mat2, Vec2};
use rounds_import::GameData;
use rounds_import::format::{Map, Object};
use rounds_render::{Color, Mask, Sprite, TextureId, UvRect};

/// A loaded texture and its size in pixels.
pub type LoadedTexture = (TextureId, (u32, u32));

/// Supplies the current poses of a map's moving objects.
pub trait TransformProvider {
    /// World transform of the rigid body on object `object` (index into
    /// [`Map::objects`]), or `None` for its imported initial pose.
    fn body_transform(&self, object: usize) -> Option<Affine2>;

    /// Current world end points of the rope on object `object`, or `None`
    /// for the imported positions (the rope object and its first child).
    fn rope_ends(&self, _object: usize) -> Option<(Vec2, Vec2)> {
        None
    }

    /// Whether object `object` is currently shown (e.g. a destroyed box is
    /// not). Default: as imported (active in hierarchy).
    fn visible(&self, _object: usize) -> Option<bool> {
        None
    }

    /// Extra world offset of everything under the top-level child
    /// `child` of the map root (the map slide, match-flow.md 7.1).
    fn slide(&self, _child: usize) -> Vec2 {
        Vec2::ZERO
    }
}

/// The imported initial poses (no simulation).
pub struct InitialPose;

impl TransformProvider for InitialPose {
    fn body_transform(&self, _object: usize) -> Option<Affine2> {
        None
    }
}

/// Sort key of a world item, ascending = drawn first (Unity's order for
/// transparent 2D renderers).
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct SortKey {
    pub layer: usize,
    pub order: i32,
    /// Negated z: farther from the camera (larger z) first.
    pub neg_z: f32,
    /// 0 = stencil writers before everything else at the same depth.
    pub rank: u8,
    pub seq: u32,
}

impl SortKey {
    pub fn new(layer: usize, order: i32, z: f32, rank: u8, seq: u32) -> Self {
        SortKey {
            layer,
            order,
            neg_z: -z,
            rank,
            seq,
        }
    }

    pub fn cmp(&self, o: &SortKey) -> std::cmp::Ordering {
        self.partial_cmp(o).unwrap_or(std::cmp::Ordering::Equal)
    }
}

/// A sub-quad of a sprite in the object's local space.
#[derive(Clone, Copy, Debug)]
struct Quad {
    min: Vec2,
    size: Vec2,
    uv: UvRect,
}

#[derive(Clone, Debug)]
struct Item {
    object: usize,
    texture: TextureId,
    quads: Vec<Quad>,
    tint: Color,
    flip: Vec2,
    mask: Mask,
    key: SortKey,
}

#[derive(Clone, Debug)]
struct Rope {
    object: usize,
    a: Vec2,
    b: Vec2,
    width: f32,
    color: Color,
    key: SortKey,
}

/// Per object: initial world transform and the rigid body it moves with.
#[derive(Clone, Copy, Debug)]
struct Anchor {
    initial: Affine2,
    /// (body object, transform relative to the body's initial pose).
    body: Option<(usize, Affine2)>,
}

/// A map ready to draw.
pub struct MapView {
    pub name: String,
    #[allow(dead_code)]
    pub build_index: usize,
    /// Camera orthographic size wanted by the map (`Map.size`).
    pub size: f32,
    anchors: Vec<Anchor>,
    items: Vec<Item>,
    ropes: Vec<Rope>,
    /// (object, local polygon).
    casters: Vec<(usize, Vec<Vec2>)>,
    active: Vec<bool>,
    parent: Vec<Option<usize>>,
    /// Per object: index of its top-level child of the map root (itself
    /// for root children), `usize::MAX` for roots.
    top: Vec<usize>,
    /// Objects that are root children (slide units), in hierarchy order.
    pub top_children: Vec<usize>,
    /// Objects with Unity's built-in `SpriteMask` component: their own
    /// stencil value (their sprite draws and writes it; particle systems
    /// below them with "visible inside mask" show only there).
    mask_values: HashMap<usize, u8>,
}

fn affine(o: &Object) -> Affine2 {
    let m = o.world.matrix;
    Affine2::from_mat2_translation(
        Mat2::from_cols(Vec2::new(m[0], m[1]), Vec2::new(m[2], m[3])),
        Vec2::new(m[4], m[5]),
    )
}

fn has_script(o: &Object, script: &str) -> bool {
    o.components
        .iter()
        .any(|c| c.script.as_deref() == Some(script))
}

fn gradient_first(v: &serde_json::Value) -> Color {
    let k = &v["key0"];
    let f = |n: &str| k[n].as_f64().unwrap_or(0.0) as f32;
    Color::rgba(f("r"), f("g"), f("b"), f("a"))
}

impl MapView {
    /// Builds the view; `texture` maps an import texture id to a renderer
    /// texture (loading it on first use).
    pub fn new(
        data: &GameData,
        map: &Map,
        texture: &mut dyn FnMut(&str) -> Option<LoadedTexture>,
    ) -> MapView {
        let objs = &map.objects;
        let sorting_index: HashMap<u32, usize> = data
            .index
            .sorting_layers
            .iter()
            .enumerate()
            .map(|(i, l)| (l.id, i))
            .collect();
        // Anchors: nearest ancestor-or-self with a dynamic rigid body.
        let mut anchors: Vec<Anchor> = Vec::with_capacity(objs.len());
        for (i, o) in objs.iter().enumerate() {
            let initial = affine(o);
            let is_body = o
                .rigidbody
                .as_ref()
                .is_some_and(|r| r.body_type != 2 && r.simulated);
            let body = if is_body {
                Some((i, Affine2::IDENTITY))
            } else {
                o.parent
                    .and_then(|p| anchors[p].body)
                    .map(|(b, _)| (b, anchors[b].initial.inverse() * initial))
            };
            anchors.push(Anchor { initial, body });
        }
        // Built-in SpriteMask components: one stencil value each (240 and
        // down; players use 2..=13, the map's piece masks 1).
        let mut mask_values: HashMap<usize, u8> = HashMap::new();
        for (i, o) in objs.iter().enumerate() {
            if o.active_in_hierarchy
                && o.components.iter().any(|c| c.kind == "SpriteMask")
                && mask_values.len() < 200
            {
                mask_values.insert(i, 240 - mask_values.len() as u8);
            }
        }
        let mut items = Vec::new();
        let mut ropes = Vec::new();
        let mut casters = Vec::new();
        let mut seq = 0u32;
        for (i, o) in objs.iter().enumerate() {
            let z = o.world.trs.position[2];
            // Inactive objects only keep their sprites (hidden unless the
            // provider shows them, or drawn explicitly: debris pieces).
            for c in o.components.iter().filter(|_| o.active_in_hierarchy) {
                if c.script.as_deref() == Some("SFPolygon") {
                    let verts: Vec<Vec2> = c.fields["_verts"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|v| {
                            Vec2::new(
                                v["x"].as_f64().unwrap_or(0.0) as f32,
                                v["y"].as_f64().unwrap_or(0.0) as f32,
                            )
                        })
                        .collect();
                    if verts.len() >= 2 {
                        casters.push((i, verts));
                    }
                }
                if c.kind == "LineRenderer" && c.fields["m_Enabled"].as_bool() != Some(false) {
                    let f = &c.fields;
                    let p = &f["m_Parameters"];
                    let w = p["widthMultiplier"].as_f64().unwrap_or(0.1) as f32;
                    let a = Vec2::new(o.world.trs.position[0], o.world.trs.position[1]);
                    let b = o
                        .children
                        .first()
                        .map(|&ch| {
                            let t = &objs[ch].world.trs.position;
                            Vec2::new(t[0], t[1])
                        })
                        .unwrap_or(a);
                    let layer = sorting_index
                        .get(&(f["m_SortingLayerID"].as_i64().unwrap_or(0) as u32))
                        .copied()
                        .unwrap_or(0);
                    let order = f["m_SortingOrder"].as_i64().unwrap_or(0) as i32;
                    seq += 1;
                    ropes.push(Rope {
                        object: i,
                        a,
                        b,
                        width: w,
                        color: gradient_first(&p["colorGradient"]),
                        key: SortKey::new(layer, order, z, 1, seq),
                    });
                }
            }
            let Some(sr) = &o.sprite else { continue };
            if !sr.enabled {
                continue;
            }
            let masker = has_script(o, "SpriteMasking.SpriteMask");
            if !masker && sr.materials.is_empty() {
                // No material: Unity draws nothing.
                continue;
            }
            let Some(sp) = sr.sprite.as_ref().and_then(|id| data.sprites.get(id)) else {
                continue;
            };
            let Some((tex, (tw, th))) = sp.texture.as_deref().and_then(&mut *texture) else {
                continue;
            };
            let ppu = sp.pixels_per_unit.max(1e-3);
            let [tx, ty, tw_px, th_px] = sp.texture_rect;
            let rect_size = Vec2::new(sp.rect[2], sp.rect[3]) / ppu;
            let pivot = Vec2::from(sp.pivot);
            let full_uv = UvRect::from_pixels(tx, ty, tw_px, th_px, tw, th);
            let quads = match sr.draw_mode {
                1 | 2 => {
                    let size = Vec2::from(sr.size);
                    let min = -pivot * size;
                    if sr.draw_mode == 1 {
                        sliced(min, size, sp.border, ppu, [tx, ty, tw_px, th_px], (tw, th))
                    } else {
                        tiled(min, size, rect_size, full_uv)
                    }
                }
                _ => {
                    let off = Vec2::from(sp.texture_rect_offset) / ppu;
                    vec![Quad {
                        min: off - pivot * rect_size,
                        size: Vec2::new(tw_px, th_px) / ppu,
                        uv: full_uv,
                    }]
                }
            };
            seq += 1;
            let [r, g, b, a] = sr.color;
            items.push(Item {
                object: i,
                texture: tex,
                quads,
                tint: Color::rgba(r, g, b, a),
                flip: Vec2::new(
                    if sr.flip_x { -1.0 } else { 1.0 },
                    if sr.flip_y { -1.0 } else { 1.0 },
                ),
                mask: match (masker, mask_values.get(&i)) {
                    (true, _) => Mask::Write,
                    (false, Some(&v)) => Mask::WriteValue(v),
                    _ => Mask::None,
                },
                key: SortKey::new(
                    sr.sorting_layer_index,
                    sr.sorting_order,
                    z,
                    if masker { 0 } else { 1 },
                    seq,
                ),
            });
        }
        let size = map
            .map
            .as_ref()
            .and_then(|m| m["size"].as_f64())
            .unwrap_or(20.0) as f32;
        let mut top = vec![usize::MAX; objs.len()];
        let mut top_children = Vec::new();
        for (i, o) in objs.iter().enumerate() {
            match o.parent {
                None => {}
                Some(p) if objs[p].parent.is_none() => {
                    top[i] = i;
                    top_children.push(i);
                }
                Some(p) => top[i] = top[p],
            }
        }
        MapView {
            parent: objs.iter().map(|o| o.parent).collect(),
            top,
            top_children,
            mask_values,
            name: map.name.clone(),
            build_index: map.build_index,
            size,
            anchors,
            items,
            ropes,
            casters,
            active: objs.iter().map(|o| o.active_in_hierarchy).collect(),
        }
    }

    /// Current world transform of an object.
    /// Draws `object`'s sprite(s) at `at` (position and rotation; the
    /// object's imported scale and flip are kept) scaled by `k`, as debris
    /// pieces of destroyed boxes. Returns false if it has no sprite.
    pub fn object_sprites_at(
        &self,
        object: usize,
        at: Affine2,
        k: f32,
        out: &mut Vec<(SortKey, Sprite)>,
    ) -> bool {
        let init = self.anchors[object].initial;
        let sc = Vec2::new(init.matrix2.x_axis.length(), init.matrix2.y_axis.length()) * k;
        let mut any = false;
        for it in self.items.iter().filter(|it| it.object == object) {
            let w = at * Affine2::from_scale(sc * it.flip);
            for q in &it.quads {
                out.push((
                    it.key,
                    Sprite {
                        texture: it.texture,
                        uv: q.uv,
                        transform: w * Affine2::from_translation(q.min),
                        size: q.size,
                        pivot: Vec2::ZERO,
                        tint: it.tint,
                        mask: it.mask,
                        ..Default::default()
                    },
                ));
                any = true;
            }
        }
        any
    }

    pub fn world(&self, object: usize, poses: &dyn TransformProvider) -> Affine2 {
        let a = &self.anchors[object];
        let w = match a.body {
            Some((b, rel)) => poses.body_transform(b).map_or(a.initial, |t| t * rel),
            None => a.initial,
        };
        match self.top.get(object) {
            Some(&t) if t != usize::MAX => {
                let off = poses.slide(t);
                if off == Vec2::ZERO {
                    w
                } else {
                    Affine2::from_translation(off) * w
                }
            }
            _ => w,
        }
    }

    /// Makes `objects` anchors (their transforms come from
    /// [`TransformProvider::body_transform`] and their descendants follow),
    /// in addition to the rigid bodies: sim props, animated groups.
    pub fn set_driven(&mut self, objects: &[usize]) {
        let n = self.anchors.len();
        let mut driven = vec![false; n];
        for &o in objects {
            if o < n {
                driven[o] = true;
            }
        }
        #[allow(clippy::needless_range_loop)]
        for i in 0..n {
            let initial = self.anchors[i].initial;
            let parent_body = self.parent[i].and_then(|p| self.anchors[p].body);
            let own = self.anchors[i].body.is_some_and(|(b, _)| b == i);
            self.anchors[i].body = if driven[i] || own {
                Some((i, Affine2::IDENTITY))
            } else {
                parent_body.map(|(b, _)| (b, self.anchors[b].initial.inverse() * initial))
            };
        }
    }

    /// Stencil value of the nearest ancestor-or-self with a built-in
    /// `SpriteMask` component.
    pub fn mask_value_for(&self, mut object: usize) -> Option<u8> {
        loop {
            if let Some(v) = self.mask_values.get(&object) {
                return Some(*v);
            }
            object = self.parent.get(object).copied().flatten()?;
        }
    }

    /// Imported (initial) world transform of an object.
    pub fn initial(&self, object: usize) -> Affine2 {
        self.anchors[object].initial
    }

    pub fn visible(&self, object: usize, poses: &dyn TransformProvider) -> bool {
        poses.visible(object).unwrap_or(self.active[object])
    }

    /// Number of stencil writers, normal sprites, ropes and casters.
    pub fn counts(&self) -> (usize, usize, usize, usize) {
        let w = self.items.iter().filter(|i| i.mask == Mask::Write).count();
        (
            w,
            self.items.len() - w,
            self.ropes.len(),
            self.casters.len(),
        )
    }

    /// Appends the map's world sprites with their sort keys.
    pub fn sprites(&self, poses: &dyn TransformProvider, out: &mut Vec<(SortKey, Sprite)>) {
        for it in &self.items {
            if !self.visible(it.object, poses) {
                continue;
            }
            let w = self.world(it.object, poses) * Affine2::from_scale(it.flip);
            for q in &it.quads {
                out.push((
                    it.key,
                    Sprite {
                        texture: it.texture,
                        uv: q.uv,
                        transform: w * Affine2::from_translation(q.min),
                        size: q.size,
                        pivot: Vec2::ZERO,
                        tint: it.tint,
                        mask: it.mask,
                        ..Default::default()
                    },
                ));
            }
        }
        for r in &self.ropes {
            if !self.visible(r.object, poses) {
                continue;
            }
            let (a, b) = poses.rope_ends(r.object).unwrap_or_else(|| {
                let off = match self.top.get(r.object) {
                    Some(&t) if t != usize::MAX => poses.slide(t),
                    _ => Vec2::ZERO,
                };
                (r.a + off, r.b + off)
            });
            let d = b - a;
            let len = d.length();
            if len < 1e-4 {
                continue;
            }
            out.push((
                r.key,
                Sprite {
                    texture: TextureId::WHITE,
                    transform: Affine2::from_angle_translation(d.y.atan2(d.x), (a + b) * 0.5),
                    size: Vec2::new(len, r.width),
                    tint: r.color,
                    ..Default::default()
                },
            ));
        }
    }

    /// World-space shadow caster polygons.
    pub fn casters(&self, poses: &dyn TransformProvider, out: &mut Vec<Vec<Vec2>>) {
        for (obj, verts) in &self.casters {
            if !self.visible(*obj, poses) {
                continue;
            }
            let w = self.world(*obj, poses);
            out.push(verts.iter().map(|&v| w.transform_point2(v)).collect());
        }
    }
}

/// 9-slice quads for a sliced sprite of `size` with `border` px
/// `[left, bottom, right, top]`; `tr` is the texture rect (PNG space).
fn sliced(
    min: Vec2,
    size: Vec2,
    border: [f32; 4],
    ppu: f32,
    tr: [f32; 4],
    tex: (u32, u32),
) -> Vec<Quad> {
    let [l, b, r, t] = border;
    let (lu, bu, ru, tu) = (l / ppu, b / ppu, r / ppu, t / ppu);
    // Shrink the borders if the size is smaller than both borders.
    let sx = if lu + ru > size.x && lu + ru > 0.0 {
        size.x / (lu + ru)
    } else {
        1.0
    };
    let sy = if bu + tu > size.y && bu + tu > 0.0 {
        size.y / (bu + tu)
    } else {
        1.0
    };
    let xs = [0.0, lu * sx, size.x - ru * sx, size.x];
    let ys = [0.0, bu * sy, size.y - tu * sy, size.y];
    // Texture px columns (left to right) and rows (bottom to top, PNG y down).
    let us = [tr[0], tr[0] + l, tr[0] + tr[2] - r, tr[0] + tr[2]];
    let vs = [tr[1] + tr[3], tr[1] + tr[3] - b, tr[1] + t, tr[1]];
    let mut out = Vec::new();
    for j in 0..3 {
        for i in 0..3 {
            let w = xs[i + 1] - xs[i];
            let h = ys[j + 1] - ys[j];
            if w <= 1e-6 || h <= 1e-6 {
                continue;
            }
            let (u0, u1) = (us[i], us[i + 1]);
            let (v_top, v_bottom) = (vs[j + 1], vs[j]);
            out.push(Quad {
                min: min + Vec2::new(xs[i], ys[j]),
                size: Vec2::new(w, h),
                uv: UvRect::from_pixels(u0, v_top, u1 - u0, v_bottom - v_top, tex.0, tex.1),
            });
        }
    }
    out
}

/// Tiled quads: whole tiles from the bottom-left, the last row/column cut.
fn tiled(min: Vec2, size: Vec2, tile: Vec2, uv: UvRect) -> Vec<Quad> {
    let mut out = Vec::new();
    if tile.x <= 1e-4 || tile.y <= 1e-4 {
        return out;
    }
    let nx = (size.x / tile.x).ceil().min(256.0) as i32;
    let ny = (size.y / tile.y).ceil().min(256.0) as i32;
    for j in 0..ny {
        for i in 0..nx {
            let o = Vec2::new(i as f32 * tile.x, j as f32 * tile.y);
            let s = (size - o).min(tile);
            let f = s / tile;
            // Cut from the right and the top (uv v grows downwards).
            let uv = UvRect {
                min: Vec2::new(uv.min.x, uv.max.y - (uv.max.y - uv.min.y) * f.y),
                max: Vec2::new(uv.min.x + (uv.max.x - uv.min.x) * f.x, uv.max.y),
            };
            out.push(Quad {
                min: min + o,
                size: s,
                uv,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiles_cover_the_size() {
        let q = tiled(
            Vec2::ZERO,
            Vec2::new(10.0, 3.0),
            Vec2::splat(6.4),
            UvRect::FULL,
        );
        assert_eq!(q.len(), 2);
        assert!((q[1].size.x - 3.6).abs() < 1e-5 && (q[1].size.y - 3.0).abs() < 1e-5);
        let area: f32 = q.iter().map(|q| q.size.x * q.size.y).sum();
        assert!((area - 30.0).abs() < 1e-4);
    }

    #[test]
    fn nine_slices() {
        let q = sliced(
            Vec2::ZERO,
            Vec2::new(4.0, 2.0),
            [16.0, 16.0, 16.0, 16.0],
            32.0,
            [0.0, 0.0, 64.0, 64.0],
            (64, 64),
        );
        assert_eq!(q.len(), 9);
        let area: f32 = q.iter().map(|q| q.size.x * q.size.y).sum();
        assert!((area - 8.0).abs() < 1e-4);
    }

    #[test]
    fn sort_key_order() {
        let a = SortKey::new(16, 0, -3.0, 0, 5);
        let b = SortKey::new(16, 0, -4.24, 1, 1);
        let c = SortKey::new(17, 0, 0.0, 1, 0);
        assert!(a.cmp(&b).is_lt(), "farther z first");
        assert!(b.cmp(&c).is_lt(), "sorting layer first");
    }
}
