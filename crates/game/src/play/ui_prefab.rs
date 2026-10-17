//! A small Unity UI (uGUI) renderer for imported prefabs: resolves
//! `RectTransform` layouts and draws `Image`s (sprite or flat colour,
//! stretched to their rect) with `Mask` clipping (stencil values), and
//! exposes TextMeshPro text rects for the caller to fill. Used for the
//! cards (the `CardBase` frame and each card's art prefab).

use glam::{Affine2, Vec2};
use rounds_import::format::{Object, Prefab};
use rounds_render::{Color, Gpu, Renderer};

use super::assets::{PlayAssets, SpriteRef};

/// One resolved UI object.
#[derive(Clone, Debug)]
pub struct UiNode {
    pub name: String,
    pub parent: Option<usize>,
    /// Transform of the node's pivot frame in root-canvas units.
    pub tf: Affine2,
    /// Rect in the node's own frame (min corner, size).
    pub min: Vec2,
    pub size: Vec2,
    pub active: bool,
    pub image: Option<UiImage>,
    /// This image clips its descendants.
    pub mask: bool,
    pub text: Option<UiText>,
    /// `PositionNoise` (amount px, speed): the card art's idle wobble.
    pub noise: Option<(f32, f32)>,
    /// Local layout parameters (animated copies re-lay out from these).
    pub local: UiLocal,
    /// Transform path below the prefab root (`""` = root), for animation
    /// curve bindings.
    pub path: String,
    /// `CanvasGroup` alpha of this node times its ancestors'.
    pub alpha: f32,
}

/// A node's own (animatable) layout values.
#[derive(Clone, Copy, Debug, Default)]
pub struct UiLocal {
    /// RectTransform: anchor min/max, size delta, pivot, anchored position.
    pub rect: Option<[Vec2; 5]>,
    /// Plain Transform position (no RectTransform).
    pub pos: Vec2,
    /// Rotation about z (radians, counter-clockwise).
    pub angle: f32,
    pub scale: Vec2,
    pub active_self: bool,
    /// `CanvasGroup.m_Alpha` (1 without a group).
    pub group_alpha: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct UiImage {
    pub sprite: Option<SpriteRef>,
    pub color: Color,
    pub preserve_aspect: bool,
}

#[derive(Clone, Debug)]
#[allow(dead_code)] // the card code lays out its texts itself
pub struct UiText {
    pub size: f32,
    pub color: Color,
    /// 0 left, 0.5 centre, 1 right.
    pub align: f32,
    pub upper: bool,
    pub text: String,
}

#[derive(Clone, Debug, Default)]
pub struct UiPrefab {
    pub nodes: Vec<UiNode>,
    /// Size of the parent rect the root is laid out in.
    pub parent_size: Vec2,
    /// Every enabled `Animator`: its node and controller id.
    pub animators: Vec<(usize, String)>,
}

fn color(v: &serde_json::Value) -> Option<Color> {
    let f = |k: &str| v[k].as_f64().map(|x| x as f32);
    Some(Color::rgba(f("r")?, f("g")?, f("b")?, f("a")?))
}

fn comp<'a>(o: &'a Object, script: &str) -> Option<&'a serde_json::Value> {
    o.components
        .iter()
        .find(|c| c.script.as_deref() == Some(script))
        .map(|c| &c.fields)
}

impl UiPrefab {
    /// Resolves `prefab` with its root placed in a parent rect of
    /// `parent_size` (centred at the origin).
    pub fn new(
        gpu: &Gpu,
        renderer: &mut Renderer,
        assets: &mut PlayAssets,
        prefab: &Prefab,
        parent_size: Vec2,
    ) -> UiPrefab {
        let objs = &prefab.objects;
        let mut nodes: Vec<UiNode> = Vec::with_capacity(objs.len());
        let mut animators = Vec::new();
        for (i, o) in objs.iter().enumerate() {
            let rect = o.rect.as_ref().map(|r| {
                [
                    Vec2::from(r.anchor_min),
                    Vec2::from(r.anchor_max),
                    Vec2::from(r.size_delta),
                    Vec2::from(r.pivot),
                    Vec2::from(r.anchored_position),
                ]
            });
            let [qx, qy, qz, qw] = o.local.rotation;
            // Rotation about z (ignore the canvas' 180-degree y flip).
            let angle = if qx.abs() + qy.abs() > 0.9 {
                0.0
            } else {
                2.0 * qz.atan2(qw)
            };
            let scale = if i == 0 {
                Vec2::ONE
            } else {
                Vec2::new(o.local.scale[0], o.local.scale[1])
            };
            let group_alpha = o
                .components
                .iter()
                .find(|c| c.kind == "CanvasGroup")
                .and_then(|c| c.fields["m_Alpha"].as_f64())
                .unwrap_or(1.0) as f32;
            if let Some(c) = o.components.iter().find(|c| c.kind == "Animator")
                && c.enabled != Some(false)
                && let Some(ctrl) = c.fields["m_Controller"]["$ref"].as_str()
            {
                animators.push((i, ctrl.to_owned()));
            }
            let path = match o.parent {
                Some(p) if p < i && objs[p].parent.is_some() => {
                    format!("{}/{}", nodes[p].path, o.name)
                }
                Some(_) => o.name.clone(),
                None => String::new(),
            };
            let image = comp(o, "UnityEngine.UI.Image").map(|f| UiImage {
                sprite: f["m_Sprite"]["$ref"]
                    .as_str()
                    .and_then(|id| assets.sprite_id(gpu, renderer, id)),
                color: color(&f["m_Color"]).unwrap_or(Color::WHITE),
                preserve_aspect: f["m_PreserveAspect"].as_i64() == Some(1),
            });
            let mask = comp(o, "UnityEngine.UI.Mask").is_some();
            let text = comp(o, "TMPro.TextMeshProUGUI").map(|f| {
                let h = f["m_HorizontalAlignment"].as_i64().unwrap_or(1);
                UiText {
                    size: f["m_fontSize"].as_f64().unwrap_or(36.0) as f32,
                    color: color(&f["m_fontColor"]).unwrap_or(Color::WHITE),
                    align: match h {
                        2 => 0.5,
                        4 => 1.0,
                        _ => 0.0,
                    },
                    upper: f["m_fontStyle"].as_i64().unwrap_or(0) & 16 != 0,
                    text: f["m_text"].as_str().unwrap_or("").to_owned(),
                }
            });
            let noise = comp(o, "PositionNoise").map(|f| {
                (
                    f["amount"].as_f64().unwrap_or(0.0) as f32,
                    f["speed"].as_f64().unwrap_or(1.0) as f32,
                )
            });
            nodes.push(UiNode {
                name: o.name.clone(),
                parent: o.parent.filter(|&p| p < i),
                tf: Affine2::IDENTITY,
                min: Vec2::ZERO,
                size: Vec2::ZERO,
                active: true,
                image,
                mask,
                text,
                noise,
                local: UiLocal {
                    rect,
                    pos: Vec2::new(o.local.position[0], o.local.position[1]),
                    angle,
                    scale,
                    active_self: o.active,
                    group_alpha,
                },
                path,
                alpha: 1.0,
            });
        }
        let mut ui = UiPrefab {
            nodes,
            parent_size,
            animators,
        };
        ui.relayout();
        ui
    }

    /// Recomputes every node's transform, rect, activity and alpha from
    /// the local values (parents come first).
    pub fn relayout(&mut self) {
        let parent_size = self.parent_size;
        for i in 0..self.nodes.len() {
            let (p_tf, p_min, p_size, p_active, p_alpha) = match self.nodes[i].parent {
                Some(p) => {
                    let n = &self.nodes[p];
                    (n.tf, n.min, n.size, n.active, n.alpha)
                }
                None => (
                    Affine2::IDENTITY,
                    -parent_size * 0.5,
                    parent_size,
                    true,
                    1.0,
                ),
            };
            let l = self.nodes[i].local;
            let (pos, size, pivot) = match l.rect {
                Some([amin, amax, delta, pivot, anchored]) => {
                    let amin = p_min + amin * p_size;
                    let amax = p_min + amax * p_size;
                    let size = (amax - amin) + delta;
                    let reference = amin + (amax - amin) * pivot;
                    (reference + anchored, size, pivot)
                }
                None => (l.pos, Vec2::ZERO, Vec2::splat(0.5)),
            };
            let n = &mut self.nodes[i];
            n.tf = p_tf * Affine2::from_scale_angle_translation(l.scale, l.angle, pos);
            n.min = -pivot * size;
            n.size = size;
            n.active = l.active_self && p_active;
            n.alpha = l.group_alpha * p_alpha;
        }
    }

    pub fn find(&self, name: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.name == name)
    }

    /// Whether `node` is `ancestor` or below it.
    pub fn under(&self, node: usize, ancestor: usize) -> bool {
        let mut n = Some(node);
        while let Some(i) = n {
            if i == ancestor {
                return true;
            }
            n = self.nodes[i].parent;
        }
        false
    }

    /// Nearest masking ancestor (excluding the node itself).
    pub fn mask_of(&self, node: usize) -> Option<usize> {
        let mut n = self.nodes[node].parent;
        while let Some(i) = n {
            if self.nodes[i].mask {
                return Some(i);
            }
            n = self.nodes[i].parent;
        }
        None
    }

    /// Centre of a node's rect in root-canvas units.
    #[allow(dead_code)]
    pub fn center(&self, node: usize) -> Vec2 {
        let n = &self.nodes[node];
        n.tf.transform_point2(n.min + n.size * 0.5)
    }
}

/// A sprite (or flat quad) filling a node's rect under `world`.
pub fn image_sprite(
    world: Affine2,
    n: &UiNode,
    img: &UiImage,
    tint: Color,
) -> rounds_render::Sprite {
    let centre = n.min + n.size * 0.5;
    let mut size = n.size;
    if img.preserve_aspect
        && let Some(s) = img.sprite
    {
        let a = s.rect_size.x / s.rect_size.y.max(1e-6);
        if size.x / size.y.max(1e-6) > a {
            size.x = size.y * a;
        } else {
            size.y = size.x / a;
        }
    }
    let t = world * n.tf * Affine2::from_translation(centre);
    match img.sprite {
        Some(s) => s.fill(t, size, tint),
        None => rounds_render::Sprite {
            transform: t,
            size,
            tint,
            ..Default::default()
        },
    }
}

/// A card art prefab playing its `Animator` (import revision 2:
/// `rounds_import::animator::AnimatorPlayer`): each frame the base prefab
/// is copied, the sampled clips' curves are applied to the bound nodes
/// (anchored / local position, scale, z rotation, Image colour and sprite,
/// GameObject activity, CanvasGroup alpha) and the copy is re-laid out.
pub struct ArtAnim {
    base: UiPrefab,
    pub current: UiPrefab,
    /// Per Animator: its node, controller id and player.
    players: Vec<(usize, String, rounds_import::animator::AnimatorPlayer)>,
    /// Sprite-swap targets of the controller's clips.
    sprites: std::collections::HashMap<String, Option<SpriteRef>>,
}

impl ArtAnim {
    pub fn new(
        gpu: &Gpu,
        renderer: &mut Renderer,
        assets: &mut PlayAssets,
        base: UiPrefab,
    ) -> ArtAnim {
        let data = assets.data.clone();
        let anims = &data.animations;
        let players: Vec<_> = base
            .animators
            .iter()
            .filter_map(|(node, ctrl)| {
                let c = anims.controllers.get(ctrl)?;
                Some((
                    *node,
                    ctrl.clone(),
                    rounds_import::animator::AnimatorPlayer::new(c, 0),
                ))
            })
            .collect();
        let mut sprites = std::collections::HashMap::new();
        for c in players
            .iter()
            .filter_map(|(_, ctrl, _)| anims.controllers.get(ctrl))
        {
            for clip in c.clips.iter().filter_map(|id| anims.clips.get(id)) {
                for id in clip.pptr.iter().flatten() {
                    sprites
                        .entry(id.clone())
                        .or_insert_with(|| assets.sprite_id(gpu, renderer, id));
                }
            }
        }
        ArtAnim {
            current: base.clone(),
            base,
            players,
            sprites,
        }
    }

    /// Advances the animator by `dt` seconds and rebuilds `current`.
    pub fn update(&mut self, anims: &rounds_import::format::Animations, dt: f32) {
        if self.players.is_empty() {
            return;
        }
        let mut cur = self.base.clone();
        let mut players = std::mem::take(&mut self.players);
        for (root, ctrl_id, player) in &mut players {
            let Some(ctrl) = anims.controllers.get(ctrl_id.as_str()) else {
                continue;
            };
            player.update(ctrl, anims, dt);
            let samples = player.sample(ctrl, anims);
            self.apply(&mut cur, *root, &samples, anims);
        }
        self.players = players;
        cur.relayout();
        self.current = cur;
    }

    /// Applies one Animator's clip samples to `cur` (curve paths relative
    /// to the Animator's node `root`).
    fn apply(
        &self,
        cur: &mut UiPrefab,
        root: usize,
        samples: &[rounds_import::animator::ClipSample],
        anims: &rounds_import::format::Animations,
    ) {
        let root_path = cur.nodes[root].path.clone();
        // Path hash (relative to the Animator's object) -> node.
        let mut by_hash = std::collections::HashMap::new();
        for (i, n) in cur.nodes.iter().enumerate() {
            let rel = if n.path == root_path {
                Some(String::new())
            } else if root_path.is_empty() {
                Some(n.path.clone())
            } else {
                n.path
                    .strip_prefix(&format!("{root_path}/"))
                    .map(str::to_owned)
            };
            if let Some(rel) = rel {
                by_hash.insert(rounds_import::format::path_hash(&rel), i);
            }
        }
        for s in samples {
            let Some(clip) = anims.clips.get(&s.clip) else {
                continue;
            };
            let w = s.weight;
            for c in &clip.curves {
                let Some(&i) = by_hash.get(&c.path) else {
                    continue;
                };
                let v = c.eval(s.time);
                let base = &self.base.nodes[i];
                let n = &mut cur.nodes[i];
                let mix = |old: f32, new: f32| old + (new - old) * w;
                let prop = c.property.as_str();
                match (c.class_id, prop) {
                    (4, "position.x") | (4, "position.y") => {
                        let k = if prop.ends_with('x') { 0 } else { 1 };
                        let new = mix(base.local.pos[k], v);
                        n.local.pos[k] = new;
                        // A RectTransform's local position moves its
                        // anchored position by the same amount.
                        if let (Some(r), Some(br)) = (&mut n.local.rect, base.local.rect) {
                            r[4][k] = br[4][k] + (new - base.local.pos[k]);
                        }
                    }
                    (4, "scale.x") => n.local.scale.x = mix(base.local.scale.x, v),
                    (4, "scale.y") => n.local.scale.y = mix(base.local.scale.y, v),
                    (4, "euler.z") => {
                        n.local.angle = mix(base.local.angle, v.to_radians());
                    }
                    (224, "m_AnchoredPosition.x") | (224, "m_AnchoredPosition.y") => {
                        if let Some(r) = &mut n.local.rect {
                            let k = if prop.ends_with('x') { 0 } else { 1 };
                            let b = base.local.rect.map_or(0.0, |r| r[4][k]);
                            r[4][k] = mix(b, v);
                        }
                    }
                    (224, "m_SizeDelta.x") | (224, "m_SizeDelta.y") => {
                        if let Some(r) = &mut n.local.rect {
                            let k = if prop.ends_with('x') { 0 } else { 1 };
                            let b = base.local.rect.map_or(0.0, |r| r[2][k]);
                            r[2][k] = mix(b, v);
                        }
                    }
                    (1, "m_IsActive") => {
                        if w >= 0.5 {
                            n.local.active_self = v > 0.5;
                        }
                    }
                    (225, "m_Alpha") => n.local.group_alpha = mix(base.local.group_alpha, v),
                    (114, p) if p.starts_with("m_Color.") => {
                        if let (Some(img), Some(bimg)) = (&mut n.image, &base.image) {
                            let ch = |c: &mut Color, b: &Color| match &p[8..] {
                                "r" => c.r = mix(b.r, v),
                                "g" => c.g = mix(b.g, v),
                                "b" => c.b = mix(b.b, v),
                                "a" => c.a = mix(b.a, v),
                                _ => {}
                            };
                            ch(&mut img.color, &bimg.color);
                        }
                    }
                    (114, "m_Sprite") if c.pptr && w >= 0.5 => {
                        if let Some(img) = &mut n.image
                            && let Some(Some(id)) = clip.pptr.get(v.max(0.0) as usize)
                            && let Some(sp) = self.sprites.get(id)
                        {
                            img.sprite = *sp;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}
