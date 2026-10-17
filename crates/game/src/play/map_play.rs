//! The maps of a match: the imported look of the current map (and of the
//! previous one while it slides out), positioned by the sim: physics props,
//! animated groups (smashers), ropes, destroyed boxes and their debris, and
//! the per-child slide in/out (match-flow.md 7.1).

use std::collections::HashMap;

use glam::{Affine2, Vec2};
use rounds_render::{Color, DrawList, Gpu, Mask, Renderer, Sprite, TextureId};
use rounds_sim::{Content, MapId, World};

use super::fx::{Key, curve};
use crate::art::MainLook;
use crate::map_scene::{MapPresentation, TextureCache};
use crate::map_view::{SortKey, TransformProvider};

use rounds_import::GameData;

/// Slide-in distance (units).
pub const PADDING: f32 = 90.0;
/// Slide curve over 0.7 s.
const SLIDE: [Key; 3] = [
    Key(0.0, 0.0, 0.0, 0.0),
    Key(0.35, 0.4978, 3.5913, 3.5913),
    Key(0.70, 1.0, 0.0, 0.0),
];
const SLIDE_TIME: f32 = 0.7;

/// Links between the sim's map content and the imported objects.
struct Links {
    /// Per sim prop (def index): its import object.
    prop_obj: Vec<Option<usize>>,
    /// Per sim prop: initial (def) pose.
    prop_pose: Vec<(Vec2, f32)>,
    /// Per import object: the prop it moves with.
    obj_prop: Vec<Option<usize>>,
    /// Per animated group: its import object.
    anim_obj: Vec<Option<usize>>,
    /// Per import object: the rope (index into `ropes`) it draws.
    rope_of: HashMap<usize, usize>,
    /// Per top-level child (by object index): slide delay (s).
    delay: HashMap<usize, f32>,
    /// Spinning saw objects: (object, deg/s of game time). Static saws
    /// spin visually (`Rotate`); prop saws spin with their body.
    saws: HashMap<usize, f32>,
    /// Per sim prop: the import objects of its destructible pieces.
    piece_obj: Vec<Vec<Option<usize>>>,
}

/// A sim rope def's authored end points (to match LineRenderer objects).
#[derive(Clone, Copy, Debug)]
struct RopeLink {
    start: Vec2,
    end: Vec2,
}

/// One map on screen.
pub struct ShownMap {
    pub id: MapId,
    pub pres: MapPresentation,
    links: Links,
    /// Game time for visual spinners (saws).
    pub spin_time: f32,
}

impl ShownMap {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        gpu: &Gpu,
        renderer: &mut Renderer,
        textures: &mut TextureCache,
        data: &GameData,
        look: &MainLook,
        content: &Content,
        id: MapId,
        art: usize,
        seed: u32,
    ) -> ShownMap {
        let mut pres =
            MapPresentation::new(gpu, renderer, textures, data, look, id.0 as usize, art);
        let map = &data.maps[id.0 as usize];
        let def = content.map(id);
        let by_id: HashMap<&str, usize> = map
            .objects
            .iter()
            .enumerate()
            .map(|(i, o)| (o.id.as_str(), i))
            .collect();
        let prop_obj: Vec<Option<usize>> = def
            .props
            .iter()
            .map(|p| by_id.get(p.id.as_str()).copied())
            .collect();
        let anim_obj: Vec<Option<usize>> = def
            .animated
            .iter()
            .map(|a| by_id.get(a.id.as_str()).copied())
            .collect();
        let saws: HashMap<usize, f32> = def
            .statics
            .iter()
            .filter_map(|st| {
                let db = st.damage_box.as_ref()?;
                let o = by_id.get(st.id.as_str()).copied()?;
                (db.rotate_speed != 0.0).then_some((o, db.rotate_speed))
            })
            .collect();
        let piece_obj: Vec<Vec<Option<usize>>> = def
            .props
            .iter()
            .map(|p| {
                p.destructible
                    .iter()
                    .flat_map(|d| &d.pieces)
                    .map(|pc| by_id.get(pc.id.as_str()).copied())
                    .collect()
            })
            .collect();
        let mut driven: Vec<usize> = prop_obj.iter().flatten().copied().collect();
        driven.extend(saws.keys().copied());
        driven.extend(anim_obj.iter().flatten().copied());
        pres.view.set_driven(&driven);
        // Objects under a prop.
        let mut obj_prop = vec![None; map.objects.len()];
        let prop_of_obj: HashMap<usize, usize> = prop_obj
            .iter()
            .enumerate()
            .filter_map(|(pi, o)| o.map(|o| (o, pi)))
            .collect();
        for (i, o) in map.objects.iter().enumerate() {
            obj_prop[i] = prop_of_obj
                .get(&i)
                .copied()
                .or_else(|| o.parent.and_then(|p| obj_prop[p]));
        }
        // Ropes: LineRenderer objects matched to the sim's rope defs.
        let ropes: Vec<RopeLink> = def
            .ropes
            .iter()
            .map(|r| RopeLink {
                start: Vec2::new(r.start.x, r.start.y),
                end: Vec2::new(r.end.x, r.end.y),
            })
            .collect();
        let mut rope_of = HashMap::new();
        for (i, o) in map.objects.iter().enumerate() {
            if !o.components.iter().any(|c| c.kind == "LineRenderer") {
                continue;
            }
            let p = Vec2::new(o.world.trs.position[0], o.world.trs.position[1]);
            if let Some(ri) = ropes
                .iter()
                .position(|r| (r.start - p).length() < 0.1 || (r.end - p).length() < 0.1)
            {
                rope_of.insert(i, ri);
            }
        }
        // Per-child slide delays (cosmetic, from the sim's seed).
        let mut rng = super::fx::Rng::new(seed as u64 ^ 0x51de);
        let delay = pres
            .view
            .top_children
            .iter()
            .map(|&c| (c, rng.range(0.0, 0.25)))
            .collect();
        ShownMap {
            id,
            pres,
            links: Links {
                prop_pose: def
                    .props
                    .iter()
                    .map(|p| (Vec2::new(p.pos.x, p.pos.y), p.angle))
                    .collect(),
                prop_obj,
                obj_prop,
                anim_obj,
                rope_of,
                delay,
                saws,
                piece_obj,
            },
            spin_time: 0.0,
        }
    }

    /// Advances the map's own particle effects (saw sparks, ...) with the
    /// live poses of the settled current map.
    pub fn update_fx(&mut self, world: &World, real: f32, game: f32) {
        let live = world.map_view().current == Some(self.id);
        let motion = if live {
            Motion::Settled
        } else {
            Motion::Waiting
        };
        // `Poses` borrows self immutably: take the effect list out.
        let mut fx = std::mem::take(&mut self.pres.fx);
        {
            let provider = Poses {
                map: self,
                world,
                motion,
                live,
            };
            MapPresentation::update_fx_list(&self.pres.view, &mut fx, &provider, real, game);
        }
        self.pres.fx = fx;
    }

    /// Draws this map with `slide` = (sliding in?, seconds since call-in),
    /// or `None` when it is the settled current map.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &self,
        look: &MainLook,
        world: &World,
        motion: Motion,
        list: &mut DrawList,
        art: bool,
        lights: bool,
    ) {
        let provider = Poses {
            map: self,
            world,
            motion,
            live: matches!(motion, Motion::Settled) && world.map_view().current == Some(self.id),
        };
        // Debris pieces of destroyed boxes: stencil writers like the boxes.
        let mut extra = Vec::new();
        if provider.live {
            for (i, piece) in world.debris().iter().enumerate() {
                let k = piece.scale();
                if k <= 0.01 {
                    continue;
                }
                let at = Affine2::from_angle_translation(
                    piece.angle,
                    Vec2::new(piece.pos.x, piece.pos.y),
                );
                // The piece object's own sprite (its renderer / mask).
                let obj = world
                    .props()
                    .get(piece.prop as usize)
                    .and_then(|pr| self.links.piece_obj.get(pr.def as usize))
                    .and_then(|v| v.get(piece.piece as usize).copied().flatten());
                if obj.is_some_and(|o| self.pres.view.object_sprites_at(o, at, k, &mut extra)) {
                    continue;
                }
                extra.push((
                    SortKey::new(0, i32::MIN, 0.0, 0, i as u32),
                    Sprite {
                        texture: TextureId::WHITE,
                        transform: at,
                        size: Vec2::new(piece.half.x, piece.half.y) * 2.0 * k,
                        tint: Color::WHITE,
                        mask: Mask::Write,
                        ..Default::default()
                    },
                ));
            }
        }
        self.pres
            .draw_world(look, &provider, list, extra, art, lights);
    }
}

/// How a map is moving this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Motion {
    /// Waiting off-screen right (loaded, not called in).
    Waiting,
    /// Sliding in, seconds since the call-in.
    Entering(f32),
    /// Sliding out, seconds since the exit started.
    Leaving(f32),
    Settled,
}

struct Poses<'a> {
    map: &'a ShownMap,
    world: &'a World,
    motion: Motion,
    live: bool,
}

impl TransformProvider for Poses<'_> {
    fn body_transform(&self, object: usize) -> Option<Affine2> {
        let l = &self.map.links;
        let view = &self.map.pres.view;
        if let Some(ai) = l.anim_obj.iter().position(|o| *o == Some(object)) {
            let scale = self
                .world
                .map_anims()
                .get(ai)
                .filter(|_| self.live || matches!(self.motion, Motion::Entering(_)))
                .map_or(1.0, |a| a.scale);
            return Some(view.initial(object) * Affine2::from_scale(Vec2::new(1.0, scale)));
        }
        if let Some(speed) = l.saws.get(&object) {
            let a = (speed * self.map.spin_time).to_radians();
            return Some(view.initial(object) * Affine2::from_angle(-a));
        }
        if !self.live {
            return None;
        }
        let pi = l.prop_obj.iter().position(|o| *o == Some(object))?;
        let pr = self.world.props().iter().find(|p| p.def as usize == pi)?;
        let (p0, a0) = l.prop_pose[pi];
        let delta = Affine2::from_angle_translation(pr.angle, Vec2::new(pr.pos.x, pr.pos.y))
            * Affine2::from_angle_translation(a0, p0).inverse();
        Some(delta * view.initial(object))
    }

    fn rope_ends(&self, object: usize) -> Option<(Vec2, Vec2)> {
        if !self.live {
            return None;
        }
        // The sim's ropes (World::ropes): empty before the map is released
        // (then the authored points are drawn).
        let ri = *self.map.links.rope_of.get(&object)?;
        let rv = self
            .world
            .ropes()
            .into_iter()
            .find(|r| r.def as usize == ri)?;
        let p = |v: rounds_sim::V2| Vec2::new(v.x, v.y);
        Some((p(rv.ends[0]), p(rv.ends[1])))
    }

    fn visible(&self, object: usize) -> Option<bool> {
        if !self.live {
            return None;
        }
        // A rope the sim dropped (its prop was destroyed) is gone.
        if let Some(&ri) = self.map.links.rope_of.get(&object) {
            let ropes = self.world.ropes();
            if !ropes.is_empty() && !ropes.iter().any(|r| r.def as usize == ri) {
                return Some(false);
            }
        }
        let pi = self.map.links.obj_prop.get(object).copied().flatten()?;
        let alive = self
            .world
            .props()
            .iter()
            .find(|p| p.def as usize == pi)
            .is_none_or(|p| p.alive);
        if alive { None } else { Some(false) }
    }

    fn slide(&self, child: usize) -> Vec2 {
        let delay = self.map.links.delay.get(&child).copied().unwrap_or(0.0);
        let x = match self.motion {
            Motion::Waiting => PADDING,
            Motion::Settled => 0.0,
            Motion::Entering(t) => {
                PADDING
                    * (1.0
                        - curve(
                            &SLIDE,
                            ((t - delay) / SLIDE_TIME).clamp(0.0, 1.0) * SLIDE_TIME,
                        ))
            }
            Motion::Leaving(t) => {
                -PADDING
                    * curve(
                        &SLIDE,
                        ((t - delay) / SLIDE_TIME).clamp(0.0, 1.0) * SLIDE_TIME,
                    )
            }
        };
        Vec2::new(x, 0.0)
    }
}
