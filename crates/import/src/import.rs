//! The import run: game data dir → import dir (see `docs/formats/import.md`).

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use rayon::prelude::*;
use serde::Serialize;
use serde_json::Value as J;
use unity_assets::classes::{self, AudioClip, Sprite, Texture2D};
use unity_assets::{AssetDir, AssetFile, texture};

use crate::format::{self, FORMAT_REVISION, FORMAT_VERSION, ObjId};
use crate::scene::{Ctx, obj_id, ref_id, script_fields, to_json};

/// How far prefab references are followed from cards and the player
/// (4 reaches the default bullet's pooled collision prefab: player -> gun ->
/// bullet -> collision).
const PREFAB_DEPTH: usize = 4;

/// Import options.
#[derive(Clone, Debug)]
pub struct Options {
    /// Game directory (with `*_Data`) or data directory.
    pub game_dir: PathBuf,
    /// Output directory; replaced atomically.
    pub out_dir: PathBuf,
    /// Re-import even if the output is up to date.
    pub force: bool,
}

/// What an import did.
#[derive(Clone, Debug, Default)]
pub struct Report {
    /// Output was already up to date; nothing was written.
    pub up_to_date: bool,
    /// Objects written per kind.
    pub counts: BTreeMap<String, usize>,
    /// Non-fatal problems.
    pub warnings: Vec<String>,
    /// Wall time.
    pub seconds: f64,
}

struct Run<'a> {
    ctx: Ctx<'a>,
    dir: &'a AssetDir,
    out: PathBuf,
    warnings: Mutex<Vec<String>>,
    /// SoundEvent and sound modifier ids (targets of `sound_refs`).
    sound_targets: BTreeSet<ObjId>,
    /// Component fields referencing sound assets (`SoundBank::references`).
    sound_refs: Mutex<BTreeMap<String, BTreeSet<ObjId>>>,
}

impl Run<'_> {
    fn warn(&self, msg: String) {
        log::warn!("{msg}");
        self.warnings.lock().unwrap().push(msg);
    }

    /// Records the sound references of a flattened hierarchy.
    fn note_sound_refs(&self, objects: &[format::Object]) {
        if self.sound_targets.is_empty() {
            return;
        }
        let mut local = BTreeMap::new();
        for o in objects {
            for c in &o.components {
                let script = c.script.as_deref().unwrap_or(&c.kind);
                crate::assets::collect_references(
                    script,
                    &c.fields,
                    &self.sound_targets,
                    &mut local,
                );
            }
        }
        if local.is_empty() {
            return;
        }
        let mut g = self.sound_refs.lock().unwrap();
        for (k, v) in local {
            g.entry(k).or_default().extend(v);
        }
    }
}

fn write_json<T: Serialize>(path: &Path, v: &T, pretty: bool) -> Result<()> {
    if let Some(p) = path.parent() {
        fs::create_dir_all(p)?;
    }
    let mut w = BufWriter::new(fs::File::create(path).with_context(|| path.display().to_string())?);
    if pretty {
        serde_json::to_writer_pretty(&mut w, v)?;
    } else {
        serde_json::to_writer(&mut w, v)?;
    }
    w.write_all(b"\n")?;
    w.flush()?;
    Ok(())
}

fn write_png(path: &Path, w: u32, h: u32, rgba: &[u8]) -> Result<()> {
    let f = BufWriter::new(fs::File::create(path)?);
    let mut e = png::Encoder::new(f, w, h);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.set_compression(png::Compression::Fast);
    e.write_header()?.write_image_data(rgba)?;
    Ok(())
}

/// Fingerprint of the data dir: names, sizes and modification times.
pub fn fingerprint(dir: &AssetDir) -> Result<String> {
    let mut entries: Vec<(String, u64, u64)> = Vec::new();
    for e in fs::read_dir(dir.data_dir())? {
        let e = e?;
        let m = e.metadata()?;
        if m.is_file() {
            let mtime = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs());
            entries.push((e.file_name().to_string_lossy().into_owned(), m.len(), mtime));
        }
    }
    entries.sort();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for (n, l, t) in &entries {
        for b in format!("{n}:{l}:{t};").bytes() {
            h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
        }
    }
    Ok(format!("{h:016x}-{}", entries.len()))
}

/// Runs an import.
pub fn import(opts: &Options) -> Result<Report> {
    let t0 = Instant::now();
    let dir = AssetDir::open(&opts.game_dir)?;
    let fp = fingerprint(&dir)?;
    let index_path = opts.out_dir.join("index.json");
    if !opts.force
        && let Ok(s) = fs::read_to_string(&index_path)
        && let Ok(idx) = serde_json::from_str::<format::Index>(&s)
        && idx.format_version == FORMAT_VERSION
        && idx.format_revision >= FORMAT_REVISION
        && idx.source_fingerprint == fp
        && crate::load::LATER_FILES
            .iter()
            .all(|f| opts.out_dir.join(f).is_file())
    {
        return Ok(Report {
            up_to_date: true,
            counts: idx.counts,
            ..Default::default()
        });
    }
    if opts.out_dir.exists()
        && !index_path.exists()
        && fs::read_dir(&opts.out_dir)?.next().is_some()
    {
        bail!(
            "{} exists, is not empty and is not an import dir; refusing to replace it",
            opts.out_dir.display()
        );
    }
    let tmp = opts.out_dir.with_extension("tmp");
    if tmp.exists() {
        fs::remove_dir_all(&tmp)?;
    }
    for sub in ["textures", "audio", "maps", "prefabs"] {
        fs::create_dir_all(tmp.join(sub))?;
    }
    let mut run = Run {
        ctx: Ctx::new(&dir)?,
        dir: &dir,
        out: tmp.clone(),
        warnings: Default::default(),
        sound_targets: BTreeSet::new(),
        sound_refs: Default::default(),
    };
    let files: Vec<_> = dir
        .serialized_file_names()?
        .iter()
        .map(|n| dir.file(n))
        .collect::<unity_assets::Result<_>>()?;
    let assets: Vec<_> = files
        .iter()
        .filter(|f| !f.name.starts_with("level"))
        .cloned()
        .collect();
    let mut counts = BTreeMap::new();

    let textures = run.textures(&files)?;
    counts.insert("textures".into(), textures.len());
    write_json(&tmp.join("textures.json"), &textures, false)?;

    let sprites = run.sprites(&files, &textures)?;
    counts.insert("sprites".into(), sprites.len());
    write_json(&tmp.join("sprites.json"), &sprites, false)?;

    let audio = run.audio(&files)?;
    counts.insert("audio".into(), audio.len());
    write_json(&tmp.join("audio.json"), &audio, false)?;

    let mut sounds = run.sounds(&assets, &audio)?;
    counts.insert("sound_events".into(), sounds.events.len());
    counts.insert("sound_containers".into(), sounds.containers.len());
    run.sound_targets = sounds
        .events
        .keys()
        .chain(sounds.shot_modifiers.keys())
        .chain(sounds.impact_modifiers.keys())
        .cloned()
        .collect();

    let categories = run.categories(&assets)?;
    counts.insert("card_categories".into(), categories.len());
    write_json(&tmp.join("categories.json"), &categories, true)?;

    let (localization, loc_warnings) = crate::assets::localization(dir.data_dir());
    for w in loc_warnings {
        run.warn(format!("localization: {w}"));
    }
    counts.insert("locales".into(), localization.locales.len());
    write_json(&tmp.join("localization.json"), &localization, false)?;

    let (scenes, maps) = run.maps()?;
    counts.insert("maps".into(), maps);

    let (cards, mut queue) = run.cards(&assets, &categories, &localization)?;
    counts.insert("cards".into(), cards.len());
    write_json(&tmp.join("cards.json"), &cards, false)?;

    let player = run.player(&assets)?;
    queue.extend(player.prefabs.iter().cloned());
    queue.extend(player.skin_prefabs.values().cloned());
    write_json(&tmp.join("player.json"), &player, false)?;

    let main = run.main_scene()?;
    let (faces, face_roots) = run.faces(&assets, &main)?;
    queue.extend(face_roots);
    counts.insert(
        "face_items".into(),
        faces.eyes.len() + faces.mouths.len() + faces.accessories.len(),
    );
    write_json(&tmp.join("faces.json"), &faces, false)?;

    let prefabs = run.prefab_closure(queue, &cards)?;
    counts.insert("prefabs".into(), prefabs);

    counts.insert("main_objects".into(), main.objects.len());
    write_json(&tmp.join("main.json"), &main, false)?;

    let materials = run.materials(&files)?;
    counts.insert("materials".into(), materials.len());
    write_json(&tmp.join("materials.json"), &materials, false)?;

    let (animations, anim_warnings) = crate::anim::animations(&dir, &files);
    for w in anim_warnings {
        run.warn(format!("animation: {w}"));
    }
    counts.insert("animation_clips".into(), animations.clips.len());
    counts.insert("animator_controllers".into(), animations.controllers.len());
    write_json(&tmp.join("animations.json"), &animations, false)?;

    let profiles = run.post_profiles(&assets)?;
    counts.insert("post_profiles".into(), profiles.len());
    write_json(&tmp.join("post_profiles.json"), &profiles, false)?;

    let fonts = run.fonts(&assets)?;
    counts.insert("fonts".into(), fonts.len());
    write_json(&tmp.join("fonts.json"), &fonts, false)?;

    sounds.references = std::mem::take(&mut *run.sound_refs.lock().unwrap())
        .into_iter()
        .map(|(k, v)| (k, v.into_iter().collect()))
        .collect();
    write_json(&tmp.join("sounds.json"), &sounds, false)?;

    let ggm = dir.file("globalgamemanagers")?;
    let tm = ggm
        .objects_of(78)
        .next()
        .map(|o| {
            dir.read(&ggm, o.path_id)
                .map(|v| classes::TagManager::from_value(&v))
        })
        .transpose()?
        .unwrap_or_default();
    let settings = run.settings(&ggm)?;
    let index = format::Index {
        format_version: FORMAT_VERSION,
        format_revision: FORMAT_REVISION,
        unity_version: ggm.meta.unity_version.clone(),
        source_fingerprint: fp,
        scenes,
        sorting_layers: tm
            .sorting_layers
            .iter()
            .map(|l| format::SortingLayer {
                name: l.name.clone(),
                id: l.unique_id as u32,
            })
            .collect(),
        layers: tm.layers.clone(),
        tags: {
            let mut t: Vec<String> = [
                "Untagged",
                "Respawn",
                "Finish",
                "EditorOnly",
                "MainCamera",
                "Player",
                "GameController",
            ]
            .map(String::from)
            .to_vec();
            t.extend(tm.tags.iter().cloned());
            t
        },
        settings,
        player_prefabs: player.prefabs.clone(),
        counts: counts.clone(),
    };
    write_json(&tmp.join("index.json"), &index, true)?;

    if opts.out_dir.exists() {
        fs::remove_dir_all(&opts.out_dir)?;
    }
    fs::rename(&tmp, &opts.out_dir)?;
    Ok(Report {
        up_to_date: false,
        counts,
        warnings: run.warnings.into_inner().unwrap(),
        seconds: t0.elapsed().as_secs_f64(),
    })
}

impl Run<'_> {
    fn textures(
        &self,
        files: &[std::sync::Arc<AssetFile>],
    ) -> Result<BTreeMap<ObjId, format::Texture>> {
        let items: Vec<_> = files
            .iter()
            .flat_map(|f| f.objects_of(28).map(move |o| (f.clone(), o.path_id)))
            .collect();
        let out: Vec<_> = items
            .par_iter()
            .filter_map(|(f, pid)| {
                let id = obj_id(f, *pid);
                match self.texture(f, *pid, &id) {
                    Ok(t) => t.map(|t| (id, t)),
                    Err(e) => {
                        self.warn(format!("texture {id}: {e:#}"));
                        None
                    }
                }
            })
            .collect();
        Ok(out.into_iter().collect())
    }

    fn texture(&self, f: &AssetFile, pid: i64, id: &str) -> Result<Option<format::Texture>> {
        let t = Texture2D::from_value(&self.dir.read(f, pid)?);
        if t.width == 0 || t.height == 0 {
            return Ok(None);
        }
        let stream;
        let data: &[u8] = if t.image_data.is_empty() {
            stream = self
                .dir
                .stream(&t.stream.path, t.stream.offset, t.stream.size)?;
            &stream
        } else {
            &t.image_data
        };
        let rgba = texture::decode(t.format, t.width, t.height, data)?;
        let png = format!("textures/{}.png", format::sanitize(id));
        write_png(&self.out.join(&png), t.width, t.height, &rgba)?;
        Ok(Some(format::Texture {
            name: t.name,
            png,
            width: t.width,
            height: t.height,
            format: texture::format_name(t.format).to_owned(),
            filter_mode: t.filter_mode,
            wrap_mode: t.wrap_u,
        }))
    }

    fn sprites(
        &self,
        files: &[std::sync::Arc<AssetFile>],
        textures: &BTreeMap<ObjId, format::Texture>,
    ) -> Result<BTreeMap<ObjId, format::Sprite>> {
        let mut out = BTreeMap::new();
        for f in files {
            for o in f.objects_of(213) {
                let id = obj_id(f, o.path_id);
                let s = match self.dir.read(f, o.path_id) {
                    Ok(v) => Sprite::from_value(&v),
                    Err(e) => {
                        self.warn(format!("sprite {id}: {e}"));
                        continue;
                    }
                };
                let tex = ref_id(f, s.texture);
                let th = tex
                    .as_ref()
                    .and_then(|t| textures.get(t))
                    .map_or(0.0, |t| t.height as f32);
                let [x, y, w, h] = s.texture_rect;
                let ppu = if s.pixels_to_units > 0.0 {
                    s.pixels_to_units
                } else {
                    100.0
                };
                out.insert(
                    id,
                    format::Sprite {
                        name: s.name.clone(),
                        texture: tex,
                        rect: s.rect,
                        texture_rect: [x, th - (y + h), w, h],
                        texture_rect_offset: s.texture_rect_offset,
                        pivot: s.pivot,
                        pixels_per_unit: ppu,
                        border: s.border,
                        packed: s.packed(),
                        packing_rotation: s.packing_rotation(),
                        physics_shape: s
                            .physics_shape
                            .iter()
                            .map(|p| p.iter().map(|q| [q[0] / ppu, q[1] / ppu]).collect())
                            .collect(),
                    },
                );
            }
        }
        Ok(out)
    }

    fn audio(
        &self,
        files: &[std::sync::Arc<AssetFile>],
    ) -> Result<BTreeMap<ObjId, format::AudioClip>> {
        // Names first (sequential, deterministic de-duplication), then convert.
        let mut used = HashSet::new();
        let mut items = Vec::new();
        for f in files {
            for o in f.objects_of(83) {
                let id = obj_id(f, o.path_id);
                let clip = match self.dir.read(f, o.path_id) {
                    Ok(v) => AudioClip::from_value(&v),
                    Err(e) => {
                        self.warn(format!("audio {id}: {e}"));
                        continue;
                    }
                };
                let base = format::sanitize(&clip.name);
                let mut name = base.clone();
                let mut n = 1;
                while !used.insert(name.to_lowercase()) {
                    n += 1;
                    name = format!("{base}_{n}");
                }
                items.push((id, clip, format!("audio/{name}.ogg")));
            }
        }
        let out: Vec<_> = items
            .into_par_iter()
            .filter_map(|(id, clip, file)| {
                let r = &clip.resource;
                let res = self
                    .dir
                    .stream(&r.path, r.offset, r.size)
                    .map_err(anyhow::Error::from)
                    .and_then(|bank| Ok(unity_assets::audio::fsb5_to_ogg(&bank)?))
                    .and_then(|(ogg, samples)| {
                        fs::write(self.out.join(&file), ogg)?;
                        Ok(samples)
                    });
                match res {
                    Ok(samples) => Some((
                        id,
                        format::AudioClip {
                            name: clip.name,
                            file,
                            channels: clip.channels,
                            frequency: clip.frequency,
                            length: clip.length,
                            samples,
                        },
                    )),
                    Err(e) => {
                        self.warn(format!("audio {id} ({}): {e:#}", clip.name));
                        None
                    }
                }
            })
            .collect();
        Ok(out.into_iter().collect())
    }

    fn maps(&self) -> Result<(Vec<format::SceneEntry>, usize)> {
        let ggm = self.dir.file("globalgamemanagers")?;
        let bs = ggm.objects_of(141).next().context("no BuildSettings")?;
        let scenes = classes::build_scenes(&self.dir.read(&ggm, bs.path_id)?);
        let entries: Vec<format::SceneEntry> = scenes
            .iter()
            .enumerate()
            .map(|(i, path)| {
                let name = path
                    .rsplit('/')
                    .next()
                    .unwrap_or(path)
                    .trim_end_matches(".unity")
                    .to_owned();
                format::SceneEntry {
                    build_index: i,
                    map_file: (i >= 2)
                        .then(|| format!("maps/{i:02}_{}.json", format::sanitize(&name))),
                    name,
                    path: path.clone(),
                }
            })
            .collect();
        let results: Vec<Result<()>> = entries
            .par_iter()
            .filter(|e| e.map_file.is_some())
            .map(|e| {
                let map = self.map(e).with_context(|| format!("map {}", e.name))?;
                write_json(&self.out.join(e.map_file.as_ref().unwrap()), &map, false)
            })
            .collect();
        let mut n = 0;
        for r in results {
            match r {
                Ok(()) => n += 1,
                Err(e) => self.warn(format!("{e:#}")),
            }
        }
        Ok((entries, n))
    }

    /// Root GameObjects of a scene file, in file order.
    fn scene_roots(&self, f: &AssetFile) -> Result<Vec<i64>> {
        let mut roots = Vec::new();
        for o in f
            .meta
            .objects
            .iter()
            .filter(|o| o.class_id == 4 || o.class_id == 224)
        {
            let t = classes::Transform::from_value(&self.dir.read(f, o.path_id)?);
            if t.father.is_null() && !t.game_object.is_null() {
                roots.push(t.game_object.path_id);
            }
        }
        Ok(roots)
    }

    /// The main scene (`level0`), with raw particle system fields.
    fn main_scene(&self) -> Result<format::Scene> {
        let f = self.dir.file("level0")?;
        let roots = self.scene_roots(&f)?;
        let objects = self.ctx.flatten_opts(&f, &roots, true)?;
        self.note_sound_refs(&objects);
        Ok(format::Scene {
            build_index: 0,
            name: "Main".into(),
            objects,
        })
    }

    /// Every Material of every file.
    fn materials(
        &self,
        files: &[std::sync::Arc<AssetFile>],
    ) -> Result<BTreeMap<ObjId, format::Material>> {
        let mut shaders: BTreeMap<ObjId, String> = BTreeMap::new();
        let mut out = BTreeMap::new();
        for f in files {
            for o in f.objects_of(21) {
                let id = obj_id(f, o.path_id);
                let v = match self.dir.read(f, o.path_id) {
                    Ok(v) => v,
                    Err(e) => {
                        self.warn(format!("material {id}: {e}"));
                        continue;
                    }
                };
                let shader = match v.get("m_Shader").as_pptr() {
                    Some(p) if !p.is_null() => {
                        let sid = ref_id(f, p).unwrap_or_default();
                        if let Some(n) = shaders.get(&sid) {
                            n.clone()
                        } else {
                            let n = match self.dir.deref(f, p) {
                                Ok(Some((_, _, sv))) => {
                                    sv.path("m_ParsedForm.m_Name").str().to_owned()
                                }
                                _ => String::new(),
                            };
                            shaders.insert(sid, n.clone());
                            n
                        }
                    }
                    _ => String::new(),
                };
                let props = v.get("m_SavedProperties");
                let pairs = |name: &str| -> Vec<(String, &unity_assets::Value)> {
                    props
                        .get(name)
                        .as_array()
                        .iter()
                        .map(|p| (p.get("first").str().to_owned(), p.get("second")))
                        .collect()
                };
                out.insert(
                    id,
                    format::Material {
                        name: v.get("m_Name").str().to_owned(),
                        shader,
                        render_queue: v.get("m_CustomRenderQueue").i64() as i32,
                        keywords: v
                            .get("m_ValidKeywords")
                            .as_array()
                            .iter()
                            .map(|k| k.str().to_owned())
                            .collect(),
                        textures: pairs("m_TexEnvs")
                            .into_iter()
                            .map(|(k, t)| {
                                (
                                    k,
                                    format::TexEnv {
                                        texture: t
                                            .get("m_Texture")
                                            .as_pptr()
                                            .and_then(|p| ref_id(f, p)),
                                        scale: t.get("m_Scale").vec2(),
                                        offset: t.get("m_Offset").vec2(),
                                    },
                                )
                            })
                            .collect(),
                        floats: pairs("m_Floats")
                            .into_iter()
                            .map(|(k, x)| (k, x.f32()))
                            .collect(),
                        colors: pairs("m_Colors")
                            .into_iter()
                            .map(|(k, c)| (k, c.color()))
                            .collect(),
                    },
                );
            }
        }
        Ok(out)
    }

    /// Post-processing profiles with their effect settings.
    fn post_profiles(
        &self,
        assets: &[std::sync::Arc<AssetFile>],
    ) -> Result<BTreeMap<ObjId, format::PostProfile>> {
        let mut out = BTreeMap::new();
        for (f, pid) in self.scripts_of(assets, "PostProcessProfile") {
            let v = self.dir.read(&f, pid)?;
            let mut effects = BTreeMap::new();
            for s in v.get("settings").as_array() {
                let Some(p) = s.as_pptr().filter(|p| !p.is_null()) else {
                    continue;
                };
                let Some((ef, epid)) = self.dir.resolve(&f, p)? else {
                    continue;
                };
                let Some(info) = ef.object(epid) else {
                    continue;
                };
                let read = self.dir.read_object(&ef, info)?;
                let class = read
                    .script
                    .as_ref()
                    .map(|k| k.class.clone())
                    .unwrap_or_default();
                effects.insert(class, script_fields(&ef, &read.value));
            }
            out.insert(
                obj_id(&f, pid),
                format::PostProfile {
                    name: v.get("m_Name").str().to_owned(),
                    effects,
                },
            );
        }
        Ok(out)
    }

    fn map(&self, e: &format::SceneEntry) -> Result<format::Map> {
        let f = self.dir.file(&format!("level{}", e.build_index))?;
        let mut roots = Vec::new();
        for o in f
            .meta
            .objects
            .iter()
            .filter(|o| o.class_id == 4 || o.class_id == 224)
        {
            let t = classes::Transform::from_value(&self.dir.read(&f, o.path_id)?);
            if t.father.is_null() && !t.game_object.is_null() {
                roots.push(t.game_object.path_id);
            }
        }
        let objects = self.ctx.flatten(&f, &roots)?;
        self.note_sound_refs(&objects);
        let mut map = None;
        let mut spawn_points = Vec::new();
        for (i, o) in objects.iter().enumerate() {
            for c in &o.components {
                match c.script.as_deref() {
                    Some("Map") if map.is_none() => map = Some(c.fields.clone()),
                    Some("SpawnPoint") => spawn_points.push(format::SpawnPoint {
                        object: i,
                        id: c.fields["ID"].as_i64().unwrap_or(0),
                        team: c.fields["TEAMID"].as_i64().unwrap_or(0),
                        position: [o.world.trs.position[0], o.world.trs.position[1]],
                    }),
                    _ => {}
                }
            }
        }
        Ok(format::Map {
            build_index: e.build_index,
            name: e.name.clone(),
            map,
            spawn_points,
            objects,
        })
    }

    /// Script MonoBehaviours of a class across files: (file, path id, GameObject id).
    fn scripts_of(
        &self,
        files: &[std::sync::Arc<AssetFile>],
        class: &str,
    ) -> Vec<(std::sync::Arc<AssetFile>, i64)> {
        let mut out = Vec::new();
        for f in files {
            for o in f.objects_of(114) {
                if self.dir.script_of(f, o).is_some_and(|k| k.class == class) {
                    out.push((f.clone(), o.path_id));
                }
            }
        }
        out
    }

    fn cards(
        &self,
        assets: &[std::sync::Arc<AssetFile>],
        categories: &BTreeMap<ObjId, String>,
        loc: &format::Localization,
    ) -> Result<(Vec<format::Card>, BTreeSet<ObjId>)> {
        let mut cards = Vec::new();
        let mut refs = BTreeSet::new();
        for (f, pid) in self.scripts_of(assets, "CardInfo") {
            let info = self.dir.read(&f, pid)?;
            let Some(go) = info.get("m_GameObject").as_pptr().filter(|p| !p.is_null()) else {
                continue;
            };
            let objects = self.ctx.flatten(&f, &[go.path_id])?;
            self.note_sound_refs(&objects);
            let root = &objects[0];
            let mut components = BTreeMap::new();
            for c in &root.components {
                let key = c.script.clone().unwrap_or_else(|| c.kind.clone());
                components.entry(key).or_insert_with(|| c.fields.clone());
            }
            let ci = script_fields(&f, &info);
            let art = ci["cardArt"]["$ref"].as_str().map(String::from);
            let mut referenced = self.prefab_refs(&objects);
            if let Some(a) = &art {
                referenced.insert(a.clone());
            }
            refs.extend(referenced.iter().cloned());
            let id = obj_id(&f, go.path_id);
            let cat_names = |key: &str| -> Vec<String> {
                ci[key]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|e| e["$ref"].as_str())
                            .map(|r| match categories.get(r) {
                                Some(n) => n.clone(),
                                None => {
                                    self.warn(format!("card {id}: category {r} not found"));
                                    r.to_owned()
                                }
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            };
            let key = |k: &str| crate::assets::localized_key(loc, &ci[k]);
            let stat_keys: Vec<Option<String>> = ci["cardStats"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|st| crate::assets::localized_key(loc, &st["m_localizedStat"]))
                        .collect()
                })
                .unwrap_or_default();
            write_json(
                &self.out.join(format::prefab_file(&id)),
                &format::Prefab {
                    id: id.clone(),
                    name: root.name.clone(),
                    objects: objects.clone(),
                },
                false,
            )?;
            cards.push(format::Card {
                prefab: format::prefab_file(&id),
                object_name: root.name.clone(),
                name: ci["cardName"].as_str().unwrap_or("").to_owned(),
                description: ci["cardDestription"].as_str().unwrap_or("").to_owned(),
                rarity: ci["rarity"].as_i64().unwrap_or(0),
                color_theme: ci["colorTheme"].as_i64().unwrap_or(0),
                art,
                components,
                referenced_prefabs: referenced.into_iter().collect(),
                categories: cat_names("categories"),
                blacklisted_categories: cat_names("blacklistedCategories"),
                name_key: key("m_localizedCardName"),
                description_key: key("m_localizedCardDescription"),
                stat_keys: if loc.tables.is_empty() {
                    vec![]
                } else {
                    stat_keys
                },
                id,
            });
        }
        cards.sort_by(|a, b| a.id.cmp(&b.id));
        Ok((cards, refs))
    }

    /// GameObjects referenced from the components of `objects` but outside them.
    fn prefab_refs(&self, objects: &[format::Object]) -> BTreeSet<ObjId> {
        let own: HashSet<&str> = objects
            .iter()
            .flat_map(|o| {
                std::iter::once(o.id.as_str()).chain(o.components.iter().map(|c| c.id.as_str()))
            })
            .collect();
        let mut ids = BTreeSet::new();
        for o in objects {
            for c in &o.components {
                collect_refs(&c.fields, &mut ids);
            }
        }
        let mut out = BTreeSet::new();
        for id in ids {
            if own.contains(id.as_str()) {
                continue;
            }
            if let Some(go) = self.game_object_of(&id)
                && !own.contains(go.as_str())
            {
                out.insert(go);
            }
        }
        out
    }

    /// The GameObject an ID denotes (itself, or a component's owner), if any.
    fn game_object_of(&self, id: &str) -> Option<ObjId> {
        let (file, pid) = id.rsplit_once(':')?;
        if file.starts_with("level") {
            return None;
        }
        let f = self.dir.file(file).ok()?;
        let pid: i64 = pid.parse().ok()?;
        let info = f.object(pid)?;
        match info.class_id {
            1 => Some(id.to_owned()),
            // Assets that are not part of a GameObject hierarchy.
            21 | 28 | 43 | 48 | 49 | 74 | 83 | 91 | 115 | 128 | 213 => None,
            _ => {
                let v = self.dir.read_object(&f, info).ok()?.value;
                let go = v.get("m_GameObject").as_pptr().filter(|p| !p.is_null())?;
                ref_id(&f, go)
            }
        }
    }

    fn prefab_closure(&self, start: BTreeSet<ObjId>, cards: &[format::Card]) -> Result<usize> {
        let mut done: HashSet<ObjId> = cards.iter().map(|c| c.id.clone()).collect();
        let mut frontier: Vec<ObjId> = start.into_iter().filter(|id| !done.contains(id)).collect();
        let mut written = done.len();
        for _ in 0..PREFAB_DEPTH {
            frontier.sort();
            frontier.dedup();
            frontier.retain(|id| done.insert(id.clone()));
            if frontier.is_empty() {
                break;
            }
            let next: Vec<BTreeSet<ObjId>> = frontier
                .par_iter()
                .map(|id| match self.prefab(id) {
                    Ok(refs) => refs,
                    Err(e) => {
                        self.warn(format!("prefab {id}: {e:#}"));
                        BTreeSet::new()
                    }
                })
                .collect();
            written += frontier.len();
            frontier = next
                .into_iter()
                .flatten()
                .filter(|id| !done.contains(id))
                .collect();
        }
        Ok(written)
    }

    fn prefab(&self, id: &str) -> Result<BTreeSet<ObjId>> {
        let (file, pid) = id.rsplit_once(':').context("bad id")?;
        let f = self.dir.file(file)?;
        let objects = self.ctx.flatten(&f, &[pid.parse()?])?;
        self.note_sound_refs(&objects);
        let refs = self.prefab_refs(&objects);
        let p = format::Prefab {
            id: id.to_owned(),
            name: objects[0].name.clone(),
            objects,
        };
        write_json(&self.out.join(format::prefab_file(id)), &p, false)?;
        Ok(refs)
    }

    /// The character creator catalog (`faces.json`) and the item
    /// GameObjects to export as prefabs.
    fn faces(
        &self,
        assets: &[std::sync::Arc<AssetFile>],
        main: &format::Scene,
    ) -> Result<(format::Faces, BTreeSet<ObjId>)> {
        let mut faces = format::Faces::default();
        let mut roots = BTreeSet::new();
        let refs = |fields: &J, key: &str| -> Vec<ObjId> {
            fields[key]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|r| r["$ref"].as_str().map(str::to_owned))
                .collect()
        };
        let comp = |script: &str| {
            main.objects
                .iter()
                .flat_map(|o| &o.components)
                .find(|c| c.script.as_deref() == Some(script))
        };
        if let Some(l) = comp("CharacterCreatorItemLoader") {
            let mut load = |key: &str, out: &mut Vec<format::FaceItem>| {
                for r in refs(&l.fields, key) {
                    match self.face_item(&r) {
                        Some(it) => {
                            roots.insert(it.prefab.clone());
                            out.push(it);
                        }
                        None => self.warn(format!("faces: {key} entry {r} is not a CharacterItem")),
                    }
                }
            };
            load("eyes", &mut faces.eyes);
            load("mouths", &mut faces.mouths);
            load("accessories", &mut faces.accessories);
        } else {
            self.warn("faces: no CharacterCreatorItemLoader in the main scene".into());
        }
        let num = |v: &J| v.as_i64().unwrap_or(0);
        let vec2 = |v: &J| {
            [
                v["x"].as_f64().unwrap_or(0.0) as f32,
                v["y"].as_f64().unwrap_or(0.0) as f32,
            ]
        };
        let preset = |f: &J| format::FacePreset {
            eye_id: num(&f["eyeID"]),
            eye_offset: vec2(&f["eyeOffset"]),
            mouth_id: num(&f["mouthID"]),
            mouth_offset: vec2(&f["mouthOffset"]),
            detail_id: num(&f["detailID"]),
            detail_offset: vec2(&f["detailOffset"]),
            detail2_id: num(&f["detail2ID"]),
            detail2_offset: vec2(&f["detail2Offset"]),
        };
        if let Some(h) = comp("CharacterCreatorHandler") {
            let list = |k: &str| -> Vec<format::FacePreset> {
                h.fields[k]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(preset)
                    .collect()
            };
            faces.presets = list("playerFaces");
            faces.selected_presets = list("selectedPlayerFaces");
            let ints = |k: &str| -> Vec<i64> {
                h.fields[k]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(num)
                    .collect()
            };
            faces.selected_face_id = ints("selectedFaceID");
            faces.locked_portraits = ints("lockedPortraits");
        }
        // The player prefab's equipper names the default eyes and mouth.
        if let Some((f, pid)) = self
            .scripts_of(assets, "CharacterCreatorItemEquipper")
            .into_iter()
            .next()
        {
            let v = self.dir.read(&f, pid)?;
            let item = |name: &str| {
                v.get(name)
                    .as_pptr()
                    .and_then(|p| ref_id(&f, p))
                    .and_then(|r| self.face_item(&r))
            };
            if let Some(it) = item("defaultEyes") {
                roots.insert(it.prefab.clone());
                faces.default_eyes = Some(it.id);
            }
            if let Some(it) = item("defaultMouth") {
                roots.insert(it.prefab.clone());
                faces.default_mouth = Some(it.id);
            }
            faces.scale_m = Some(v.get("scaleM").f32());
        }
        Ok((faces, roots))
    }

    /// The `CharacterItem` that `id` (the component, or its GameObject)
    /// denotes.
    fn face_item(&self, id: &str) -> Option<format::FaceItem> {
        let go = self.game_object_of(id)?;
        let (file, pid) = go.rsplit_once(':')?;
        let f = self.dir.file(file).ok()?;
        let objects = self.ctx.flatten(&f, &[pid.parse().ok()?]).ok()?;
        let root = objects.first()?;
        let c = root
            .components
            .iter()
            .find(|c| c.script.as_deref() == Some("CharacterItem"))?;
        let fl = &c.fields;
        let n = |v: &J, d: f32| v.as_f64().map_or(d, |x| x as f32);
        let sr = root.sprite.as_ref();
        Some(format::FaceItem {
            id: c.id.clone(),
            prefab: root.id.clone(),
            name: root.name.clone(),
            item_type: fl["itemType"].as_i64().unwrap_or(0) as i32,
            sprite: fl["sprite"]["$ref"]
                .as_str()
                .map(str::to_owned)
                .or_else(|| sr.and_then(|s| s.sprite.clone())),
            offset: [n(&fl["offset"]["x"], 0.0), n(&fl["offset"]["y"], 0.0)],
            scale: n(&fl["scale"], 1.0),
            color: sr.map_or([0.1509, 0.1509, 0.1509, 1.0], |s| s.color),
            move_health_bar_up: n(&fl["moveHealthBarUp"], 0.0),
            fields: fl.clone(),
        })
    }

    fn player(&self, assets: &[std::sync::Arc<AssetFile>]) -> Result<format::Player> {
        let mut prefabs = Vec::new();
        for (f, pid) in self.scripts_of(assets, "CharacterData") {
            let v = self.dir.read(&f, pid)?;
            if let Some(go) = v.get("m_GameObject").as_pptr().and_then(|p| ref_id(&f, p)) {
                prefabs.push(go);
            }
        }
        let mut skins = BTreeMap::new();
        let mut skin_prefabs = BTreeMap::new();
        for class in ["PlayerSkinBank", "PlayerSkin"] {
            for (f, pid) in self.scripts_of(assets, class) {
                let v = self.dir.read(&f, pid)?;
                if class == "PlayerSkin"
                    && let Some(go) = v.get("m_GameObject").as_pptr().and_then(|p| ref_id(&f, p))
                {
                    skin_prefabs.insert(obj_id(&f, pid), go);
                }
                let mut j = script_fields(&f, &v);
                if let J::Object(m) = &mut j {
                    m.insert("$name".into(), J::String(v.get("m_Name").str().to_owned()));
                    m.insert("$script".into(), J::String(class.into()));
                }
                skins.insert(obj_id(&f, pid), j);
            }
        }
        prefabs.sort();
        Ok(format::Player {
            prefabs,
            skins,
            skin_prefabs,
        })
    }

    /// Sound events, containers, modifiers, poly groups and mixes.
    fn sounds(
        &self,
        assets: &[std::sync::Arc<AssetFile>],
        audio: &BTreeMap<ObjId, format::AudioClip>,
    ) -> Result<format::SoundBank> {
        use crate::assets as a;
        let mut bank = format::SoundBank::default();
        for class in [
            "SoundEvent",
            "SoundContainer",
            "SoundShotModifier",
            "SoundImpactModifier",
            "SoundPolyGroup",
            "SoundMix",
        ] {
            for (f, pid) in self.scripts_of(assets, class) {
                let v = match self.dir.read(&f, pid) {
                    Ok(v) => v,
                    Err(e) => {
                        self.warn(format!("{class} {}: {e}", obj_id(&f, pid)));
                        continue;
                    }
                };
                let name = v.get("m_Name").str().to_owned();
                let j = script_fields(&f, &v);
                let id = obj_id(&f, pid);
                match class {
                    "SoundEvent" => {
                        bank.events.insert(id, a::sound_event(&name, &j));
                    }
                    "SoundContainer" => {
                        bank.containers
                            .insert(id, a::sound_container(&name, &j, audio));
                    }
                    "SoundShotModifier" => {
                        bank.shot_modifiers.insert(id, a::sound_modifier(&name, &j));
                    }
                    "SoundImpactModifier" => {
                        bank.impact_modifiers
                            .insert(id, a::sound_modifier(&name, &j));
                    }
                    "SoundPolyGroup" => {
                        bank.poly_groups.insert(id, a::poly_group(&name, &j));
                    }
                    _ => {
                        bank.mixes.insert(id, a::sound_mix(&name, &j));
                    }
                }
            }
        }
        Ok(bank)
    }

    /// `CardCategory` assets: id -> name.
    fn categories(&self, assets: &[std::sync::Arc<AssetFile>]) -> Result<BTreeMap<ObjId, String>> {
        let mut out = BTreeMap::new();
        for (f, pid) in self.scripts_of(assets, "CardCategory") {
            let v = self.dir.read(&f, pid)?;
            out.insert(obj_id(&f, pid), v.get("m_Name").str().to_owned());
        }
        Ok(out)
    }

    fn fonts(&self, assets: &[std::sync::Arc<AssetFile>]) -> Result<Vec<format::Font>> {
        let mut fonts = Vec::new();
        for (f, pid) in self.scripts_of(assets, "TMP_FontAsset") {
            let v = self.dir.read(&f, pid)?;
            let atlases = v
                .get("m_AtlasTextures")
                .as_array()
                .iter()
                .filter_map(|p| p.as_pptr().and_then(|p| ref_id(&f, p)))
                .collect();
            let name = v.get("m_Name").str().to_owned();
            let mut fields = script_fields(&f, &v);
            let (info, suspicious) = crate::fonts::font_info(&mut fields);
            if !suspicious.is_empty() {
                self.warn(format!(
                    "font {name}: glyph rects outside the atlas for {}",
                    suspicious.join(", ")
                ));
            }
            fonts.push(format::Font {
                id: obj_id(&f, pid),
                name,
                atlases,
                fields,
                info: Some(info),
            });
        }
        Ok(fonts)
    }

    fn settings(&self, ggm: &AssetFile) -> Result<format::ProjectSettings> {
        let mut s = format::ProjectSettings::default();
        if let Some(o) = ggm.objects_of(5).next() {
            let v = self.dir.read(ggm, o.path_id)?;
            s.fixed_timestep = v.get("Fixed Timestep").f32();
            s.time = to_json(ggm, &v);
        }
        if let Some(o) = ggm.objects_of(55).next() {
            let v = self.dir.read(ggm, o.path_id)?;
            s.gravity_3d = v.get("m_Gravity").vec3();
        }
        if let Some(o) = ggm.objects_of(19).next() {
            let v = self.dir.read(ggm, o.path_id)?;
            s.gravity = v.get("m_Gravity").vec2();
            s.physics2d = to_json(ggm, &v);
        }
        Ok(s)
    }
}

/// Collects all `{"$ref": id}` targets in a JSON tree.
pub fn collect_refs(j: &J, out: &mut BTreeSet<ObjId>) {
    match j {
        J::Object(m) => {
            if let Some(J::String(id)) = m.get("$ref") {
                out.insert(id.clone());
                return;
            }
            m.values().for_each(|v| collect_refs(v, out));
        }
        J::Array(a) => a.iter().for_each(|v| collect_refs(v, out)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refs_are_collected() {
        let j = serde_json::json!({"a": {"$ref": "x:1"}, "b": [{"$ref": "y:2"}, null, {"c": {"$ref": "x:1"}}]});
        let mut s = BTreeSet::new();
        collect_refs(&j, &mut s);
        assert_eq!(s.into_iter().collect::<Vec<_>>(), ["x:1", "y:2"]);
    }
}
