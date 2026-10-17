//! Particle effects of the match from the imported particle systems
//! (`crate::psys`): the player prefab's own systems (jump / land smoke,
//! block, recharge, heal, snow, out-of-bounds bursts), the death effects
//! (`PlayerDeath`, `PlayerDeath_Phoenix`), every spawned effect object whose
//! prefab has particle systems (explosions, fields, sparks, ...), and the
//! body fill of each player (`Skin_PlayerOne..Four`, player.md 18.2).
//!
//! Colours follow the original: `SetTeamColor` replaces a system's start
//! colour with the spawning player's skin colour (match-flow.md 3), the
//! death effect colours `DeathEffect.partToColor` / `partsToColor` with the
//! player's colour and points its `ROT` systems along the killing blow.

use std::collections::HashMap;
use std::rc::Rc;

use glam::{Mat4, Quat, Vec2, Vec3};
use rounds_render::{Color, DrawList, Gpu, Layer, Mask, Renderer, Sprite};
use rounds_sim::{Content, Event, EventKind as K, World};

use super::assets::{PlayAssets, Skin};
use super::player_view::v;
use crate::psys::{Effect, EffectDef, Play, look_rotation};

/// Effect definitions, loaded once per match scene.
#[derive(Default)]
pub struct EffectBank {
    /// Prefab id -> effect (prefabs with particle systems only).
    by_prefab: HashMap<String, Rc<EffectDef>>,
    /// Root name -> prefab id.
    by_name: HashMap<String, String>,
    player: Option<Rc<EffectDef>>,
    /// Death effects with the component ids of the systems to colour.
    death: Option<(Rc<EffectDef>, Vec<String>)>,
    phoenix: Option<(Rc<EffectDef>, Vec<String>)>,
    skins: [Option<Rc<EffectDef>>; 4],
    gravity: (f32, f32),
}

fn prefab_id_from_file(name: &str) -> Option<String> {
    let stem = name.strip_suffix(".json")?;
    let (file, pid) = stem.rsplit_once('_')?;
    pid.parse::<i64>().ok()?;
    Some(format!("{file}:{pid}"))
}

impl EffectBank {
    pub fn load(gpu: &Gpu, renderer: &mut Renderer, assets: &mut PlayAssets) -> EffectBank {
        let data = assets.data.clone();
        let mut bank = EffectBank {
            gravity: (
                match data.index.settings.gravity_3d[1] {
                    0.0 => -9.81,
                    g => g,
                },
                data.index.settings.gravity[1],
            ),
            ..Default::default()
        };
        let Ok(dir) = std::fs::read_dir(data.dir.join("prefabs")) else {
            return bank;
        };
        let mut files: Vec<_> = dir.flatten().map(|e| e.file_name()).collect();
        files.sort();
        let player_id = data.player.prefabs.first().cloned();
        for f in files {
            let Some(id) = f.to_str().and_then(prefab_id_from_file) else {
                continue;
            };
            let Ok(text) = std::fs::read_to_string(data.dir.join("prefabs").join(&f)) else {
                continue;
            };
            if !text.contains("\"particles\"") {
                continue;
            }
            let Ok(pf) = serde_json::from_str::<rounds_import::format::Prefab>(&text) else {
                continue;
            };
            let Some(root) = pf.objects.first() else {
                continue;
            };
            let name = root.name.clone();
            let mut tex = |t: &str| {
                assets
                    .textures
                    .get(gpu, renderer, &data, t)
                    .map(|(id, _)| id)
            };
            let def = EffectDef::from_objects(&name, &pf.objects, 0, &data, &mut tex);
            if def.systems.is_empty() {
                continue;
            }
            let def = Rc::new(def);
            let colour_parts = || -> Vec<String> {
                root.components
                    .iter()
                    .filter(|c| c.script.as_deref() == Some("DeathEffect"))
                    .flat_map(|c| {
                        let mut v: Vec<String> = Vec::new();
                        if let Some(r) = c.fields["partToColor"]["$ref"].as_str() {
                            v.push(r.to_owned());
                        }
                        for x in c.fields["partsToColor"].as_array().into_iter().flatten() {
                            if let Some(r) = x["$ref"].as_str() {
                                v.push(r.to_owned());
                            }
                        }
                        v
                    })
                    .collect()
            };
            match name.as_str() {
                "PlayerDeath" => bank.death = Some((def.clone(), colour_parts())),
                "PlayerDeath_Phoenix" => bank.phoenix = Some((def.clone(), colour_parts())),
                "Skin_PlayerOne" => bank.skins[0] = Some(def.clone()),
                "Skin_PlayerTwo" => bank.skins[1] = Some(def.clone()),
                "Skin_PlayerThree" => bank.skins[2] = Some(def.clone()),
                "Skin_PlayerFour" => bank.skins[3] = Some(def.clone()),
                _ => {}
            }
            if Some(&id) == player_id.as_ref() {
                bank.player = Some(def.clone());
            }
            bank.by_name.entry(name.clone()).or_insert(id.clone());
            bank.by_prefab.insert(id, def);
        }
        log::info!(
            "effects: {} prefabs with particle systems (player {}, death {}, skins {})",
            bank.by_prefab.len(),
            bank.player.is_some(),
            bank.death.is_some(),
            bank.skins.iter().filter(|s| s.is_some()).count()
        );
        bank
    }

    /// The prefab of a content object has particle systems.
    pub fn has(&self, prefab: &str) -> bool {
        self.by_prefab.contains_key(prefab)
    }
}

enum Follow {
    Fixed,
    Object(u32),
    Player(u8),
}

struct Run {
    fx: Effect,
    follow: Follow,
}

/// The running effects of a match.
#[derive(Default)]
pub struct Effects {
    pub bank: EffectBank,
    runs: Vec<Run>,
    /// Body fill per player.
    skins: Vec<Option<(usize, Effect)>>,
    seed: u64,
}

fn team_colors(skin: &Skin) -> [Color; 4] {
    [skin.color, skin.background, skin.particle, skin.win_text]
}

fn root(pos: Vec2, rot: Quat, scale: f32) -> Mat4 {
    Mat4::from_scale_rotation_translation(Vec3::splat(scale), rot, pos.extend(0.0))
}

impl Effects {
    pub fn new(bank: EffectBank) -> Effects {
        Effects {
            bank,
            ..Default::default()
        }
    }

    pub fn loaded(&self) -> bool {
        !self.bank.by_prefab.is_empty()
    }

    fn next_seed(&mut self) -> u64 {
        self.seed = self.seed.wrapping_add(0x9E37_79B9);
        self.seed
    }

    fn add(&mut self, fx: Effect, follow: Follow) {
        if self.runs.len() > 400 {
            self.runs.remove(0);
        }
        let g = self.bank.gravity;
        self.runs.push(Run {
            fx: fx.with_gravity(g.0, g.1),
            follow,
        });
    }

    /// Player prefab systems by name, one-shot.
    fn player_systems(
        &mut self,
        names: &[&str],
        at: Mat4,
        place: Option<Option<Vec2>>,
        skin: &Skin,
        follow: Follow,
    ) {
        let Some(def) = self.bank.player.clone() else {
            return;
        };
        let seed = self.next_seed();
        let mut fx = Effect::new(def, at, seed, Play::Named(names));
        if let Some(dir) = place {
            fx.relocate(|s| s.matches(names), dir);
        }
        fx.set_team_colors(team_colors(skin));
        self.add(fx, follow);
    }

    /// Effects for a sim event. Returns true when the event got an
    /// imported effect (the caller then skips its procedural fallback).
    pub fn on_event(
        &mut self,
        e: &Event,
        w: &World,
        content: &Content,
        assets: &PlayAssets,
    ) -> bool {
        if !self.loaded() {
            return false;
        }
        let skin_of = |p: u8| {
            w.players()
                .get(p as usize)
                .map(|pl| assets.skin(pl.skin as usize))
        };
        let player = |p: u8| w.players().get(p as usize);
        match &e.kind {
            K::Jumped {
                player: p,
                pos,
                dir,
                ..
            } => {
                let Some(skin) = skin_of(*p) else {
                    return false;
                };
                let d = v(*dir).normalize_or(Vec2::Y);
                self.player_systems(
                    &["Jump", "JumpSmoke"],
                    root(v(*pos), Quat::IDENTITY, 1.0),
                    Some(Some(d)),
                    &skin,
                    Follow::Fixed,
                );
                true
            }
            K::Landed {
                player: p,
                pos,
                normal,
                impact,
                ..
            } => {
                if *impact < 20.0 {
                    return true;
                }
                let Some(skin) = skin_of(*p) else {
                    return false;
                };
                let k = (impact / 40.0).clamp(0.5, 1.0) * 0.5;
                self.player_systems(
                    &["LandSmoke"],
                    root(v(*pos), Quat::IDENTITY, k * 2.0),
                    Some(Some(v(*normal).normalize_or(Vec2::Y))),
                    &skin,
                    Follow::Fixed,
                );
                true
            }
            K::BlockStarted { player: p, .. } | K::BlockRecharged { player: p, .. } => {
                let (Some(pl), Some(skin)) = (player(*p), skin_of(*p)) else {
                    return false;
                };
                let name = if matches!(e.kind, K::BlockStarted { .. }) {
                    "Block"
                } else {
                    "Particles/ReloadBlock"
                };
                self.player_systems(
                    &[name],
                    root(v(pl.pos), Quat::IDENTITY, pl.scale),
                    None,
                    &skin,
                    Follow::Player(*p),
                );
                matches!(e.kind, K::BlockStarted { .. })
            }
            K::Healed { player: p, amount } if *amount > 1.0 => {
                let (Some(pl), Some(skin)) = (player(*p), skin_of(*p)) else {
                    return false;
                };
                self.player_systems(
                    &["Heal"],
                    root(v(pl.pos), Quat::IDENTITY, pl.scale),
                    None,
                    &skin,
                    Follow::Player(*p),
                );
                true
            }
            K::Slowed { player: p, .. } => {
                let (Some(pl), Some(skin)) = (player(*p), skin_of(*p)) else {
                    return false;
                };
                self.player_systems(
                    &["Snow"],
                    root(v(pl.pos), Quat::IDENTITY, pl.scale),
                    None,
                    &skin,
                    Follow::Player(*p),
                );
                true
            }
            K::OutOfBounds {
                player: p,
                pos,
                inward,
                shielded,
                ..
            } => {
                let Some(skin) = skin_of(*p) else {
                    return false;
                };
                let names: &[&str] = if *shielded {
                    &[
                        "OutOfBounds/Particles/ShieldBurst",
                        "OutOfBounds/Particles/ShieldBurstBig",
                    ]
                } else {
                    &[
                        "OutOfBounds/Particles/Burst",
                        "OutOfBounds/Particles/BurstBig",
                    ]
                };
                let Some(def) = self.bank.player.clone() else {
                    return false;
                };
                let seed = self.next_seed();
                let d = v(*inward).normalize_or(Vec2::Y);
                let mut fx = Effect::new(
                    def,
                    root(v(*pos), Quat::IDENTITY, 1.0),
                    seed,
                    Play::Named(names),
                );
                fx.relocate(|s| s.matches(names), Some(d));
                fx.set_team_colors(team_colors(&skin));
                self.add(fx, Follow::Fixed);
                true
            }
            K::Died { player: p, dir, .. } | K::PhoenixDied { player: p, dir } => {
                let phoenix = matches!(e.kind, K::PhoenixDied { .. });
                let Some((def, parts)) = (if phoenix {
                    self.bank.phoenix.clone().or(self.bank.death.clone())
                } else {
                    self.bank.death.clone()
                }) else {
                    return false;
                };
                let (Some(pl), Some(skin)) = (player(*p), skin_of(*p)) else {
                    return false;
                };
                let seed = self.next_seed();
                let scale = def.root_scale.x.abs().max(1e-3) * pl.scale;
                let mut fx = Effect::new(
                    def,
                    root(v(pl.pos), Quat::IDENTITY, scale),
                    seed,
                    Play::Awake,
                );
                // DeathEffect.PlayDeath: velocity at least 30 along the blow.
                let d = v(*dir).normalize_or(Vec2::Y);
                fx.look_along(|s| s.name.contains("ROT"), d);
                fx.set_start_color(|s| parts.contains(&s.sys.component), skin.color);
                fx.set_team_colors(team_colors(&skin));
                self.add(fx, Follow::Fixed);
                true
            }
            K::ObjectSpawned {
                id,
                def,
                pos,
                dir,
                scale,
            } => {
                let od = content.object(*def);
                let Some(edef) = self.bank.by_prefab.get(&od.id).cloned() else {
                    return false;
                };
                let d = v(*dir);
                let rot = if d.length_squared() > 1e-8 {
                    look_rotation(d)
                } else {
                    Quat::IDENTITY
                };
                let seed = self.next_seed();
                let mut fx =
                    Effect::new(edef, root(v(*pos), rot, scale.max(1e-3)), seed, Play::Awake);
                let spawner = w
                    .objects()
                    .iter()
                    .find(|o| o.id == *id)
                    .and_then(|o| o.spawner);
                if let Some(skin) = spawner.and_then(skin_of) {
                    fx.set_team_colors(team_colors(&skin));
                }
                self.add(fx, Follow::Object(*id));
                true
            }
            K::Explosion { def: Some(d), .. } => self.bank.has(&content.object(*d).id),
            _ => false,
        }
    }

    /// Follows objects and players, advances every effect, keeps the skin
    /// fills in sync with the players.
    pub fn update(&mut self, w: &World, assets: &PlayAssets, real: f32, game: f32) {
        for r in &mut self.runs {
            match r.follow {
                Follow::Fixed => {}
                Follow::Object(id) => match w.objects().iter().find(|o| o.id == id && !o.dead) {
                    Some(o) => {
                        let d = v(o.dir);
                        let rot = if d.length_squared() > 1e-8 {
                            look_rotation(d)
                        } else {
                            Quat::IDENTITY
                        };
                        r.fx.root = root(v(o.pos), rot, o.scale.max(1e-3));
                    }
                    None => {
                        r.fx.stop();
                        r.follow = Follow::Fixed;
                    }
                },
                Follow::Player(p) => {
                    if let Some(pl) = w.players().get(p as usize) {
                        r.fx.root = root(v(pl.pos), Quat::IDENTITY, pl.scale);
                    }
                }
            }
            r.fx.update(real, game);
        }
        self.runs.retain(|r| r.fx.alive());

        // Skin fills.
        let n = w.players().len();
        self.skins.resize_with(n, || None);
        for (i, pl) in w.players().iter().enumerate() {
            let skin_id = pl.skin as usize;
            let want = self.bank.skins[skin_id % 4].clone();
            let Some(def) = want else {
                self.skins[i] = None;
                continue;
            };
            let stale = !matches!(&self.skins[i], Some((s, _)) if *s == skin_id);
            if stale {
                let skin = assets.skin(skin_id);
                let mut fx = Effect::new(def, Mat4::IDENTITY, 0x5EED + i as u64, Play::All);
                fx.set_start_range(skin.fill_min, skin.fill_max);
                self.skins[i] = Some((skin_id, fx));
            }
            if let Some((_, fx)) = &mut self.skins[i] {
                fx.root = root(v(pl.pos), Quat::IDENTITY, pl.scale);
                fx.update(real, game);
            }
        }
    }

    /// Damage flash on a player's body fill.
    pub fn blink(&mut self, player: usize, c: Color) {
        if let Some(Some((_, fx))) = self.skins.get_mut(player) {
            fx.recolor_alive(c);
        }
    }

    /// The body fill sprites of player `i` (drawn by the player view,
    /// clipped to its body mask), relative positions in world space.
    pub fn skin_sprites(&self, i: usize, out: &mut Vec<((usize, i32), Sprite)>) -> bool {
        match self.skins.get(i) {
            Some(Some((_, fx))) => {
                fx.sprites(out);
                true
            }
            _ => false,
        }
    }

    /// Dev preview: plays the prefab with this root name at `pos`.
    pub fn spawn_named(&mut self, name: &str, pos: Vec2, scale: f32, team: [Color; 4]) -> bool {
        let Some(def) = self
            .bank
            .by_name
            .get(name)
            .and_then(|id| self.bank.by_prefab.get(id))
            .cloned()
        else {
            return false;
        };
        let seed = self.next_seed();
        let mut fx = Effect::new(def, root(pos, Quat::IDENTITY, scale), seed, Play::Awake);
        fx.set_team_colors(team);
        self.add(fx, Follow::Fixed);
        true
    }

    /// Dev preview: advances the free-standing effects.
    pub fn update_free(&mut self, real: f32, game: f32) {
        for r in &mut self.runs {
            r.fx.update(real, game);
        }
        self.runs.retain(|r| r.fx.alive());
    }

    pub fn names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.bank.by_name.keys().cloned().collect();
        v.sort();
        v
    }

    pub fn draw(&self, list: &mut DrawList) {
        let mut out = Vec::new();
        for r in &self.runs {
            r.fx.sprites(&mut out);
        }
        for ((layer, order), s) in out {
            list.sprite(Layer::World, crate::psys::sort_value(layer, order), s);
        }
    }

    /// Draws player `i`'s body fill inside its body mask at `sort`.
    pub fn draw_skin(
        &self,
        i: usize,
        list: &mut DrawList,
        sort: i32,
        mask: u8,
        k: f32,
        pos: Vec2,
    ) -> bool {
        let mut out = Vec::new();
        if !self.skin_sprites(i, &mut out) {
            return false;
        }
        for (_, mut s) in out {
            // Spawn-in scale: shrink around the body centre.
            if k < 0.999 {
                let t = s.transform.translation;
                s.transform.translation = pos + (t - pos) * k;
                s.size *= k;
            }
            s.mask = Mask::InsideValue(mask);
            list.sprite(Layer::World, sort, s);
        }
        true
    }
}
