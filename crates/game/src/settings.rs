//! Player settings, persisted as `settings.json` in the user data dir
//! (docs/platform.md "Settings"): the options page (ui.md 2.7), face
//! presets (ui.md 2.6), per-device profiles, match rules and online
//! preferences. Unknown or missing fields fall back to defaults, so old
//! files keep loading.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Window mode (ui.md 2.7 `OPTION_FULLSCREEN`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DisplayMode {
    /// Exclusive fullscreen at the chosen resolution.
    Exclusive,
    /// Borderless fullscreen window at the desktop resolution.
    #[default]
    FullscreenWindow,
    Maximized,
    Windowed,
}

impl DisplayMode {
    pub const ALL: [DisplayMode; 4] = [
        DisplayMode::Exclusive,
        DisplayMode::FullscreenWindow,
        DisplayMode::Maximized,
        DisplayMode::Windowed,
    ];

    /// Localisation key and English text.
    pub fn label(self) -> (&'static str, &'static str) {
        match self {
            DisplayMode::Exclusive => ("OPTION_EXCLUSIVEFULLSCREEN", "Exclusive fullscreen"),
            DisplayMode::FullscreenWindow => ("OPTION_FULLSCREENWINDOW", "Fullscreen window"),
            DisplayMode::Maximized => ("OPTION_MAXIMIZEWINDOW", "Maximized window"),
            DisplayMode::Windowed => ("OPTION_WINDOWED", "Windowed"),
        }
    }
}

/// One face (ui.md 2.6 `PlayerFace`): item index per category (into the
/// face catalog's lists) and the dragged offset of each item.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Face {
    pub eyes: u16,
    pub eyes_offset: [f32; 2],
    pub mouth: u16,
    pub mouth_offset: [f32; 2],
    pub detail: u16,
    pub detail_offset: [f32; 2],
    pub detail2: u16,
    pub detail2_offset: [f32; 2],
}

impl Face {
    /// Item index and offset of category `c` (0 eyes, 1 mouth, 2/3 details).
    pub fn item(&self, c: usize) -> (u16, [f32; 2]) {
        match c {
            0 => (self.eyes, self.eyes_offset),
            1 => (self.mouth, self.mouth_offset),
            2 => (self.detail, self.detail_offset),
            _ => (self.detail2, self.detail2_offset),
        }
    }

    pub fn set_item(&mut self, c: usize, id: u16) {
        match c {
            0 => (self.eyes, self.eyes_offset) = (id, [0.0; 2]),
            1 => (self.mouth, self.mouth_offset) = (id, [0.0; 2]),
            2 => (self.detail, self.detail_offset) = (id, [0.0; 2]),
            _ => (self.detail2, self.detail2_offset) = (id, [0.0; 2]),
        }
    }

    pub fn set_offset(&mut self, c: usize, o: [f32; 2]) {
        match c {
            0 => self.eyes_offset = o,
            1 => self.mouth_offset = o,
            2 => self.detail_offset = o,
            _ => self.detail2_offset = o,
        }
    }
}

/// Number of face presets (the original stores 10, shows 8).
pub const FACE_PRESETS: usize = 10;

/// What a device remembers between sessions (keyed by
/// `input::DeviceId::profile_key`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    /// Selected face preset.
    pub face: usize,
    /// Preferred colour (0..12).
    pub colour: u8,
}

/// Which maps a match picks from.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MapPool {
    /// Map numbers (build order) left out; empty = every map.
    pub excluded: Vec<u16>,
}

/// Rules of the next match (lobby settings).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MatchRules {
    /// 0 = free-for-all, 2..=6 = that many teams.
    pub teams: u8,
    pub points_to_win_round: u8,
    pub rounds_to_win: u8,
    pub pick_phase: bool,
    /// How a pick phase deals hands (match-flow.md 16, port decision).
    pub pick_mode: PickModeSetting,
    pub maps: MapPool,
    /// Built-in mods (docs/MODS.md), all off by default. Online the
    /// host's set travels with these rules to every peer.
    pub mods: rounds_sim::Mods,
}

/// `rounds_sim::PickMode` as a setting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PickModeSetting {
    /// Simultaneous when more than 3 players pick.
    #[default]
    Auto,
    /// The original: one picker after another.
    Sequential,
    /// Every picker at once.
    Simultaneous,
}

impl PickModeSetting {
    pub fn to_sim(self) -> rounds_sim::PickMode {
        match self {
            PickModeSetting::Auto => rounds_sim::PickMode::Auto,
            PickModeSetting::Sequential => rounds_sim::PickMode::Sequential,
            PickModeSetting::Simultaneous => rounds_sim::PickMode::Simultaneous,
        }
    }
}

impl Default for MatchRules {
    fn default() -> Self {
        MatchRules {
            teams: 0,
            points_to_win_round: 2,
            rounds_to_win: 5,
            pick_phase: true,
            pick_mode: PickModeSetting::Auto,
            maps: MapPool::default(),
            mods: rounds_sim::Mods::default(),
        }
    }
}

/// Online host preferences.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct OnlinePrefs {
    /// false = unlisted (code / invite only).
    pub public: bool,
    /// 0 open, 1 code, 2 approve.
    pub entry: u8,
}

impl Default for OnlinePrefs {
    fn default() -> Self {
        OnlinePrefs {
            public: true,
            entry: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub master_volume: f32,
    pub sfx_volume: f32,
    pub music_volume: f32,
    pub display_mode: DisplayMode,
    /// Window/fullscreen resolution; `None` = desktop / default.
    pub resolution: Option<(u32, u32)>,
    pub vsync: bool,
    /// Screen shake strength 0..1 (port option).
    pub screen_shake: f32,
    /// Locale code of the import's string tables (`en-US`, `de`, ...).
    pub language: String,
    /// Controllers aim with the left stick while the right one is centred.
    pub left_stick_aim: bool,
    /// Snap mouse aim to 16 directions (`OPTION_MOUSE_AIM8DIR`).
    pub mouse_aim_snap: bool,
    /// Snap stick aim to 16 directions (`OPTION_CONTROLLER_AIM8DIR`).
    pub controller_aim_snap: bool,
    pub show_stat_numbers: bool,
    pub faces: Vec<Face>,
    pub profiles: BTreeMap<String, Profile>,
    pub rules: MatchRules,
    pub online: OnlinePrefs,
    /// Remapped controls (Options, Controls).
    pub bindings: crate::input::Bindings,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            master_volume: 1.0,
            sfx_volume: 1.0,
            music_volume: 1.0,
            display_mode: DisplayMode::Windowed,
            resolution: None,
            vsync: true,
            screen_shake: 1.0,
            language: "en-US".into(),
            left_stick_aim: true,
            mouse_aim_snap: false,
            controller_aim_snap: false,
            show_stat_numbers: false,
            faces: Vec::new(),
            profiles: BTreeMap::new(),
            rules: MatchRules::default(),
            online: OnlinePrefs::default(),
            bindings: crate::input::Bindings::default(),
        }
    }
}

/// Path of the settings file (`None` without a data dir).
pub fn path() -> Option<PathBuf> {
    crate::paths::data_dir().map(|d| d.join("settings.json"))
}

impl Settings {
    /// Loads the settings file, or defaults (also on parse errors, which are
    /// logged). The system language is the default if the import has it.
    pub fn load(locales: &[String]) -> Settings {
        let mut s = match path().and_then(|p| std::fs::read_to_string(p).ok()) {
            Some(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                log::warn!("settings.json: {e}; using defaults");
                Settings::default()
            }),
            None => Settings {
                language: system_language(locales),
                ..Settings::default()
            },
        };
        s.normalise();
        s
    }

    /// Clamps values and fills the face presets.
    pub fn normalise(&mut self) {
        for v in [
            &mut self.master_volume,
            &mut self.sfx_volume,
            &mut self.music_volume,
            &mut self.screen_shake,
        ] {
            *v = if v.is_finite() {
                v.clamp(0.0, 1.0)
            } else {
                1.0
            };
        }
        self.faces.resize(FACE_PRESETS, Face::default());
        let r = &mut self.rules;
        r.points_to_win_round = r.points_to_win_round.clamp(1, 10);
        r.rounds_to_win = r.rounds_to_win.clamp(1, 20);
        if r.teams == 1 || r.teams > 6 {
            r.teams = 0;
        }
        self.online.entry = self.online.entry.min(2);
    }

    /// Writes `settings.json` (errors are logged, not fatal).
    pub fn save(&self) {
        let Some(p) = path() else { return };
        if std::env::var_os("ROUNDS_NO_SAVE").is_some() {
            return;
        }
        let res = (|| -> anyhow::Result<()> {
            if let Some(d) = p.parent() {
                std::fs::create_dir_all(d)?;
            }
            let tmp = p.with_extension("json.tmp");
            std::fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
            std::fs::rename(&tmp, &p)?;
            Ok(())
        })();
        if let Err(e) = res {
            log::warn!("saving {}: {e:#}", p.display());
        }
    }

    pub fn profile(&self, key: &str) -> Profile {
        self.profiles.get(key).cloned().unwrap_or_default()
    }

    /// Mixer gain of a volume slider: the original maps the slider to dB
    /// (`20 log10(s)`, -60 dB and below = silent); as a linear gain that
    /// is the slider value itself with a cut-off at 0.001.
    pub fn gain(slider: f32) -> f32 {
        if slider <= 0.001 { 0.0 } else { slider }
    }
}

/// The system language as one of the import's locales (two-letter match),
/// else `en-US` (ui.md 7).
pub fn system_language(locales: &[String]) -> String {
    let lang = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .find(|v| !v.is_empty() && v != "C" && v != "POSIX")
        .unwrap_or_default();
    let two = lang.get(..2).unwrap_or("").to_ascii_lowercase();
    if lang.to_ascii_lowercase().starts_with("pt")
        && let Some(l) = locales.iter().find(|l| l.starts_with("pt"))
    {
        return l.clone();
    }
    locales
        .iter()
        .find(|l| l.get(..2).is_some_and(|p| p.eq_ignore_ascii_case(&two)))
        .cloned()
        .unwrap_or_else(|| "en-US".into())
}

/// Language label keys per locale (ui.md 2.7).
pub fn language_label(locale: &str) -> (&'static str, &'static str) {
    match locale {
        "fr" => ("LANG_FRENCH", "Français"),
        "it" => ("LANG_ITALIAN", "Italiano"),
        "de" => ("LANG_GERMAN", "Deutsch"),
        "es" => ("LANG_SPANISH", "Español"),
        "pt-BR" => ("LANG_PORTUGUESE_BR", "Português (Brasil)"),
        "ru" => ("LANG_RUSSIAN", "Русский"),
        "ja" => ("LANG_JAPANESE", "日本語"),
        "zh" => ("LANG_CHINESE", "中文"),
        _ => ("LANG_ENGLISH", "English"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_defaults() {
        let mut s = Settings::default();
        s.normalise();
        assert_eq!(s.faces.len(), FACE_PRESETS);
        s.master_volume = 0.5;
        s.profiles
            .insert("controller:x".into(), Profile { face: 3, colour: 7 });
        let text = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&text).unwrap();
        assert_eq!(back, s);
        // Partial files keep the defaults of missing fields.
        let p: Settings = serde_json::from_str(r#"{"vsync": false}"#).unwrap();
        assert!(!p.vsync);
        assert_eq!(p.rules.rounds_to_win, 5);
        assert_eq!(p.master_volume, 1.0);
    }

    #[test]
    fn face_items() {
        let mut f = Face::default();
        f.set_offset(2, [1.0, 2.0]);
        f.set_item(2, 5);
        assert_eq!(f.item(2), (5, [0.0, 0.0]), "equipping resets the offset");
    }
}
