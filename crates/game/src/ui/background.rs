//! The main menu backdrop (ui.md 2.3): the `ArtHandler.menuArt` particle
//! systems ("fire clouds") under the light camera's fog and the menu art's
//! grade, drawn like a map without map pieces.

use glam::Vec2;
use rounds_import::GameData;
use rounds_render::{
    BackgroundSettings, Camera2D, Color, DrawList, FrameSettings, Gpu, Layer, Light,
    LightingSettings, PostSettings, Renderer, Sprite, TextureId,
};

use crate::art::{ArtSystem, MainLook};
use crate::map_scene::{TARGET_ASPECT, TextureCache};
use crate::map_view::SortKey;

pub struct MenuBackground {
    art: Vec<ArtSystem>,
    cookies: Vec<Option<TextureId>>,
    /// Use one of the map arts when the menu art has no systems.
    art_index: Option<usize>,
}

impl MenuBackground {
    pub fn new(
        gpu: &Gpu,
        renderer: &mut Renderer,
        textures: &mut TextureCache,
        data: &GameData,
        look: &MainLook,
    ) -> MenuBackground {
        let mut tex = |id: &str| {
            textures
                .get(gpu, renderer, data, id)
                .map_or(TextureId::WHITE, |t| t.0)
        };
        let mut art_index = None;
        let mut art = look.start_art(data, &look.menu_art, 0x4d454e55, &mut tex);
        if art.is_empty() && !look.arts.is_empty() {
            art_index = Some(0);
            art = look.start_art(data, &look.arts[0], 0x4d454e55, &mut tex);
        }
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
        MenuBackground {
            art,
            cookies,
            art_index,
        }
    }

    pub fn update(&mut self, dt: f32) {
        for s in &mut self.art {
            s.system.update(dt);
        }
    }

    /// Fills the frame settings and draws the particles; `dim` darkens the
    /// result (sub pages, overlays).
    pub fn draw(
        &self,
        look: &MainLook,
        frame: &mut FrameSettings,
        list: &mut DrawList,
        size: (u32, u32),
    ) {
        let aspect = size.0 as f32 / size.1.max(1) as f32;
        let ortho = look.ortho_size;
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
        let profile = match self.art_index {
            Some(i) => look.arts.get(i).map(|a| &a.profile),
            None => Some(&look.menu_art.profile),
        };
        frame.background = Some(BackgroundSettings {
            clear: look.background_clear.with_alpha(1.0),
            fog: Some(sf.fog),
            grading: profile.and_then(|p| p.grading),
        });
        frame.post = PostSettings {
            grading: look.main_profile.grading,
            unity_vignette: look.main_profile.vignette,
            bloom: look.main_profile.bloom,
            ..Default::default()
        };
        for (l, cookie) in look.lights.iter().zip(&self.cookies) {
            list.light(Light {
                position: l.position,
                color: l.color,
                intensity: l.intensity,
                range: l.size.length() * 0.5,
                source_radius: l.radius,
                casts_shadows: false,
                cookie: *cookie,
                cookie_size: l.size,
            });
        }
        let mut bg: Vec<(SortKey, Sprite)> = Vec::new();
        let mut world: Vec<(SortKey, Sprite)> = Vec::new();
        let mut tmp = Vec::new();
        let mut seq = 0u32;
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
        for (items, layer) in [(&mut bg, Layer::Background), (&mut world, Layer::World)] {
            items.sort_by(|a, b| a.0.cmp(&b.0));
            for (i, (_, s)) in items.drain(..).enumerate() {
                list.sprite(layer, i as i32, s);
            }
        }
    }
}
