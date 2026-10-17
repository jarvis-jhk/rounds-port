//! The original's look, read from the main scene (`main.json`): camera,
//! `SFRenderer` + `SFLight` (lights, fog), the post-processing profiles of
//! the two cameras, and `ArtHandler`'s nine "arts" (a background grading
//! profile plus background and foreground particle systems). See
//! `docs/spec/maps.md` sections 10 and 11.

use std::collections::BTreeMap;
use std::rc::Rc;

use glam::Vec2;
use rounds_import::GameData;
use rounds_import::format::{Object, PostProfile, Scene};
use rounds_render::{Color, ColorGrading, Fog, TextureId, Tonemapper, Vignette};
use serde_json::Value;

use crate::psys::{Effect, EffectDef, Play};

fn num(v: &Value) -> f32 {
    v.as_f64().unwrap_or(0.0) as f32
}

fn color4(v: &Value) -> [f32; 4] {
    [num(&v["r"]), num(&v["g"]), num(&v["b"]), num(&v["a"])]
}

fn rgba(c: [f32; 4]) -> Color {
    Color::rgba(c[0], c[1], c[2], c[3])
}

fn ref_id(v: &Value) -> Option<&str> {
    v.get("$ref").and_then(Value::as_str)
}

/// A post-processing profile reduced to what the renderer implements.
#[derive(Clone, Debug, Default)]
pub struct Profile {
    pub name: String,
    pub grading: Option<ColorGrading>,
    pub vignette: Option<Vignette>,
    /// Post-processing v2 bloom, when the profile has it on.
    pub bloom: Option<rounds_render::Bloom>,
}

/// A profile parameter: the stored value when overridden, else `None`
/// (Unity then uses the effect's default).
fn param<'a>(effect: &'a Value, name: &str) -> Option<&'a Value> {
    let p = effect.get(name)?;
    (p.get("overrideState").and_then(Value::as_i64) == Some(1)).then(|| &p["value"])
}

fn effect_on(effect: &Value) -> bool {
    effect.get("active").and_then(Value::as_i64) == Some(1)
        && param(effect, "enabled")
            .is_some_and(|v| v.as_i64() == Some(1) || v.as_bool() == Some(true))
}

impl Profile {
    pub fn from_import(p: &PostProfile) -> Profile {
        let mut out = Profile {
            name: p.name.clone(),
            ..Default::default()
        };
        if let Some(e) = p.effects.get("ColorGrading").filter(|e| effect_on(e)) {
            let f = |k: &str, d: f32| param(e, k).map_or(d, num);
            let v4 = |k: &str| {
                param(e, k).map_or([1.0, 1.0, 1.0, 0.0], |v| {
                    [num(&v["x"]), num(&v["y"]), num(&v["z"]), num(&v["w"])]
                })
            };
            let mixer =
                |a: &str, b: &str, c: &str, d: [f32; 3]| [f(a, d[0]), f(b, d[1]), f(c, d[2])];
            out.grading = Some(ColorGrading {
                tonemapper: match param(e, "tonemapper").and_then(Value::as_i64) {
                    Some(2) => Tonemapper::Aces,
                    _ => Tonemapper::None,
                },
                post_exposure: f("postExposure", 0.0),
                contrast: f("contrast", 0.0),
                saturation: f("saturation", 0.0),
                hue_shift: f("hueShift", 0.0),
                color_filter: param(e, "colorFilter").map_or(Color::WHITE, |v| rgba(color4(v))),
                mixer_red: mixer(
                    "mixerRedOutRedIn",
                    "mixerRedOutGreenIn",
                    "mixerRedOutBlueIn",
                    [100.0, 0.0, 0.0],
                ),
                mixer_green: mixer(
                    "mixerGreenOutRedIn",
                    "mixerGreenOutGreenIn",
                    "mixerGreenOutBlueIn",
                    [0.0, 100.0, 0.0],
                ),
                mixer_blue: mixer(
                    "mixerBlueOutRedIn",
                    "mixerBlueOutGreenIn",
                    "mixerBlueOutBlueIn",
                    [0.0, 0.0, 100.0],
                ),
                lift: v4("lift"),
                gamma: v4("gamma"),
                gain: v4("gain"),
            });
        }
        if let Some(e) = p.effects.get("Vignette").filter(|e| effect_on(e)) {
            let f = |k: &str, d: f32| param(e, k).map_or(d, num);
            out.vignette = Some(Vignette {
                color: param(e, "color").map_or(Color::BLACK, |v| rgba(color4(v))),
                center: param(e, "center")
                    .map_or(Vec2::splat(0.5), |v| Vec2::new(num(&v["x"]), num(&v["y"]))),
                intensity: f("intensity", 0.0),
                smoothness: f("smoothness", 0.2),
                roundness: f("roundness", 1.0),
                rounded: param(e, "rounded").and_then(Value::as_i64) == Some(1),
            });
        }
        if let Some(e) = p.effects.get("Bloom").filter(|e| effect_on(e)) {
            let f = |k: &str, d: f32| param(e, k).map_or(d, num);
            let color = param(e, "color")
                .map(|c| Color::rgba(num(&c["r"]), num(&c["g"]), num(&c["b"]), 1.0))
                .unwrap_or(Color::WHITE);
            out.bloom = Some(rounds_render::Bloom {
                intensity: f("intensity", 0.0),
                threshold: f("threshold", 1.0),
                soft_knee: f("softKnee", 0.5),
                diffusion: f("diffusion", 7.0),
                color,
                clamp: f("clamp", 65472.0),
            });
        }
        out
    }
}

/// `SFRenderer` settings plus the fog derived from them.
#[derive(Clone, Debug)]
pub struct SfSettings {
    pub ambient: Color,
    pub exposure: f32,
    pub light_map_scale: u32,
    pub shadow_map_scale: u32,
    pub shadows: bool,
    pub fog: Fog,
}

impl Default for SfSettings {
    fn default() -> Self {
        // The shipped values (maps.md 10.2), used if the scene lacks them.
        SfSettings {
            ambient: Color::rgb(0.6226, 0.6226, 0.6226),
            exposure: 2.88,
            light_map_scale: 8,
            shadow_map_scale: 1,
            shadows: true,
            fog: Fog {
                color: Color::rgba(0.2264, 0.2264, 0.2264, 0.14),
                scatter: Color::rgb(0.181, 0.181, 0.181),
                soft_hard_mix: 0.827,
                exposure: 2.88,
            },
        }
    }
}

/// An `SFLight`.
#[derive(Clone, Debug)]
pub struct SfLight {
    pub position: Vec2,
    pub color: Color,
    pub intensity: f32,
    /// Light source radius (penumbra softness).
    pub radius: f32,
    /// World size of the light's rect (cookie extent).
    pub size: Vec2,
    pub cookie: Option<String>,
}

/// One art: background grading profile plus its particle systems.
#[derive(Clone, Debug, Default)]
pub struct Art {
    pub profile: Profile,
    /// Indices into the main scene's objects of the particle systems that
    /// play while this art is on.
    pub systems: Vec<usize>,
}

/// Everything about the look that comes from the main scene.
#[derive(Clone, Debug)]
pub struct MainLook {
    pub ortho_size: f32,
    /// Light camera clear colour (alpha forced to 1).
    pub background_clear: Color,
    pub sf: SfSettings,
    pub lights: Vec<SfLight>,
    /// Main camera profile ("Default").
    pub main_profile: Profile,
    pub arts: Vec<Art>,
    /// The main menu's art (not used by maps).
    #[allow(dead_code)]
    pub menu_art: Art,
    scene: Scene,
}

fn comp<'a>(o: &'a Object, script_or_type: &str) -> Option<&'a Value> {
    o.components
        .iter()
        .find(|c| {
            c.kind == script_or_type
                || c.script.as_deref().is_some_and(|s| {
                    // Namespace-qualified script names
                    // (`UnityEngine.Rendering.PostProcessing.PostProcessVolume`).
                    s == script_or_type
                        || s.strip_suffix(script_or_type)
                            .is_some_and(|ns| ns.ends_with('.'))
                })
        })
        .map(|c| &c.fields)
}

impl MainLook {
    pub fn from_data(data: &GameData) -> MainLook {
        let scene = data.main.clone().unwrap_or_default();
        let objs = &scene.objects;
        let by_comp: BTreeMap<&str, usize> = objs
            .iter()
            .enumerate()
            .flat_map(|(i, o)| o.components.iter().map(move |c| (c.id.as_str(), i)))
            .collect();
        let profile_of = |id: Option<&str>| {
            id.and_then(|id| data.post_profiles.get(id))
                .map(Profile::from_import)
                .unwrap_or_default()
        };

        // Cameras: the main camera draws everything but layer 9; the light
        // camera (with SFRenderer) draws layer 9.
        let mut ortho_size = 20.0;
        let mut background_clear = Color::rgb(1.0, 0.0, 0.0);
        let mut sf = SfSettings::default();
        let mut main_volume_mask = 1u64 << 8;
        for o in objs.iter() {
            let Some(cam) = comp(o, "Camera") else {
                continue;
            };
            if let Some(r) = comp(o, "SFRenderer") {
                let c = color4(&cam["m_BackGroundColor"]);
                background_clear = Color::rgb(c[0], c[1], c[2]);
                let amb = color4(&r["_ambientLight"]);
                let fog = color4(&r["_fogColor"]);
                let sc = color4(&r["_scatterColor"]);
                let exposure = num(&r["_exposure"]);
                sf = SfSettings {
                    ambient: Color::rgb(amb[0], amb[1], amb[2]),
                    exposure,
                    light_map_scale: num(&r["_lightMapScale"]).round().max(1.0) as u32,
                    shadow_map_scale: num(&r["_shadowMapScale"]).round().max(1.0) as u32,
                    shadows: r["_shadows"].as_i64() != Some(0),
                    fog: Fog {
                        color: rgba(fog),
                        scatter: Color::rgb(sc[0], sc[1], sc[2]),
                        soft_hard_mix: num(&r["_softHardMix"]),
                        exposure,
                    },
                };
            } else {
                ortho_size = num(&cam["orthographic size"]).max(1.0);
                if let Some(pl) = comp(o, "PostProcessLayer") {
                    main_volume_mask = pl["volumeLayer"]["m_Bits"]
                        .as_u64()
                        .unwrap_or(main_volume_mask);
                }
            }
        }
        let mut main_profile = Profile::default();
        for o in objs.iter() {
            if let Some(v) = comp(o, "PostProcessVolume")
                && main_volume_mask & (1u64 << o.layer) != 0
            {
                main_profile = profile_of(ref_id(&v["sharedProfile"]));
            }
        }

        let lights = objs
            .iter()
            .filter(|o| o.active_in_hierarchy)
            .filter_map(|o| {
                let l = comp(o, "SFLight")?;
                let c = color4(&l["_color"]);
                let size = o.rect.map_or(Vec2::splat(1.0), |r| {
                    Vec2::from(r.size_delta) * Vec2::new(o.world.trs.scale[0], o.world.trs.scale[1])
                });
                Some(SfLight {
                    position: Vec2::new(o.world.trs.position[0], o.world.trs.position[1]),
                    color: Color::rgba(c[0], c[1], c[2], 1.0),
                    intensity: num(&l["_intensity"]),
                    radius: num(&l["_radius"]),
                    size,
                    cookie: ref_id(&l["_cookieTexture"]).map(String::from),
                })
            })
            .collect();

        // Arts.
        let art_of = |a: &Value| -> Art {
            let mut systems = Vec::new();
            for p in a["parts"].as_array().into_iter().flatten() {
                if let Some(&i) = ref_id(p).and_then(|id| by_comp.get(id)) {
                    collect_systems(objs, i, &mut systems);
                }
            }
            systems.sort_unstable();
            systems.dedup();
            Art {
                profile: profile_of(ref_id(&a["profile"])),
                systems,
            }
        };
        let (mut arts, mut menu_art) = (Vec::new(), Art::default());
        for o in objs.iter() {
            if let Some(h) = comp(o, "ArtHandler") {
                arts = h["arts"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(art_of)
                    .collect();
                menu_art = art_of(&h["menuArt"]);
            }
        }
        MainLook {
            ortho_size,
            background_clear,
            sf,
            lights,
            main_profile,
            arts,
            menu_art,
            scene,
        }
    }

    /// Starts the particle systems of `art` (pre-warmed). `texture` maps a
    /// texture id of the import to a renderer texture.
    pub fn start_art(
        &self,
        data: &GameData,
        art: &Art,
        seed: u64,
        texture: &mut dyn FnMut(&str) -> TextureId,
    ) -> Vec<ArtSystem> {
        let objs = &self.scene.objects;
        let mut out = Vec::new();
        for (n, &i) in art.systems.iter().enumerate() {
            let o = &objs[i];
            // Typed particle systems (import revision 2): shapes, bursts,
            // modules as authored (psys).
            let mut tex = |id: &str| Some(texture(id));
            let def = EffectDef::single(&o.name, objs, i, data, &mut tex);
            let Some(sd) = def.systems.first() else {
                continue;
            };
            let (layer, order, max_size) = (
                sd.look.layer,
                sd.look.order,
                sd.sys.main.start_size.max_value(),
            );
            let w = &o.world.trs;
            let fx = Effect::new(
                Rc::new(def),
                EffectDef::world_of(objs, i),
                seed.wrapping_add(n as u64 * 7919),
                Play::Forced,
            )
            .with_gravity(
                match data.index.settings.gravity_3d[1] {
                    0.0 => -9.81,
                    g => g,
                },
                data.index.settings.gravity[1],
            );
            out.push(ArtSystem {
                background: o.layer == 9,
                sorting_layer: layer,
                sorting_order: order,
                z: w.position[2],
                system: ArtParticles { fx, max_size },
            });
        }
        // Systems at the same depth are drawn in this order; Unity leaves
        // the order of such ties unspecified. Draw backdrops (big
        // particles, e.g. "NightSky") before detail (UNCERTAIN, maps.md 11).
        out.sort_by(|a, b| b.system.max_size.total_cmp(&a.system.max_size));
        out
    }
}

/// A part's object and its active descendants that have particle systems.
fn collect_systems(objs: &[Object], i: usize, out: &mut Vec<usize>) {
    if objs[i]
        .components
        .iter()
        .any(|c| c.kind == "ParticleSystem")
    {
        out.push(i);
    }
    for &c in &objs[i].children {
        if objs[c].active {
            collect_systems(objs, c, out);
        }
    }
}

/// A running art particle system and where it is drawn.
pub struct ArtSystem {
    /// Drawn by the background stage (layer 9 "Lighting") instead of the
    /// main camera.
    pub background: bool,
    pub sorting_layer: usize,
    pub sorting_order: i32,
    pub z: f32,
    pub system: ArtParticles,
}

/// One art particle system (a [`crate::psys::Effect`] of a single system).
pub struct ArtParticles {
    fx: Effect,
    /// Largest start size (orders systems that tie in Unity's sort).
    pub max_size: f32,
}

impl ArtParticles {
    /// Advances by `dt` seconds of presentation time.
    pub fn update(&mut self, dt: f32) {
        self.fx.update(dt, dt);
    }

    /// Billboard sprites in world space.
    pub fn sprites(&self, out: &mut Vec<rounds_render::Sprite>) {
        let mut tmp = Vec::new();
        self.fx.sprites(&mut tmp);
        out.extend(tmp.into_iter().map(|(_, s)| s));
    }

    pub fn len(&self) -> usize {
        self.fx.particle_count()
    }
}
