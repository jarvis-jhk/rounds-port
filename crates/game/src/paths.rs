//! Where things live: the user data dir and the original game's install.
//!
//! - Data dir: `$ROUNDS_PORT_DATA_DIR`, else the platform's per-user data
//!   dir (`$XDG_DATA_HOME/rounds-port` or `~/.local/share/rounds-port` on
//!   Linux, `%APPDATA%\rounds-port` on Windows, `~/Library/Application
//!   Support/rounds-port` on macOS).
//! - ROUNDS install, first match: `--game DIR`, `$ROUNDS_GAME_DIR`,
//!   `$WARPZONE_DATA_ROUNDS` (set by the Warpzone launcher), then every Steam
//!   library (`steamapps/libraryfolders.vdf`) of every Steam root found:
//!   `steamapps/common/ROUNDS`.

use std::path::{Path, PathBuf};

const APP_DIR: &str = "rounds-port";
/// Steam app id of ROUNDS.
const ROUNDS_APP_ID: &str = "1557740";

/// The user data dir (not created here).
pub fn data_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("ROUNDS_PORT_DATA_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(d));
    }
    platform_data_dir().map(|d| d.join(APP_DIR))
}

#[cfg(target_os = "windows")]
fn platform_data_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(PathBuf::from)
}

#[cfg(target_os = "macos")]
fn platform_data_dir() -> Option<PathBuf> {
    home().map(|h| h.join("Library/Application Support"))
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn platform_data_dir() -> Option<PathBuf> {
    if let Some(x) = std::env::var_os("XDG_DATA_HOME").filter(|x| !x.is_empty()) {
        return Some(PathBuf::from(x));
    }
    home().map(|h| h.join(".local/share"))
}

fn home() -> Option<PathBuf> {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var)
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

/// Whether `dir` looks like a ROUNDS install (or its `ROUNDS_Data` dir).
pub fn is_rounds_dir(dir: &Path) -> bool {
    unity_data_dir(dir).is_some()
}

/// The Unity data dir of an install: `dir` itself when it holds
/// `globalgamemanagers` and `level0`, else its `*_Data` child (any case: the
/// install may have been copied from a case-insensitive file system).
fn unity_data_dir(dir: &Path) -> Option<PathBuf> {
    if dir.join("globalgamemanagers").is_file() && dir.join("level0").is_file() {
        return Some(dir.to_path_buf());
    }
    let exact = dir.join("ROUNDS_Data");
    if exact.join("globalgamemanagers").is_file() {
        return Some(exact);
    }
    let mut subs: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.to_ascii_lowercase().ends_with("_data"))
                && p.join("globalgamemanagers").is_file()
        })
        .collect();
    subs.sort();
    subs.into_iter().next()
}

/// Why `dir` is not a usable install, for the "not found" message.
fn why_not(dir: &Path) -> &'static str {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return if dir.exists() {
            "cannot be read"
        } else {
            "does not exist"
        };
    };
    let names: Vec<String> = rd
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_ascii_lowercase())
        .collect();
    if names.is_empty() {
        "is empty"
    } else if names.iter().any(|n| n.ends_with("_data")) {
        "has ROUNDS_Data but no globalgamemanagers in it: incomplete download? \
         Let Steam finish or verify the game files"
    } else if names.iter().any(|n| n == "rounds.exe") {
        "has ROUNDS.exe but no ROUNDS_Data folder: incomplete download? \
         Let Steam finish or verify the game files"
    } else {
        "has no ROUNDS_Data folder"
    }
}

/// `~/x` → `$HOME/x` (paths from launchers and config files are not always
/// expanded by a shell).
fn expand_tilde(p: &Path) -> PathBuf {
    if let Ok(rest) = p.strip_prefix("~")
        && let Some(h) = home()
    {
        return h.join(rest);
    }
    p.to_path_buf()
}

/// The install dir Steam's manifest for ROUNDS names in `lib`
/// (`steamapps/appmanifest_<id>.acf`, `"installdir"`), if any.
fn manifest_install_dir(lib: &Path) -> Option<String> {
    let acf = lib
        .join("steamapps")
        .join(format!("appmanifest_{ROUNDS_APP_ID}.acf"));
    let text = std::fs::read_to_string(acf).ok()?;
    acf_value(&text, "installdir")
}

/// The value of the first `"key" "value"` line in a Steam `.acf`/`.vdf`.
fn acf_value(text: &str, key: &str) -> Option<String> {
    let quoted = format!("\"{key}\"");
    text.lines().find_map(|l| {
        let rest = l.trim().strip_prefix(quoted.as_str())?;
        let v = rest.trim().strip_prefix('"')?.strip_suffix('"')?;
        (!v.is_empty()).then(|| v.to_string())
    })
}

/// Where an install was found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Install {
    pub dir: PathBuf,
    /// Human-readable origin ("--game", "ROUNDS_GAME_DIR", "Steam library ...").
    pub source: String,
}

/// Finds the ROUNDS install. `cli` is `--game DIR`. On failure, returns the
/// list of places that were tried.
pub fn find_install(cli: Option<&Path>) -> Result<Install, Vec<String>> {
    let mut tried = Vec::new();
    let mut check = |dir: PathBuf, source: String| -> Option<Install> {
        let dir = expand_tilde(&dir);
        if is_rounds_dir(&dir) {
            Some(Install { dir, source })
        } else {
            tried.push(format!("{} ({source}): {}", dir.display(), why_not(&dir)));
            None
        }
    };
    if let Some(d) = cli
        && let Some(i) = check(d.to_path_buf(), "--game".into())
    {
        return Ok(i);
    }
    for var in ["ROUNDS_GAME_DIR", "WARPZONE_DATA_ROUNDS"] {
        if let Some(d) = std::env::var_os(var).filter(|d| !d.is_empty())
            && let Some(i) = check(PathBuf::from(d), var.into())
        {
            return Ok(i);
        }
    }
    for lib in steam_libraries() {
        let common = lib.join("steamapps").join("common");
        let mut names = vec!["ROUNDS".to_string()];
        if let Some(d) = manifest_install_dir(&lib).filter(|d| d != "ROUNDS") {
            names.insert(0, d);
        }
        for name in names {
            if let Some(i) = check(
                common.join(name),
                format!("Steam library {}", lib.display()),
            ) {
                return Ok(i);
            }
        }
    }
    Err(tried)
}

/// Steam installation roots that exist on this machine.
pub fn steam_roots() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        if let Some(p) = windows_registry_steam_path() {
            roots.push(p);
        }
        for var in ["ProgramFiles(x86)", "ProgramFiles"] {
            if let Some(p) = std::env::var_os(var) {
                roots.push(PathBuf::from(p).join("Steam"));
            }
        }
        roots.push(PathBuf::from(r"C:\Program Files (x86)\Steam"));
    } else if let Some(h) = home() {
        for rel in [
            ".steam/steam",
            ".steam/root",
            ".local/share/Steam",
            ".var/app/com.valvesoftware.Steam/.local/share/Steam",
            ".var/app/com.valvesoftware.Steam/data/Steam",
            "snap/steam/common/.local/share/Steam",
            "Library/Application Support/Steam",
        ] {
            roots.push(h.join(rel));
        }
    }
    let mut out: Vec<PathBuf> = Vec::new();
    for r in roots {
        if !r.join("steamapps").is_dir() {
            continue;
        }
        let canon = std::fs::canonicalize(&r).unwrap_or(r);
        if !out.contains(&canon) {
            out.push(canon);
        }
    }
    out
}

/// All Steam library folders (each holding `steamapps/`), the roots first.
/// Libraries whose manifest lists ROUNDS come before the others.
pub fn steam_libraries() -> Vec<PathBuf> {
    let mut libs: Vec<(bool, PathBuf)> = Vec::new();
    for root in steam_roots() {
        let mut push = |p: PathBuf, has: bool| {
            let canon = std::fs::canonicalize(&p).unwrap_or(p);
            if let Some(e) = libs.iter_mut().find(|(_, q)| *q == canon) {
                e.0 |= has;
            } else {
                libs.push((has, canon));
            }
        };
        push(root.clone(), false);
        let vdf = root.join("steamapps").join("libraryfolders.vdf");
        if let Ok(text) = std::fs::read_to_string(&vdf) {
            for (path, apps) in parse_library_folders(&text) {
                push(PathBuf::from(path), apps.iter().any(|a| a == ROUNDS_APP_ID));
            }
        }
    }
    libs.sort_by_key(|(has, _)| !*has);
    libs.into_iter().map(|(_, p)| p).collect()
}

#[cfg(windows)]
fn windows_registry_steam_path() -> Option<PathBuf> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    for (key, value) in [
        (r"HKCU\Software\Valve\Steam", "SteamPath"),
        (r"HKLM\SOFTWARE\WOW6432Node\Valve\Steam", "InstallPath"),
        (r"HKLM\SOFTWARE\Valve\Steam", "InstallPath"),
    ] {
        let out = std::process::Command::new("reg")
            .args(["query", key, "/v", value])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines() {
            if let Some(rest) = line.trim().strip_prefix(value) {
                let rest = rest.trim_start();
                if let Some(v) = rest.strip_prefix("REG_SZ") {
                    let p = v.trim().replace('/', "\\");
                    if !p.is_empty() {
                        return Some(PathBuf::from(p));
                    }
                }
            }
        }
    }
    None
}

#[cfg(not(windows))]
fn windows_registry_steam_path() -> Option<PathBuf> {
    None
}

/// Parses Steam's `libraryfolders.vdf`: `(path, app ids)` per library.
/// Handles both the current format (`"0" { "path" "..." "apps" {...} }`) and
/// the old one (`"1" "D:\\SteamLibrary"`).
pub fn parse_library_folders(text: &str) -> Vec<(String, Vec<String>)> {
    // Tokenise quoted strings and braces.
    let mut toks: Vec<String> = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' | '}' => toks.push(c.to_string()),
            '"' => {
                let mut s = String::new();
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => {
                            if let Some(n) = chars.next() {
                                s.push(n);
                            }
                        }
                        '"' => break,
                        _ => s.push(c),
                    }
                }
                toks.push(format!("\"{s}"));
            }
            _ => {}
        }
    }
    let unq = |t: &str| t.strip_prefix('"').map(str::to_owned);
    let mut out = Vec::new();
    // Walk: key value | key { ... }, tracking depth.
    let mut i = 0;
    let mut depth = 0;
    let mut current: Option<(String, Vec<String>)> = None;
    let mut in_apps = false;
    while i < toks.len() {
        let t = &toks[i];
        if t == "{" {
            depth += 1;
            i += 1;
            continue;
        }
        if t == "}" {
            if in_apps && depth == 3 {
                in_apps = false;
            }
            if depth == 2
                && let Some(c) = current.take()
            {
                out.push(c);
            }
            depth -= 1;
            i += 1;
            continue;
        }
        let key = unq(t).unwrap_or_default();
        let next = toks.get(i + 1).map(String::as_str);
        match next {
            Some("{") => {
                if depth == 1 {
                    current = Some((String::new(), Vec::new()));
                } else if depth == 2 && key.eq_ignore_ascii_case("apps") {
                    in_apps = true;
                }
                i += 1;
            }
            Some(v) if v.starts_with('"') => {
                let val = unq(v).unwrap_or_default();
                if depth == 1 && key.chars().all(|c| c.is_ascii_digit()) {
                    // Old format: "1" "path".
                    out.push((val, Vec::new()));
                } else if depth == 2 && key.eq_ignore_ascii_case("path") {
                    if let Some(c) = current.as_mut() {
                        c.0 = val;
                    }
                } else if depth == 3
                    && in_apps
                    && let Some(c) = current.as_mut()
                {
                    c.1.push(key);
                }
                i += 2;
            }
            _ => i += 1,
        }
    }
    out.retain(|(p, _)| !p.is_empty());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_folders_new_format() {
        let vdf = r#"
"libraryfolders"
{
	"0"
	{
		"path"		"/home/u/.local/share/Steam"
		"label"		""
		"apps"
		{
			"228980"		"493211355"
		}
	}
	"1"
	{
		"path"		"D:\\SteamLibrary"
		"apps"
		{
			"1557740"		"1189430131"
		}
	}
}
"#;
        let libs = parse_library_folders(vdf);
        assert_eq!(libs.len(), 2);
        assert_eq!(libs[0].0, "/home/u/.local/share/Steam");
        assert_eq!(libs[0].1, vec!["228980"]);
        assert_eq!(libs[1].0, r"D:\SteamLibrary");
        assert_eq!(libs[1].1, vec!["1557740"]);
    }

    #[test]
    fn library_folders_old_format() {
        let vdf = r#""LibraryFolders" { "TimeNextStatsReport" "123" "1" "E:\\Games\\Steam" }"#;
        let libs = parse_library_folders(vdf);
        assert_eq!(libs, vec![(r"E:\Games\Steam".to_string(), vec![])]);
    }

    #[test]
    fn acf_install_dir() {
        let acf =
            "\"AppState\"\n{\n\t\"appid\"\t\t\"1557740\"\n\t\"installdir\"\t\t\"ROUNDS\"\n}\n";
        assert_eq!(acf_value(acf, "installdir").as_deref(), Some("ROUNDS"));
        assert_eq!(acf_value(acf, "appid").as_deref(), Some("1557740"));
        assert_eq!(acf_value(acf, "name"), None);
    }

    #[test]
    fn install_detection_and_diagnosis() {
        let base = std::env::temp_dir().join(format!("rounds-port-paths-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let missing = base.join("missing");
        assert!(why_not(&missing).contains("does not exist"));

        // Only the exe: an unfinished download.
        let partial = base.join("partial");
        std::fs::create_dir_all(&partial).unwrap();
        std::fs::write(partial.join("ROUNDS.exe"), b"").unwrap();
        assert!(!is_rounds_dir(&partial));
        assert!(why_not(&partial).contains("no ROUNDS_Data"));

        // Data dir with another case.
        let odd = base.join("odd");
        std::fs::create_dir_all(odd.join("Rounds_Data")).unwrap();
        std::fs::write(odd.join("Rounds_Data/globalgamemanagers"), b"").unwrap();
        assert!(is_rounds_dir(&odd));
        assert_eq!(unity_data_dir(&odd), Some(odd.join("Rounds_Data")));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn tilde_expansion() {
        let p = Path::new("/abs/x");
        assert_eq!(expand_tilde(p), p);
        if let Some(h) = home() {
            assert_eq!(expand_tilde(Path::new("~/a/b")), h.join("a/b"));
        }
    }

    #[test]
    fn data_dir_env_override() {
        // Only checks the override logic, without touching the environment
        // of other tests: the variable is set by env.sh in development.
        if let Some(d) = std::env::var_os("ROUNDS_PORT_DATA_DIR") {
            assert_eq!(data_dir(), Some(PathBuf::from(d)));
        } else {
            assert!(data_dir().is_some_and(|d| d.ends_with(APP_DIR)));
        }
    }
}
