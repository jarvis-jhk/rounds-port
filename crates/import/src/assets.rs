//! Shared assets beyond hierarchies: Sonigon sound data (`sounds.json`),
//! card categories (`categories.json`) and the Unity Localization string
//! tables from the Addressables bundles (`localization.json`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::Value as J;
use unity_assets::bundle;

use crate::format::{self, Fields, ObjId};

fn rf(j: &J) -> Option<ObjId> {
    j.get("$ref").and_then(J::as_str).map(String::from)
}

fn num(j: &J, k: &str) -> f32 {
    j.get(k).and_then(J::as_f64).unwrap_or(0.0) as f32
}

fn flag(j: &J, k: &str) -> bool {
    match j.get(k) {
        Some(J::Bool(b)) => *b,
        Some(J::Number(n)) => n.as_f64().is_some_and(|x| x != 0.0),
        _ => false,
    }
}

/// `SoundEvent` from its raw script fields (references as `$ref`).
pub fn sound_event(name: &str, f: &J) -> format::SoundEvent {
    let v = f.get("variables").cloned().unwrap_or(J::Null);
    let timeline = v
        .get("timelineSoundContainerSetting")
        .and_then(J::as_array)
        .cloned()
        .unwrap_or_default();
    let layers = f
        .get("soundContainerArray")
        .and_then(J::as_array)
        .map(|a| {
            a.iter()
                .enumerate()
                .filter_map(|(k, c)| {
                    let t = timeline.get(k);
                    Some(format::SoundEventLayer {
                        container: rf(c)?,
                        delay: t.map_or(0.0, |t| num(t, "delay")),
                        volume_ratio: t.map_or(1.0, |t| num(t, "volumeRatio")),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let trigger = |kind: &str| -> Option<format::SoundTrigger> {
        if !flag(&v, &format!("triggerOn{kind}Enable")) {
            return None;
        }
        Some(format::SoundTrigger {
            events: v
                .get(format!("triggerOn{kind}SoundEvents").as_str())
                .and_then(J::as_array)
                .map(|a| a.iter().filter_map(rf).collect())
                .unwrap_or_default(),
            which: v
                .get(format!("triggerOn{kind}WhichToPlayEnum").as_str())
                .and_then(J::as_i64)
                .unwrap_or(0),
        })
    };
    format::SoundEvent {
        name: name.to_owned(),
        layers,
        cooldown: num(&v, "cooldownTime"),
        probability: v.get("probability").and_then(J::as_f64).unwrap_or(100.0) as f32,
        disabled: flag(&v, "disableEnable"),
        poly_group: v.get("soundPolyGroup").and_then(rf),
        poly_group_priority: num(&v, "soundPolyGroupPriority"),
        mix: v.get("soundMix").and_then(rf),
        modifier: v.get("soundEventModifier").cloned().unwrap_or(J::Null),
        trigger_on_play: trigger("Play"),
        trigger_on_stop: trigger("Stop"),
        trigger_on_tail: trigger("Tail"),
        tail_length: num(&v, "triggerOnTailLength"),
        fields: v,
    }
}

/// `SoundContainer` from its raw script fields; `audio` maps clip ids to
/// their imported files.
pub fn sound_container(
    name: &str,
    f: &J,
    audio: &BTreeMap<ObjId, format::AudioClip>,
) -> format::SoundContainer {
    let s = f.get("setting").cloned().unwrap_or(J::Null);
    let clips: Vec<ObjId> = f
        .get("audioClip")
        .and_then(J::as_array)
        .map(|a| a.iter().filter_map(rf).collect())
        .unwrap_or_default();
    format::SoundContainer {
        name: name.to_owned(),
        files: clips
            .iter()
            .filter_map(|c| audio.get(c).map(|a| a.file.clone()))
            .collect(),
        clips,
        looping: flag(&s, "loopEnabled"),
        volume_db: num(&s, "volumeDecibel"),
        volume_ratio: num(&s, "volumeRatio"),
        volume_random_db: flag(&s, "volumeRandomEnable").then(|| num(&s, "volumeRandomRangeDb")),
        pitch_semitone: num(&s, "pitchSemitone"),
        pitch_ratio: num(&s, "pitchRatio"),
        pitch_random_semitone: flag(&s, "pitchRandomEnable")
            .then(|| num(&s, "pitchRandomRangeSemitone")),
        spatial_blend: num(&s, "spatialBlend"),
        distance_enabled: flag(&s, "distanceEnabled"),
        distance_scale: num(&s, "distanceScale"),
        priority: num(&s, "priority"),
        random_start_position: flag(&s, "randomStartPosition"),
        settings: s,
    }
}

/// Shot/impact modifier: every non-null reference field is a slot.
pub fn sound_modifier(name: &str, f: &J) -> format::SoundModifier {
    let mut events = BTreeMap::new();
    if let J::Object(m) = f {
        for (k, v) in m {
            if let Some(id) = rf(v) {
                events.insert(k.clone(), id);
            }
        }
    }
    format::SoundModifier {
        name: name.to_owned(),
        priority: f.get("priority").and_then(J::as_i64).unwrap_or(0),
        events,
    }
}

pub fn poly_group(name: &str, f: &J) -> format::SoundPolyGroup {
    format::SoundPolyGroup {
        name: name.to_owned(),
        limit: flag(f, "polyphonyLimitEnable")
            .then(|| f.get("polyphonyLimit").and_then(J::as_i64).unwrap_or(0)),
        skip_lower_priority: flag(f, "skipLowerPriority"),
        allow_fade_out: flag(f, "allowFadeOut"),
    }
}

pub fn sound_mix(name: &str, f: &J) -> format::SoundMix {
    format::SoundMix {
        name: name.to_owned(),
        parent: f.get("soundMixParent").and_then(rf),
        modifier: f.get("soundEventModifier").cloned().unwrap_or(J::Null),
    }
}

/// Adds every field of `fields` (component `script`) that references one of
/// `targets` to `out` as `"<script>.<path>" -> ids`.
pub fn collect_references(
    script: &str,
    fields: &Fields,
    targets: &BTreeSet<ObjId>,
    out: &mut BTreeMap<String, BTreeSet<ObjId>>,
) {
    fn walk(
        j: &J,
        path: &mut String,
        targets: &BTreeSet<ObjId>,
        out: &mut BTreeMap<String, BTreeSet<ObjId>>,
    ) {
        match j {
            J::Object(m) => {
                if let Some(J::String(id)) = m.get("$ref") {
                    if targets.contains(id) {
                        out.entry(path.clone()).or_default().insert(id.clone());
                    }
                    return;
                }
                for (k, v) in m {
                    let n = path.len();
                    path.push('.');
                    path.push_str(k);
                    walk(v, path, targets, out);
                    path.truncate(n);
                }
            }
            J::Array(a) => {
                let n = path.len();
                path.push_str("[]");
                for v in a {
                    walk(v, path, targets, out);
                }
                path.truncate(n);
            }
            _ => {}
        }
    }
    let mut path = script.to_owned();
    walk(fields, &mut path, targets, out);
}

/// Reads the Localization string tables from the Addressables bundles under
/// `<data dir>/StreamingAssets/aa/<platform>/`. Missing bundles give an
/// empty result; problems are returned as warnings.
pub fn localization(data_dir: &Path) -> (format::Localization, Vec<String>) {
    let mut warnings = Vec::new();
    let mut out = format::Localization::default();
    let aa = data_dir.join("StreamingAssets").join("aa");
    let mut bundles = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&aa) {
        for d in rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_dir())
        {
            if let Ok(rd2) = std::fs::read_dir(&d) {
                bundles.extend(rd2.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| {
                    p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                        n.ends_with(".bundle")
                            && (n.starts_with("localization-string-tables")
                                || n.starts_with("localization-assets-shared"))
                    })
                }));
            }
        }
    }
    bundles.sort();
    if bundles.is_empty() {
        warnings.push(format!(
            "no localisation bundles under {}; localization.json is empty",
            aa.display()
        ));
        return (out, warnings);
    }
    // (table name, locale, entries id -> text)
    type Entries = Vec<(i64, String)>;
    let mut tables: Vec<(String, String, Entries)> = Vec::new();
    let mut locales = BTreeSet::new();
    for path in &bundles {
        let entries = match std::fs::read(path)
            .map_err(|e| e.to_string())
            .and_then(|d| bundle::read_bundle(&d).map_err(|e| e.to_string()))
        {
            Ok(e) => e,
            Err(e) => {
                warnings.push(format!("{}: {e}", path.display()));
                continue;
            }
        };
        for e in entries.iter().filter(|e| e.flags & 4 != 0) {
            let (objs, errs) = match bundle::read_embedded(&e.data) {
                Ok(r) => r,
                Err(err) => {
                    warnings.push(format!("{} {}: {err}", path.display(), e.path));
                    continue;
                }
            };
            for err in errs {
                warnings.push(format!("{} {}: {err}", path.display(), e.path));
            }
            for o in objs.iter().filter(|o| o.class_id == 114) {
                let v = &o.value;
                let entries = v.get("m_Entries");
                if !entries.is_null() && !v.get("m_TableCollectionName").is_null() {
                    let name = v.get("m_TableCollectionName").str().to_owned();
                    let t = out.tables.entry(name).or_default();
                    t.guid = v
                        .get("m_TableCollectionNameGuidString")
                        .str()
                        .replace('-', "")
                        .to_ascii_lowercase();
                    for en in entries.as_array() {
                        t.keys
                            .insert(en.get("m_Id").i64(), en.get("m_Key").str().to_owned());
                    }
                } else if !v.get("m_TableData").is_null() {
                    let code = v.path("m_LocaleId.m_Code").str().to_owned();
                    let name = v.get("m_Name").str();
                    let table = name
                        .strip_suffix(&format!("_{code}"))
                        .unwrap_or(name)
                        .to_owned();
                    let data = v
                        .get("m_TableData")
                        .as_array()
                        .iter()
                        .map(|d| (d.get("m_Id").i64(), d.get("m_Localized").str().to_owned()))
                        .collect();
                    locales.insert(code.clone());
                    tables.push((table, code, data));
                }
            }
        }
    }
    for (table, code, data) in tables {
        let Some(t) = out.tables.get_mut(&table) else {
            warnings.push(format!("string table {table} ({code}) has no shared data"));
            continue;
        };
        let strings = t.strings.entry(code.clone()).or_default();
        for (id, text) in data {
            match t.keys.get(&id) {
                Some(k) => {
                    strings.insert(k.clone(), text);
                }
                None => warnings.push(format!("{table} ({code}): unknown key id {id}")),
            }
        }
    }
    out.locales = locales.into_iter().collect();
    (out, warnings)
}

/// Key name of a raw `LocalizedString` field.
pub fn localized_key(loc: &format::Localization, localized: &J) -> Option<String> {
    let t = loc.table(
        localized
            .pointer("/m_TableReference/m_TableCollectionName")?
            .as_str()?,
    )?;
    let e = localized.get("m_TableEntryReference")?;
    t.key(
        e.get("m_Key").and_then(J::as_str).unwrap_or(""),
        e.get("m_KeyId").and_then(J::as_i64).unwrap_or(0),
    )
    .map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn references_are_collected_with_paths() {
        let targets: BTreeSet<ObjId> = ["a:1".to_string(), "a:2".to_string()].into();
        let f =
            json!({"s": {"$ref": "a:1"}, "list": [{"e": {"$ref": "a:2"}}, {"e": {"$ref": "x:9"}}]});
        let mut out = BTreeMap::new();
        collect_references("Thing", &f, &targets, &mut out);
        assert_eq!(out.len(), 2);
        assert!(out["Thing.s"].contains("a:1"));
        assert!(out["Thing.list[].e"].contains("a:2"));
    }

    #[test]
    fn container_and_event_fields() {
        let audio: BTreeMap<ObjId, format::AudioClip> = [(
            "f:7".to_string(),
            format::AudioClip {
                name: "c".into(),
                file: "audio/c.ogg".into(),
                channels: 1,
                frequency: 44100,
                length: 1.0,
                samples: 44100,
            },
        )]
        .into();
        let c = sound_container(
            "X_SC",
            &json!({"audioClip": [{"$ref": "f:7"}, {"$ref": "f:8"}],
                    "setting": {"loopEnabled": 1, "volumeRandomEnable": 0, "volumeRandomRangeDb": -3.0,
                                "pitchRandomEnable": 1, "pitchRandomRangeSemitone": 0.25}}),
            &audio,
        );
        assert_eq!(c.clips.len(), 2);
        assert_eq!(c.files, ["audio/c.ogg"]);
        assert!(c.looping && c.volume_random_db.is_none());
        assert_eq!(c.pitch_random_semitone, Some(0.25));
        let e = sound_event(
            "X_SE",
            &json!({"soundContainerArray": [{"$ref": "f:1"}],
                    "variables": {"timelineSoundContainerSetting": [{"delay": 0.2, "volumeRatio": 1.0}],
                                  "cooldownTime": 0.03, "probability": 100.0,
                                  "triggerOnPlayEnable": 1, "triggerOnPlaySoundEvents": [{"$ref": "f:2"}, null]}}),
        );
        assert_eq!(e.layers[0].container, "f:1");
        assert_eq!(e.layers[0].delay, 0.2);
        assert_eq!(e.trigger_on_play.unwrap().events, ["f:2"]);
        assert!(e.trigger_on_stop.is_none());
    }
}
