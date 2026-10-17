//! Sound events (audio.md): sim cue names and presentation triggers ->
//! imported clips, with the SoundContainer rules (random clip without
//! immediate repeats, volume/pitch jitter, cooldowns, polyphony per event),
//! stereo position, looping sounds keyed by emitter, and the music
//! playlist with its card/battle layers.
//!
//! Events come from the import's SoundEvent/SoundContainer bank
//! (`sounds.json`): every container layer plays (with its delay), random
//! clip, volume/pitch jitter, loop flag, event cooldown, probability and
//! polyphony. Imports without the bank fall back to clips named `X` /
//! `X_<n>` for an event `X_SE` with volumes from audio.md section 7.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::rc::Rc;
use std::sync::Arc;

use rounds_import::GameData;

use super::fx::Rng;
use crate::audio::{Audio, Bus, PlayParams, SoundData, VoiceId, decode_ogg_file};

#[derive(Clone, Debug)]
struct Layer {
    /// Ogg files (relative to the import dir).
    files: Vec<String>,
    /// Container volume (dB) incl. the event layer ratio.
    db: f32,
    /// Static pitch (semitones).
    pitch: f32,
    /// Random pitch range (semitones).
    pitch_st: f32,
    /// Random volume attenuation range (dB, <= 0).
    vol_db: f32,
    #[allow(dead_code)] // loops are started through `set_loop`
    looping: bool,
    delay: f32,
    spatial: bool,
}

#[derive(Clone, Debug)]
struct EventDef {
    layers: Vec<Layer>,
    /// Minimum real seconds between plays.
    cooldown: f32,
    /// Percent chance to play.
    probability: f32,
    ui: bool,
    polyphony: usize,
}

/// Container volume by event name (audio.md 7; default -6 dB).
fn event_db(n: &str) -> f32 {
    let table: &[(&str, f32)] = &[
        ("UI_Arms_Race_A", -20.0),
        ("UI_Arms_Race_B", -13.0),
        ("UI_Arms_Race_C", -19.0),
        ("UI_Button_Click", -12.0),
        ("UI_Button_Hover", -18.0),
        ("UI_Card_Pick", -24.0),
        ("UI_Card_Switch", -18.0),
        ("UI_Cards_Appear", -28.0),
        ("UI_Level_Transition", -22.0),
        ("UI_Match_Found", -6.0),
        ("UI_Player_Added", -12.0),
        ("UI_Player_Ball", -20.0),
        ("UI_Text_", -23.0),
        ("UI_Win_Round", -23.0),
        ("SFX_Character_Block_Start", -6.0),
        ("SFX_Character_Block", -3.0),
        ("SFX_Character_Bounce", -3.0),
        ("SFX_Character_Cant_Shoot", -9.0),
        ("SFX_Character_Die", -10.0),
        ("SFX_Character_Effect_Damage_Screen_Edge", -8.0),
        ("SFX_Character_Effect_Ensnare", -12.0),
        ("SFX_Character_Effect_Slow_Freeze", -18.0),
        ("SFX_Character_Effect_Stun_Loop", -12.0),
        ("SFX_Character_Jump_Ensnare", -9.0),
        ("SFX_Character_Jump_Ground_Big", -6.0),
        ("SFX_Character_Jump_Ground", -8.0),
        ("SFX_Character_Reload_Complete", -24.0),
        ("SFX_Character_Reload_Start", -19.0),
        ("SFX_Character_Spawn", -9.0),
        ("SFX_Character_Step_Big", -4.0),
        ("SFX_Environment_Box_Destruction", -9.0),
        ("SFX_Environment_Platform", -18.0),
        ("SFX_Environment_Rope", -9.0),
        ("SFX_Environment_Saw_Damage", -9.0),
        ("SFX_Environment_Saw_Loop", -12.0),
        ("SFX_Environment_Smasher_Impact", -12.0),
        ("SFX_Environment_Smasher_Move", -15.0),
        ("SFX_Impact_Bullet_On_Bullet", -3.0),
        ("SFX_Impact_Character_Explosive", 0.0),
        ("SFX_Impact_Damage_To_Explosion_Medium", -21.0),
        ("SFX_Impact_Environment_Ensnare", -3.0),
        ("SFX_Impact_Environment_Explosive", -1.5),
        ("_Shotgun_Auto", -9.0),
        ("SFX_Shot_", -11.0),
        ("SFX_Block_Bombs_Away_Bomb_Impact", -3.0),
        ("SFX_Block_Black_Hole_Spawn", -3.0),
        ("SFX_Block_Bombs_Away", -9.0),
        ("SFX_Block_EMP_Bullet", -9.0),
        ("SFX_Block_Empower_Shoot", -5.0),
        ("SFX_Block_Empower_Spawn", -10.0),
        ("SFX_Block_Frost_Slam", -4.0),
        ("SFX_Block_Overpower", -11.0),
        ("SFX_Block_Phoenix_Charge", 0.0),
        ("SFX_Block_Tactical", -8.0),
        ("SFX_Block_Abyssal_Countdown_Charge", -8.0),
        ("SFX_Bullet_Effect_Grow", -12.0),
        ("SFX_Bullet_Effect_Remote", -10.0),
        ("SFX_Bullet_Effect_Sneaky", -14.0),
        ("SFX_Bullet_Effect_Thruster", -12.0),
        ("SFX_Bullet_Effect_Toxic", -9.0),
        ("SFX_Passive_Pristine", -8.0),
        ("SFX_Passive_Radiance", -9.0),
        ("SFX_Passive_Taste_Of_Blood_Activate", -12.0),
        ("AMB_Generic", -21.0),
        ("MUS_", -40.0),
    ];
    table
        .iter()
        .find(|(k, _)| n.contains(k))
        .map_or(-6.0, |(_, d)| *d)
}

/// Container pitch randomisation (semitones) by name (default 0.25).
fn event_pitch(n: &str) -> f32 {
    if n.starts_with("MUS_") || n.starts_with("UI_") && !n.contains("Text") {
        0.0
    } else if n.contains("Box_Impact") || n.contains("Damage_To_Explosion_Medium") {
        2.0
    } else if n.contains("Platform_Move") {
        4.0
    } else if n.contains("Grow_Explosion") {
        6.0
    } else if n.contains("Demonic_Pact_Single") {
        1.8
    } else if n.contains("Jump_Ground") && !n.contains("Big") {
        0.5
    } else {
        0.25
    }
}

/// Real-time cooldown by name (audio.md 5, 7).
fn event_cooldown(n: &str) -> f32 {
    if n.starts_with("SFX_Impact_") || n.contains("Bomb_Impact") || n.contains("Bomb_Stick") {
        0.03
    } else if n.contains("Cant_Shoot") || n.contains("Homing_Found") || n.contains("Target_Bounce")
    {
        0.1
    } else {
        0.0
    }
}

/// The match's sound system.
pub struct Sounds {
    data: Rc<GameData>,
    /// Off in headless runs without a WAV (nothing to hear).
    pub enabled: bool,
    clip_files: HashMap<String, String>,
    events: HashMap<String, Option<EventDef>>,
    /// Sound modifiers of each card (by card import id): (shot, impact).
    card_mods: HashMap<String, (Option<String>, Option<String>)>,
    /// Delayed layer plays: (time, file, gain, pitch, pan, ui).
    delayed: Vec<(f64, String, f32, f32, f32, bool)>,
    cache: HashMap<String, Option<Arc<SoundData>>>,
    recent: HashMap<String, VecDeque<usize>>,
    last_play: HashMap<String, f64>,
    voices: HashMap<String, VecDeque<VoiceId>>,
    loops: HashMap<u32, VoiceId>,
    rng: Rng,
    time: f64,
    /// Event names that resolved to no clips.
    pub missing: BTreeSet<String>,
    pub played: u64,
    music: Music,
}

#[derive(Default)]
struct Music {
    started: bool,
    /// Card and game layer voices of the running track(s).
    tracks: Vec<(VoiceId, VoiceId, f64)>,
    next_at: f64,
    recent: VecDeque<usize>,
    /// Smoothed intensity 0 = card, 1 = battle.
    intensity: f32,
    target: f32,
    ambience: Option<VoiceId>,
    /// Main-menu theme voice and its fade-in (0..1).
    menu: Option<(VoiceId, f32)>,
    menu_started_once: bool,
}

const TRACKS: [&str; 10] = [
    "A_Very_Good_Vintage_Indeed",
    "Catching_The_Lead",
    "Combo_Of_The_Day",
    "I_Might_Be_On_To_Something",
    "Limits_Of_The_Possible",
    "One_More_Round",
    "Push_It_Further",
    "Set_To_Slow_Fry",
    "Stacked_Deck",
    "Take_Me_On",
];

/// Overall gain on top of the container volume: the original lifts the
/// mixer by +16 dB master and +10 dB SFX/music (audio.md 2); our mixer
/// clips at 1, so the offset is smaller.
const SFX_OFFSET_DB: f32 = 8.0;
const MUSIC_GAIN: f32 = 0.35;

impl Sounds {
    pub fn new(data: Rc<GameData>, enabled: bool, seed: u64) -> Sounds {
        let clip_files = data
            .audio
            .values()
            .map(|c| (c.name.clone(), c.file.clone()))
            .collect();
        let r = |f: &serde_json::Value| f["$ref"].as_str().map(str::to_owned);
        let card_mods = data
            .cards
            .iter()
            .filter_map(|c| {
                let g = c.components.get("Gun")?;
                Some((
                    c.id.clone(),
                    (r(&g["soundShotModifier"]), r(&g["soundImpactModifier"])),
                ))
            })
            .collect();
        Sounds {
            data,
            enabled,
            clip_files,
            card_mods,
            delayed: Vec::new(),
            events: HashMap::new(),
            cache: HashMap::new(),
            recent: HashMap::new(),
            last_play: HashMap::new(),
            voices: HashMap::new(),
            loops: HashMap::new(),
            rng: Rng::new(seed ^ 0x50_0d),
            time: 0.0,
            missing: BTreeSet::new(),
            played: 0,
            music: Music::default(),
        }
    }

    fn resolve(&mut self, event: &str) -> Option<EventDef> {
        if let Some(e) = self.events.get(event) {
            return e.clone();
        }
        let def = self
            .resolve_bank(event)
            .or_else(|| self.resolve_by_name(event));
        if def.is_none() {
            self.missing.insert(event.to_owned());
        }
        self.events.insert(event.to_owned(), def.clone());
        def
    }

    /// From the imported SoundEvent bank.
    fn resolve_bank(&self, event: &str) -> Option<EventDef> {
        let bank = &self.data.sounds;
        let (_, e) = bank.event_by_name(event)?;
        if e.disabled {
            return None;
        }
        let m = &e.modifier;
        let used = |k: &str| m[format!("{k}Use")].as_i64().unwrap_or(0) != 0;
        let ev_db = if used("volume") {
            20.0 * m["volumeRatio"].as_f64().unwrap_or(1.0).max(1e-4).log10() as f32
        } else {
            0.0
        };
        let ev_pitch = if used("pitch") {
            m["pitchSemitone"].as_f64().unwrap_or(0.0) as f32
        } else {
            0.0
        };
        let ev_delay = if used("delay") {
            m["delay"].as_f64().unwrap_or(0.0) as f32
        } else {
            0.0
        };
        let layers: Vec<Layer> = e
            .layers
            .iter()
            .filter_map(|l| {
                let c = bank.containers.get(&l.container)?;
                if c.files.is_empty() {
                    return None;
                }
                Some(Layer {
                    files: c.files.clone(),
                    db: c.volume_db + 20.0 * l.volume_ratio.max(1e-4).log10() + ev_db,
                    pitch: c.pitch_semitone + ev_pitch,
                    pitch_st: c.pitch_random_semitone.unwrap_or(0.0),
                    vol_db: c.volume_random_db.unwrap_or(0.0).min(0.0),
                    looping: c.looping,
                    delay: l.delay + ev_delay,
                    spatial: c.spatial_blend > 0.0,
                })
            })
            .collect();
        if layers.is_empty() {
            return None;
        }
        let poly = if used("polyphony") {
            m["polyphony"].as_i64().unwrap_or(4).max(1) as usize
        } else {
            4
        };
        let poly = match e.poly_group.as_ref().and_then(|g| bank.poly_groups.get(g)) {
            Some(g) => poly.max(g.limit.unwrap_or(6) as usize).min(6),
            None => poly,
        };
        Some(EventDef {
            ui: !layers.iter().any(|l| l.spatial),
            layers,
            cooldown: e.cooldown,
            probability: e.probability,
            polyphony: poly,
        })
    }

    /// Fallback without the bank: clips named after the event.
    fn resolve_by_name(&self, event: &str) -> Option<EventDef> {
        let base = event.strip_suffix("_SE").unwrap_or(event);
        let mut names: Vec<&String> = self
            .clip_files
            .keys()
            .filter(|n| {
                n.as_str() == base
                    || n.strip_prefix(base)
                        .and_then(|r| r.strip_prefix('_'))
                        .is_some_and(|r| !r.is_empty() && r.chars().all(|c| c.is_ascii_digit()))
            })
            .collect();
        if names.is_empty() {
            return None;
        }
        names.sort();
        let ui = base.starts_with("UI_") || base.starts_with("MUS_");
        Some(EventDef {
            layers: vec![Layer {
                files: names.iter().map(|n| self.clip_files[*n].clone()).collect(),
                db: event_db(base),
                pitch: 0.0,
                pitch_st: event_pitch(base),
                vol_db: if base.starts_with("UI_Text") {
                    -3.0
                } else {
                    0.0
                },
                looping: base.contains("_Loop"),
                delay: 0.0,
                spatial: !ui,
            }],
            cooldown: event_cooldown(base),
            probability: 100.0,
            ui,
            polyphony: if base.contains("_Impact_") { 6 } else { 4 },
        })
    }

    fn clip(&mut self, file: &str) -> Option<Arc<SoundData>> {
        if let Some(c) = self.cache.get(file) {
            return c.clone();
        }
        let p = self.data.dir.join(file);
        let r = match decode_ogg_file(&p) {
            Ok(s) => Some(Arc::new(s)),
            Err(e) => {
                log::warn!("audio {}: {e:#}", p.display());
                None
            }
        };
        self.cache.insert(file.to_owned(), r.clone());
        r
    }

    /// A clip by asset name (music).
    fn clip_named(&mut self, name: &str) -> Option<Arc<SoundData>> {
        let f = self.clip_files.get(name)?.clone();
        self.clip(&f)
    }

    /// Random clip index with a no-repeat memory of `n / 2` picks.
    fn pick(&mut self, event: &str, n: usize) -> usize {
        if n <= 1 {
            return 0;
        }
        let mem = n / 2;
        let recent = self.recent.entry(event.to_owned()).or_default();
        let mut i = (self.rng.f() * n as f32) as usize % n;
        for _ in 0..16 {
            if !recent.contains(&i) {
                break;
            }
            i = (self.rng.f() * n as f32) as usize % n;
        }
        recent.push_back(i);
        while recent.len() > mem {
            recent.pop_front();
        }
        i
    }

    /// Plays a one-shot event at world `x` (None = 2D). `volume_db` is an
    /// extra offset (shot layering).
    pub fn play(&mut self, audio: &mut Audio, event: &str, x: Option<f32>, volume_db: f32) {
        let Some(def) = self.resolve(event) else {
            return;
        };
        if !self.enabled {
            self.played += 1;
            return;
        }
        if def.cooldown > 0.0
            && let Some(t) = self.last_play.get(event)
            && self.time - t < def.cooldown as f64
        {
            return;
        }
        if def.probability < 100.0 && self.rng.f() * 100.0 >= def.probability {
            return;
        }
        self.last_play.insert(event.to_owned(), self.time);
        for (li, l) in def.layers.iter().enumerate() {
            let key = format!("{event}#{li}");
            let i = self.pick(&key, l.files.len());
            let jitter_db = if l.vol_db < 0.0 {
                self.rng.range(l.vol_db, 0.0)
            } else {
                0.0
            };
            let offset = if def.ui {
                SFX_OFFSET_DB + 4.0
            } else {
                SFX_OFFSET_DB
            };
            let gain = 10f32
                .powf((l.db + offset + jitter_db + volume_db) / 20.0)
                .min(1.0);
            let pitch = 2f32.powf((l.pitch + self.rng.range(-l.pitch_st, l.pitch_st)) / 12.0);
            let pan = match (x, l.spatial) {
                (Some(x), true) => (x / 35.56).clamp(-1.0, 1.0) * 0.75 * 0.5,
                _ => 0.0,
            };
            if l.delay > 0.0 {
                self.delayed.push((
                    self.time + l.delay as f64,
                    l.files[i].clone(),
                    gain,
                    pitch,
                    pan,
                    def.ui,
                ));
                continue;
            }
            let Some(sound) = self.clip(&l.files[i]) else {
                continue;
            };
            let id = audio.play(
                &sound,
                PlayParams {
                    volume: gain,
                    pitch,
                    pan,
                    looping: false,
                    bus: if def.ui { Bus::Ui } else { Bus::Sfx },
                },
            );
            let q = self.voices.entry(key).or_default();
            q.push_back(id);
            while q.len() > def.polyphony {
                if let Some(old) = q.pop_front() {
                    audio.stop(old);
                }
            }
        }
        self.played += 1;
    }

    /// Shot and impact modifier events of a player's cards (audio.md 5):
    /// up to `n` distinct modifiers by priority, newest card first;
    /// `slot` = "single" / "shotgun" / "impactCharacter" / ...
    pub fn modifier_events(
        &self,
        card_ids: &[&str],
        impact: bool,
        slot: &str,
        n: usize,
    ) -> Vec<String> {
        let bank = &self.data.sounds;
        let mods = if impact {
            &bank.impact_modifiers
        } else {
            &bank.shot_modifiers
        };
        let mut list: Vec<(i64, &String)> = Vec::new();
        for id in card_ids.iter().rev() {
            let Some((s, i)) = self.card_mods.get(*id) else {
                continue;
            };
            let m = if impact { i } else { s };
            if let Some(m) = m
                && let Some(def) = mods.get(m)
                && !list.iter().any(|(_, x)| *x == m)
            {
                list.push((def.priority, m));
            }
        }
        list.sort_by_key(|x| std::cmp::Reverse(x.0));
        let mut out: Vec<String> = list
            .iter()
            .take(n)
            .filter_map(|(_, m)| mods.get(*m)?.events.get(slot))
            .filter_map(|e| bank.events.get(e).map(|e| e.name.clone()))
            .collect();
        if out.is_empty()
            && let Some(basic) = mods.values().find(|m| m.name.starts_with("Basic"))
            && let Some(e) = basic.events.get(slot).and_then(|e| bank.events.get(e))
        {
            out.push(e.name.clone());
        }
        out
    }

    /// Starts (or stops) a looping event for emitter `key`.
    pub fn set_loop(&mut self, audio: &mut Audio, key: u32, event: &str, x: f32, on: bool) {
        if !on {
            if let Some(v) = self.loops.remove(&key) {
                audio.stop(v);
            }
            return;
        }
        if self.loops.contains_key(&key) {
            return;
        }
        let Some(def) = self.resolve(event) else {
            return;
        };
        if !self.enabled {
            return;
        }
        let l = def.layers[0].clone();
        let i = self.pick(event, l.files.len());
        let Some(sound) = self.clip(&l.files[i]) else {
            return;
        };
        let gain = 10f32.powf((l.db + SFX_OFFSET_DB) / 20.0).min(1.0);
        let id = audio.play(
            &sound,
            PlayParams {
                volume: gain,
                pitch: 1.0,
                pan: (x / 35.56).clamp(-1.0, 1.0) * 0.375,
                looping: true,
                bus: Bus::Sfx,
            },
        );
        self.loops.insert(key, id);
    }

    /// Stops every loop whose key is not in `alive`.
    pub fn retain_loops(&mut self, audio: &mut Audio, alive: impl Fn(u32) -> bool) {
        let dead: Vec<u32> = self.loops.keys().copied().filter(|k| !alive(*k)).collect();
        for k in dead {
            if let Some(v) = self.loops.remove(&k) {
                audio.stop(v);
            }
        }
    }

    /// Music state: `battle` = in-game layer, else the card layer.
    pub fn music(&mut self, audio: &mut Audio, playing: bool, battle: bool) {
        if !self.enabled || !playing {
            return;
        }
        self.music.target = if battle { 1.0 } else { 0.0 };
        if !self.music.started {
            self.music.started = true;
            self.music.intensity = self.music.target;
            self.music.next_at = self.time;
            if let Some(s) = self.clip_named("AMB_Generic_Loop") {
                self.music.ambience = Some(audio.play(
                    &s,
                    PlayParams {
                        volume: 0.12,
                        looping: true,
                        bus: Bus::Music,
                        ..Default::default()
                    },
                ));
            }
        }
        if self.time >= self.music.next_at {
            // Next track: random, no repeat of the last 5.
            let mut i = (self.rng.f() * 10.0) as usize % 10;
            for _ in 0..16 {
                if !self.music.recent.contains(&i) {
                    break;
                }
                i = (self.rng.f() * 10.0) as usize % 10;
            }
            self.music.recent.push_back(i);
            while self.music.recent.len() > 5 {
                self.music.recent.pop_front();
            }
            let card = self.clip_named(&format!("MUS_Level_{}_Card", TRACKS[i]));
            let game = self.clip_named(&format!("MUS_Level_{}_Game", TRACKS[i]));
            if let (Some(c), Some(g)) = (card, game) {
                let len = c.duration().max(g.duration());
                let k = self.music.intensity;
                let play = |audio: &mut Audio, s: &Arc<SoundData>, v: f32| {
                    audio.play(
                        s,
                        PlayParams {
                            volume: v,
                            looping: false,
                            bus: Bus::Music,
                            ..Default::default()
                        },
                    )
                };
                let vc = play(audio, &c, layer(1.0 - k) * MUSIC_GAIN);
                let vg = play(audio, &g, layer(k) * MUSIC_GAIN);
                self.music.tracks.push((vc, vg, self.time + len));
                self.music.next_at = self.time + (len - 4.0).max(5.0);
            } else {
                self.music.next_at = self.time + 3600.0;
            }
        }
    }

    /// Main-menu theme (`MUS_Main_Menu_Theme_SE`, audio.md 3): a looped
    /// clip at the music base volume; faded in the first time, stopped when
    /// a match starts (`on` = false).
    pub fn menu_music(&mut self, audio: &mut Audio, on: bool) {
        if !self.enabled {
            return;
        }
        match (on, self.music.menu) {
            (true, None) => {
                let Some(s) = self.clip_named("MUS_Main_Menu_Theme") else {
                    return;
                };
                let fade = if self.music.menu_started_once {
                    1.0
                } else {
                    0.0
                };
                self.music.menu_started_once = true;
                let id = audio.play(
                    &s,
                    PlayParams {
                        volume: MUSIC_GAIN * fade,
                        looping: true,
                        bus: Bus::Music,
                        ..Default::default()
                    },
                );
                self.music.menu = Some((id, fade));
            }
            (false, Some((id, _))) => {
                audio.stop(id);
                self.music.menu = None;
            }
            _ => {}
        }
    }

    /// Advances real time by `dt` (cooldowns, music crossfade).
    pub fn update(&mut self, audio: &mut Audio, dt: f32) {
        self.time += dt as f64;
        let now = self.time;
        let due: Vec<_> = self
            .delayed
            .iter()
            .filter(|d| d.0 <= now)
            .cloned()
            .collect();
        self.delayed.retain(|d| d.0 > now);
        for (_, file, volume, pitch, pan, ui) in due {
            if let Some(s) = self.clip(&file) {
                audio.play(
                    &s,
                    PlayParams {
                        volume,
                        pitch,
                        pan,
                        looping: false,
                        bus: if ui { Bus::Ui } else { Bus::Sfx },
                    },
                );
            }
        }
        let m = &mut self.music;
        if let Some((id, fade)) = &mut m.menu
            && *fade < 1.0
        {
            // First start: 3 s fade-in.
            *fade = (*fade + dt / 3.0).min(1.0);
            audio.set_volume(*id, MUSIC_GAIN * *fade);
        }
        if !m.started {
            return;
        }
        // Intensity smoothing 0.3 s.
        let k = 1.0 - (-dt / 0.3).exp();
        let old = m.intensity;
        m.intensity += (m.target - m.intensity) * k;
        if (m.intensity - old).abs() > 1e-4 {
            for (c, g, _) in &m.tracks {
                audio.set_volume(*c, layer(1.0 - m.intensity) * MUSIC_GAIN);
                audio.set_volume(*g, layer(m.intensity) * MUSIC_GAIN);
            }
        }
        let now = self.time;
        m.tracks.retain(|t| t.2 > now);
    }
}

/// Layer volume of the music crossfade (rolloff curves, audio.md 3).
fn layer(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volumes_and_sets() {
        assert_eq!(event_db("SFX_Shot_Basic_Single"), -11.0);
        assert_eq!(event_db("SFX_Character_Block_Start"), -6.0);
        assert_eq!(event_db("SFX_Character_Block_Blocked"), -3.0);
    }
}
