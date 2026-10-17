//! UI texts from the import's string tables (ui.md 7) in the chosen
//! locale, with English fallbacks for keys the original does not have.

use std::rc::Rc;

use rounds_import::GameData;

pub struct Strings {
    data: Rc<GameData>,
    pub locale: String,
}

impl Strings {
    pub fn new(data: Rc<GameData>, locale: &str) -> Strings {
        Strings {
            data,
            locale: locale.to_owned(),
        }
    }

    /// Locales the import has (sorted, `en-US` first).
    pub fn locales(&self) -> Vec<String> {
        let mut v = self.data.localization.locales.clone();
        v.sort_by_key(|l| (l != "en-US", l.clone()));
        if v.is_empty() {
            v.push("en-US".into());
        }
        v
    }

    /// Text of `key` in `StringTableDefault`, else `fallback`. TMP tags
    /// (`<sprite ...>`, `<b>`) are removed.
    pub fn tr(&self, key: &str, fallback: &str) -> String {
        let t = self
            .data
            .localization
            .text(&self.locale, "StringTableDefault", key)
            .filter(|t| !t.trim().is_empty())
            .unwrap_or(fallback);
        strip_tags(t)
    }

    /// Like [`tr`](Self::tr), upper-cased (TMP style flag 16).
    pub fn up(&self, key: &str, fallback: &str) -> String {
        self.tr(key, fallback).to_uppercase()
    }
}

/// Removes `<...>` tags and collapses the remaining whitespace runs.
pub fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0;
    for ch in s.chars() {
        match ch {
            '<' => depth += 1,
            '>' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    out.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags() {
        assert_eq!(
            strip_tags("Press <sprite name=BUTTON_SOUTH> to join"),
            "Press  to join"
        );
        assert_eq!(strip_tags("<b>Hi</b>"), "Hi");
    }
}
