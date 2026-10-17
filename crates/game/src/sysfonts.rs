//! System fonts for scripts the import has no font for (ui.md 7: the
//! original ships per-locale TMP fonts in Addressables bundles that are
//! not imported). For Japanese and Chinese (and Korean) the game looks for a
//! CJK font installed on the system at runtime instead of bundling one:
//! fontconfig (`fc-match`) and the usual font directories on Linux,
//! `%WINDIR%\Fonts` on Windows, `/System/Library/Fonts` on macOS.

use std::path::{Path, PathBuf};

/// The script a locale needs beyond Latin/Cyrillic, if any.
pub fn cjk_lang(locale: &str) -> Option<&'static str> {
    let l = locale.to_ascii_lowercase();
    if l.starts_with("ja") {
        Some("ja")
    } else if l.starts_with("zh") {
        Some(
            if l.contains("hant") || l.contains("tw") || l.contains("hk") {
                "zh-tw"
            } else {
                "zh-cn"
            },
        )
    } else if l.starts_with("ko") {
        Some("ko")
    } else {
        None
    }
}

/// A sample character the font must have for `lang`.
pub fn probe_char(lang: &str) -> char {
    match lang {
        "ko" => '한',
        "ja" => 'あ',
        _ => '中',
    }
}

/// Candidate font files (path, face index) for `lang`, best first.
pub fn cjk_candidates(lang: &str) -> Vec<(PathBuf, u32)> {
    let mut out: Vec<(PathBuf, u32)> = Vec::new();
    // Noto Sans CJK collections: faces JP, KR, SC, TC, HK.
    let noto_index = match lang {
        "ja" => 0,
        "ko" => 1,
        "zh-tw" => 3,
        _ => 2,
    };
    if cfg!(windows) {
        let dir = std::env::var_os("WINDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
            .join("Fonts");
        let names: &[&str] = match lang {
            "ja" => &["YuGothM.ttc", "YuGothR.ttc", "meiryo.ttc", "msgothic.ttc"],
            "ko" => &["malgun.ttf", "gulim.ttc"],
            "zh-tw" => &["msjh.ttc", "mingliu.ttc", "msyh.ttc"],
            _ => &["msyh.ttc", "simhei.ttf", "simsun.ttc"],
        };
        for n in names {
            let p = dir.join(n);
            if p.exists() {
                out.push((p, 0));
            }
        }
        return out;
    }
    if cfg!(target_os = "macos") {
        for p in [
            "/System/Library/Fonts/Hiragino Sans GB.ttc",
            "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc",
            "/System/Library/Fonts/PingFang.ttc",
            "/System/Library/Fonts/AppleSDGothicNeo.ttc",
        ] {
            if Path::new(p).exists() {
                out.push((PathBuf::from(p), 0));
            }
        }
        return out;
    }
    // Linux / BSD: fontconfig first (it knows the user's configuration).
    let pattern = format!(
        "sans:lang={}",
        match lang {
            "zh-cn" => "zh-cn",
            "zh-tw" => "zh-tw",
            l => l,
        }
    );
    if let Ok(o) = std::process::Command::new("fc-match")
        .args(["-f", "%{file}\\n%{index}\\n", &pattern])
        .output()
        && o.status.success()
    {
        let s = String::from_utf8_lossy(&o.stdout);
        let mut it = s.lines();
        if let (Some(f), Some(i)) = (it.next(), it.next()) {
            let p = PathBuf::from(f);
            if p.exists() {
                out.push((p, i.trim().parse().unwrap_or(0)));
            }
        }
    }
    // Well-known files in the font directories.
    let prefs: &[&str] = match lang {
        "ja" => &[
            "notosanscjkjp",
            "notosansjp",
            "notosanscjk",
            "sourcehansans",
            "ipag",
            "takao",
            "vl-gothic",
            "droidsansfallback",
        ],
        "ko" => &[
            "notosanscjkkr",
            "notosanskr",
            "notosanscjk",
            "sourcehansans",
            "nanumgothic",
            "undotum",
            "droidsansfallback",
        ],
        "zh-tw" => &[
            "notosanscjktc",
            "notosanstc",
            "notosanscjk",
            "sourcehansans",
            "wqy-microhei",
            "wqy-zenhei",
            "droidsansfallback",
        ],
        _ => &[
            "notosanscjksc",
            "notosanssc",
            "notosanscjk",
            "sourcehansans",
            "wqy-microhei",
            "wqy-zenhei",
            "droidsansfallback",
        ],
    };
    let mut dirs: Vec<PathBuf> = vec!["/usr/share/fonts".into(), "/usr/local/share/fonts".into()];
    if let Some(d) = std::env::var_os("XDG_DATA_HOME") {
        dirs.push(PathBuf::from(d).join("fonts"));
    }
    if let Some(h) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(&h).join(".local/share/fonts"));
        dirs.push(PathBuf::from(&h).join(".fonts"));
    }
    let mut files = Vec::new();
    for d in dirs {
        collect(&d, 5, &mut files);
    }
    for pref in prefs {
        for f in &files {
            let name = f
                .file_name()
                .map(|n| {
                    n.to_string_lossy()
                        .to_ascii_lowercase()
                        .replace([' ', '_'], "")
                })
                .unwrap_or_default();
            if name.starts_with(pref) && !out.iter().any(|(p, _)| p == f) {
                let index = if name.ends_with(".ttc") && name.starts_with("notosanscjk") {
                    noto_index
                } else {
                    0
                };
                out.push((f.clone(), index));
            }
        }
    }
    out
}

fn collect(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if depth > 0 {
                collect(&p, depth - 1, out);
            }
        } else if p
            .extension()
            .and_then(|x| x.to_str())
            .is_some_and(|x| matches!(x.to_ascii_lowercase().as_str(), "ttf" | "otf" | "ttc"))
        {
            out.push(p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locales() {
        assert_eq!(cjk_lang("ja"), Some("ja"));
        assert_eq!(cjk_lang("zh"), Some("zh-cn"));
        assert_eq!(cjk_lang("zh-Hant"), Some("zh-tw"));
        assert_eq!(cjk_lang("en-US"), None);
        assert_eq!(cjk_lang("ru"), None);
    }
}
