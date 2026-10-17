//! Shows an original map with the original look: art particles, the light
//! camera's fog and shadows, both post-process grades. Also the windowed
//! map viewer (Left/Right: map, Up/Down: art) and the contact sheet of all
//! maps (`--map-sheet`).

use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

use anyhow::{Context, Result, bail};
use glam::Vec2;
use rounds_import::GameData;
use rounds_render::{
    BackgroundSettings, Camera2D, Color, DrawList, Filter, FrameSettings, Gpu, Image, Layer, Light,
    LightingSettings, OFFSCREEN_FORMAT, OffscreenTarget, PostSettings, Renderer, Sprite, TextureId,
    TextureOptions, Wrap,
};

use crate::app::{DrawContext, LoadContext, Scene, TickContext};
use crate::art::{ArtSystem, MainLook};
use crate::input::Buttons;
use crate::map_view::{InitialPose, LoadedTexture, MapView, SortKey, TransformProvider};

/// The 16:9 play field of the original (maps.md 9).
pub const TARGET_ASPECT: f32 = 16.0 / 9.0;

/// Loads import textures into the renderer on first use.
#[derive(Default)]
pub struct TextureCache {
    map: HashMap<String, Option<LoadedTexture>>,
}

impl TextureCache {
    pub fn get(
        &mut self,
        gpu: &Gpu,
        renderer: &mut Renderer,
        data: &GameData,
        id: &str,
    ) -> Option<LoadedTexture> {
        if let Some(t) = self.map.get(id) {
            return *t;
        }
        let t = (|| {
            let info = data.textures.get(id)?;
            let path = data.texture_path(id)?;
            let img = match Image::load_png(&path) {
                Ok(i) => i,
                Err(e) => {
                    log::warn!("texture {id} ({}): {e}", path.display());
                    return None;
                }
            };
            let opts = TextureOptions {
                filter: if info.filter_mode == 0 {
                    Filter::Nearest
                } else {
                    Filter::Linear
                },
                wrap: if info.wrap_mode == 0 {
                    Wrap::Repeat
                } else {
                    Wrap::Clamp
                },
                srgb: true,
            };
            Some((
                renderer.create_texture(gpu, &img, opts),
                (img.width, img.height),
            ))
        })();
        self.map.insert(id.to_owned(), t);
        t
    }
}

/// Resolves `--map N|name` to an index into `data.maps`. `N` is the map
/// number 0..69 (the order of the build settings, as in maps.md section 8);
/// names match the scene name case-insensitively, with or without `_M`.
pub fn find_map(data: &GameData, key: &str) -> Result<usize> {
    if let Ok(n) = key.parse::<usize>() {
        if n < data.maps.len() {
            return Ok(n);
        }
        bail!("map number {n} out of range (0..{})", data.maps.len() - 1);
    }
    let k = key.to_ascii_lowercase();
    let norm = |s: &str| s.to_ascii_lowercase().trim_end_matches("_m").to_owned();
    data.maps
        .iter()
        .position(|m| m.name.to_ascii_lowercase() == k || norm(&m.name) == norm(&k))
        .with_context(|| {
            let names: Vec<_> = data.maps.iter().map(|m| m.name.as_str()).collect();
            format!("no map named {key:?}; maps: {}", names.join(", "))
        })
}

/// A map with its art, ready to draw.
pub struct MapPresentation {
    pub view: MapView,
    pub art_index: usize,
    art: Vec<ArtSystem>,
    cookies: Vec<Option<TextureId>>,
    /// The map's own particle systems (saw sparks, ...), per top-most
    /// particle object: (object, its imported 3D transform, effect).
    pub(crate) fx: Vec<MapFx>,
}

pub(crate) type MapFx = (usize, glam::Mat4, crate::psys::Effect);

/// The map objects with particle systems whose ancestors have none, as
/// effects (`play_on_awake` systems of active objects play).
fn map_effects(
    data: &GameData,
    map_index: usize,
    view: &MapView,
    texture: &mut dyn FnMut(&str) -> Option<TextureId>,
) -> Vec<(usize, glam::Mat4, crate::psys::Effect)> {
    use crate::psys::{Effect, EffectDef, Play};
    let objs = &data.maps[map_index].objects;
    let mut out = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        if o.particles.is_none() || !o.active_in_hierarchy {
            continue;
        }
        let mut p = o.parent;
        let mut covered = false;
        while let Some(j) = p {
            covered |= objs[j].particles.is_some();
            p = objs[j].parent;
        }
        if covered {
            continue;
        }
        let mut def = EffectDef::from_objects(&o.name, objs, i, data, texture);
        // "Visible inside mask" against the owning built-in SpriteMask.
        for s in &mut def.systems {
            if s.look.mask == rounds_render::Mask::VisibleInside
                && let Some(v) = view.mask_value_for(s.object)
            {
                s.look.mask = rounds_render::Mask::InsideValue(v);
            }
        }
        let def = std::rc::Rc::new(def);
        if def.systems.is_empty() {
            continue;
        }
        let w = &o.world.trs;
        let m = glam::Mat4::from_scale_rotation_translation(
            glam::Vec3::from(w.scale),
            glam::Quat::from_array(w.rotation).normalize(),
            glam::Vec3::from(w.position),
        );
        let g = (
            match data.index.settings.gravity_3d[1] {
                0.0 => -9.81,
                g => g,
            },
            data.index.settings.gravity[1],
        );
        let fx = Effect::new(def, m, 0xfa11 ^ (i as u64) << 8, Play::Awake).with_gravity(g.0, g.1);
        out.push((i, m, fx));
    }
    out
}

impl MapPresentation {
    pub fn new(
        gpu: &Gpu,
        renderer: &mut Renderer,
        textures: &mut TextureCache,
        data: &GameData,
        look: &MainLook,
        map_index: usize,
        art_index: usize,
    ) -> MapPresentation {
        let map = &data.maps[map_index];
        let view = MapView::new(data, map, &mut |id| textures.get(gpu, renderer, data, id));
        let art_index = if look.arts.is_empty() {
            0
        } else {
            art_index % look.arts.len()
        };
        let art = look
            .arts
            .get(art_index)
            .map(|a| {
                look.start_art(data, a, 0x5eed ^ map_index as u64, &mut |id| {
                    textures
                        .get(gpu, renderer, data, id)
                        .map_or(TextureId::WHITE, |t| t.0)
                })
            })
            .unwrap_or_default();
        let cookies = look
            .lights
            .iter()
            .map(|l| {
                l.cookie
                    .as_deref()
                    .and_then(|c| textures.get(gpu, renderer, data, c))
                    .map(|t| t.0)
            })
            .collect();
        let fx = map_effects(data, map_index, &view, &mut |id| {
            textures.get(gpu, renderer, data, id).map(|t| t.0)
        });
        MapPresentation {
            view,
            art_index,
            art,
            cookies,
            fx,
        }
    }

    /// Moves the map's particle effects with their objects and advances
    /// them.
    pub fn update_fx(&mut self, poses: &dyn TransformProvider, real: f32, game: f32) {
        let mut fx = std::mem::take(&mut self.fx);
        Self::update_fx_list(&self.view, &mut fx, poses, real, game);
        self.fx = fx;
    }

    /// [`MapPresentation::update_fx`] on a list taken out of the
    /// presentation (when `poses` borrows it).
    pub(crate) fn update_fx_list(
        view: &MapView,
        list: &mut [MapFx],
        poses: &dyn TransformProvider,
        real: f32,
        game: f32,
    ) {
        for (obj, m, fx) in list {
            let cur = view.world(*obj, poses);
            let init = view.initial(*obj);
            let d = cur * init.inverse();
            let d4 = glam::Mat4::from_cols(
                glam::Vec4::new(d.matrix2.x_axis.x, d.matrix2.x_axis.y, 0.0, 0.0),
                glam::Vec4::new(d.matrix2.y_axis.x, d.matrix2.y_axis.y, 0.0, 0.0),
                glam::Vec4::Z,
                glam::Vec4::new(d.translation.x, d.translation.y, 0.0, 1.0),
            );
            fx.root = d4 * *m;
            if !view.visible(*obj, poses) {
                fx.stop();
            }
            fx.update(real, game);
        }
    }

    /// Advances the art particles by `dt` seconds.
    pub fn update(&mut self, dt: f32) {
        for s in &mut self.art {
            s.system.update(dt);
        }
    }

    /// Fills the frame settings and the draw list for a target of `size`.
    pub fn draw(
        &self,
        look: &MainLook,
        poses: &dyn TransformProvider,
        frame: &mut FrameSettings,
        list: &mut DrawList,
        size: (u32, u32),
    ) {
        self.frame_settings(look, frame, size);
        self.draw_world(look, poses, list, Vec::new(), true, true);
        letterbox(list, size);
    }

    /// Camera, clear colour, lighting, background stage and post grading.
    pub fn frame_settings(&self, look: &MainLook, frame: &mut FrameSettings, size: (u32, u32)) {
        // Camera: the map's orthographic size on a 16:9 field; narrower
        // targets get letterbox bars, wider ones see more to the sides.
        let aspect = size.0 as f32 / size.1.max(1) as f32;
        let ortho = if self.view.size > 0.0 {
            self.view.size
        } else {
            look.ortho_size
        };
        let fit = if aspect < TARGET_ASPECT {
            ortho * TARGET_ASPECT / aspect
        } else {
            ortho
        };
        frame.camera = Camera2D::new(Vec2::ZERO, fit);
        frame.clear = Color::BLACK;
        let sf = &look.sf;
        frame.lighting = LightingSettings {
            enabled: true,
            ambient: sf.ambient,
            exposure: sf.exposure,
            light_map_scale: sf.light_map_scale,
            shadow_map_scale: sf.shadow_map_scale,
            multiply_world: false,
        };
        let art = look.arts.get(self.art_index);
        frame.background = Some(BackgroundSettings {
            clear: look.background_clear.with_alpha(1.0),
            fog: Some(sf.fog),
            grading: art.and_then(|a| a.profile.grading),
        });
        frame.post = PostSettings {
            grading: look.main_profile.grading,
            unity_vignette: look.main_profile.vignette,
            bloom: look.main_profile.bloom,
            ..Default::default()
        };
    }

    /// The map's lights, shadow casters, sprites and (with `art`) the art
    /// particles; `extra` world sprites are sorted in with the map's.
    /// `lights`: also add the scene lights (once per frame).
    pub fn draw_world(
        &self,
        look: &MainLook,
        poses: &dyn TransformProvider,
        list: &mut DrawList,
        extra: Vec<(SortKey, Sprite)>,
        art: bool,
        lights: bool,
    ) {
        let sf = &look.sf;
        if lights {
            for (l, cookie) in look.lights.iter().zip(&self.cookies) {
                list.light(Light {
                    position: l.position,
                    color: l.color,
                    intensity: l.intensity,
                    range: l.size.length() * 0.5,
                    source_radius: l.radius,
                    casts_shadows: sf.shadows,
                    cookie: *cookie,
                    cookie_size: l.size,
                });
            }
        }
        let mut polys = Vec::new();
        self.view.casters(poses, &mut polys);
        for p in &polys {
            list.shadow_caster(p);
        }

        // Background stage (light camera) and world (main camera).
        let mut bg: Vec<(SortKey, Sprite)> = Vec::new();
        let mut world: Vec<(SortKey, Sprite)> = extra;
        self.view.sprites(poses, &mut world);
        let mut seq = 1_000_000u32;
        let mut tmp = Vec::new();
        let mut fx_sprites = Vec::new();
        for (obj, _, fx) in &self.fx {
            if self.view.visible(*obj, poses) {
                fx.sprites(&mut fx_sprites);
            }
        }
        for ((layer, order), q) in fx_sprites {
            seq += 1;
            world.push((SortKey::new(layer, order, 0.0, 1, seq), q));
        }
        if art {
            for s in &self.art {
                tmp.clear();
                s.system.sprites(&mut tmp);
                let dst = if s.background { &mut bg } else { &mut world };
                for q in tmp.drain(..) {
                    seq += 1;
                    dst.push((
                        SortKey::new(s.sorting_layer, s.sorting_order, s.z, 1, seq),
                        q,
                    ));
                }
            }
        }
        for (items, layer) in [(&mut bg, Layer::Background), (&mut world, Layer::World)] {
            items.sort_by(|a, b| a.0.cmp(&b.0));
            for (i, (_, s)) in items.drain(..).enumerate() {
                list.sprite(layer, i as i32, s);
            }
        }
    }

    /// Number of live art particles.
    pub fn particles(&self) -> usize {
        self.art.iter().map(|s| s.system.len()).sum()
    }
}

/// Letterbox bars below 16:9.
pub fn letterbox(list: &mut DrawList, size: (u32, u32)) {
    let aspect = size.0 as f32 / size.1.max(1) as f32;
    if aspect < TARGET_ASPECT {
        let h = size.0 as f32 / TARGET_ASPECT;
        let bar = (size.1 as f32 - h) * 0.5;
        let (w, hh) = (size.0 as f32, size.1 as f32);
        list.rect(
            Layer::Screen,
            i32::MAX,
            Vec2::new(w * 0.5, bar * 0.5),
            Vec2::new(w, bar),
            0.0,
            Color::BLACK,
        );
        list.rect(
            Layer::Screen,
            i32::MAX,
            Vec2::new(w * 0.5, hh - bar * 0.5),
            Vec2::new(w, bar),
            0.0,
            Color::BLACK,
        );
    }
}

/// The `Scene` showing one map; in a window, Left/Right change the map and
/// Up/Down the art.
pub struct MapScene {
    data: Rc<GameData>,
    look: Rc<MainLook>,
    map_index: usize,
    art_index: usize,
    textures: TextureCache,
    current: Option<MapPresentation>,
    dirty: bool,
    /// Print what is shown when the map changes.
    pub verbose: bool,
}

impl MapScene {
    pub fn new(data: Rc<GameData>, map_index: usize, art_index: Option<usize>) -> MapScene {
        let look = Rc::new(MainLook::from_data(&data));
        let n_arts = look.arts.len().max(1);
        MapScene {
            data,
            look,
            map_index,
            art_index: art_index.unwrap_or(map_index % n_arts),
            textures: TextureCache::default(),
            current: None,
            dirty: true,
            verbose: true,
        }
    }

    fn rebuild(&mut self, gpu: &Gpu, renderer: &mut Renderer) {
        let p = MapPresentation::new(
            gpu,
            renderer,
            &mut self.textures,
            &self.data,
            &self.look,
            self.map_index,
            self.art_index,
        );
        if self.verbose {
            let (w, s, r, c) = p.view.counts();
            let art = self
                .look
                .arts
                .get(p.art_index)
                .map_or("-", |a| a.profile.name.as_str());
            println!(
                "map {} {} (art {} {}): {w} masked pieces, {s} sprites, {r} ropes, {c} shadow casters, {} particles",
                self.map_index,
                p.view.name,
                p.art_index,
                art,
                p.particles()
            );
        }
        self.current = Some(p);
        self.dirty = false;
    }
}

impl Scene for MapScene {
    fn load(&mut self, ctx: &mut LoadContext<'_>) -> Result<()> {
        if self.data.main.is_none() {
            log::warn!("the import has no main scene (main.json); re-import for the original look");
        }
        self.rebuild(ctx.gpu, ctx.renderer);
        Ok(())
    }

    fn poll(&mut self, ctx: &mut LoadContext<'_>) -> Result<()> {
        if self.dirty {
            self.rebuild(ctx.gpu, ctx.renderer);
        }
        Ok(())
    }

    fn tick(&mut self, ctx: &mut TickContext<'_>) {
        let n = self.data.maps.len();
        let arts = self.look.arts.len().max(1);
        for (_, input) in ctx.inputs {
            let p = input.pressed;
            if p.contains(Buttons::RIGHT) {
                self.map_index = (self.map_index + 1) % n;
                self.dirty = true;
            }
            if p.contains(Buttons::LEFT) {
                self.map_index = (self.map_index + n - 1) % n;
                self.dirty = true;
            }
            if p.contains(Buttons::UP) {
                self.art_index = (self.art_index + 1) % arts;
                self.dirty = true;
            }
            if p.contains(Buttons::DOWN) {
                self.art_index = (self.art_index + arts - 1) % arts;
                self.dirty = true;
            }
        }
        if let Some(c) = &mut self.current {
            c.update(ctx.dt);
            c.update_fx(&InitialPose, ctx.dt, ctx.dt);
        }
    }

    fn draw(&mut self, ctx: &mut DrawContext<'_>) {
        if let Some(c) = &self.current {
            c.draw(&self.look, &InitialPose, ctx.frame, ctx.list, ctx.size);
        }
    }
}

/// Copies `src` into `dst` at (x, y).
fn blit(dst: &mut Image, src: &Image, x: u32, y: u32) {
    for row in 0..src.height {
        let dy = y + row;
        if dy >= dst.height {
            break;
        }
        let w = src.width.min(dst.width.saturating_sub(x)) as usize;
        let s = (row * src.width) as usize * 4;
        let d = (dy * dst.width + x) as usize * 4;
        dst.pixels[d..d + w * 4].copy_from_slice(&src.pixels[s..s + w * 4]);
    }
}

/// Renders every map through the real renderer into a contact sheet.
/// `cell` is the size of one map; `art` forces one art for all maps
/// (default: map number modulo the number of arts, so all arts appear).
pub fn map_sheet(
    data: Rc<GameData>,
    cell: (u32, u32),
    art: Option<usize>,
    out: &Path,
) -> Result<()> {
    let gpu = Gpu::headless().context("creating a headless GPU device")?;
    println!("GPU: {}", gpu.describe());
    let mut renderer = Renderer::new(&gpu, OFFSCREEN_FORMAT);
    let target = OffscreenTarget::new(&gpu, cell, OFFSCREEN_FORMAT);
    let look = MainLook::from_data(&data);
    let n = data.maps.len();
    let cols = 10u32.min(n as u32).max(1);
    let rows = (n as u32).div_ceil(cols);
    let mut sheet = Image::solid(cols * cell.0, rows * cell.1, [0, 0, 0, 255]);
    let mut textures = TextureCache::default();
    let mut list = DrawList::new();
    let n_arts = look.arts.len().max(1);
    let label_px = (cell.1 as f32 / 16.0).max(10.0);
    for i in 0..n {
        let a = art.unwrap_or(i % n_arts);
        let p = MapPresentation::new(&gpu, &mut renderer, &mut textures, &data, &look, i, a);
        let mut frame = FrameSettings::default();
        list.clear();
        p.draw(&look, &InitialPose, &mut frame, &mut list, cell);
        let art_name = look
            .arts
            .get(p.art_index)
            .map_or("-", |a| a.profile.name.as_str());
        let label = format!("{i} {} ({art_name})", p.view.name);
        for (d, c) in [(1.0, Color::BLACK), (0.0, Color::WHITE)] {
            list.text(
                Layer::Screen,
                i32::MAX,
                label.clone(),
                Vec2::new(6.0 + d, 4.0 + d),
                label_px,
                Vec2::ZERO,
                c,
            );
        }
        renderer.render(&gpu, target.target(), &frame, &list);
        let img = target.capture(&gpu)?;
        blit(
            &mut sheet,
            &img,
            (i as u32 % cols) * cell.0,
            (i as u32 / cols) * cell.1,
        );
    }
    sheet.save_png(out)?;
    println!(
        "map sheet ({n} maps, {}x{} each): {}",
        cell.0,
        cell.1,
        out.display()
    );
    Ok(())
}
