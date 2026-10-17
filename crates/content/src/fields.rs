//! Helpers for the importer's raw component fields (`serde_json::Value`,
//! Unity field names, references as `{"$ref": id}`), and the rewriting of
//! raw fields into the parameter form handed to the sim as
//! [`rounds_sim::content::EffectDesc::params`].
//!
//! Rewritten parameter form (documented for the effect implementation):
//! - references: `{"$ref": id}` gains a resolution key:
//!   `"$prefab": "<ObjectDef/ProjectileDef name>"` for spawnable prefabs,
//!   `"$path": "A/B"` (+ `"$component": "<script or class>"` when the target
//!   is a component) for objects inside the same prefab/scene,
//!   `"$audio": "<clip name>"`, `"$sprite": "<sprite name>"`;
//! - `AnimationCurve` (`{"m_Curve": [...]}`) becomes
//!   `{"keys": [{"time","value","in_slope","out_slope"}], "pre": n, "post": n,
//!   "weighted": bool}`, i.e. `serde_json::from_value::<rounds_sim::math::Curve>`
//!   works directly;
//! - `UnityEvent` (`{"m_PersistentCalls": ...}`) becomes
//!   `{"calls": [{"method", "target", "mode", "arg", "state"}]}` with `target`
//!   rewritten like any reference and `arg` the argument matching `mode`
//!   (1 void → null, 2 object, 3 int, 4 float, 5 string, 6 bool);
//! - everything else is passed through (numbers, `0/1` flags, `{x,y,z}`,
//!   `{r,g,b,a}`, `{"m_Bits": n}` layer masks).

use std::collections::BTreeMap;

use rounds_sim::math::{Curve, Key, V2, v2};
use serde_json::{Map, Value, json};

/// Number field (f32), `None` if missing or not a number.
pub fn num(v: &Value, key: &str) -> Option<f32> {
    v.get(key)?.as_f64().map(|x| x as f32)
}

/// Number field with a default.
pub fn f(v: &Value, key: &str, default: f32) -> f32 {
    num(v, key).unwrap_or(default)
}

/// Integer field with a default.
pub fn i(v: &Value, key: &str, default: i64) -> i64 {
    match v.get(key) {
        Some(x) => x
            .as_i64()
            .or_else(|| x.as_f64().map(|f| f as i64))
            .unwrap_or(default),
        None => default,
    }
}

/// Flag field (`0/1` or `true/false`) with a default.
pub fn b(v: &Value, key: &str, default: bool) -> bool {
    match v.get(key) {
        Some(Value::Bool(x)) => *x,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|x| x != 0.0),
        _ => default,
    }
}

/// The id of a reference value `{"$ref": id}`.
pub fn ref_id(v: &Value) -> Option<&str> {
    v.get("$ref")?.as_str()
}

/// The id of a reference field.
pub fn field_ref<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    ref_id(v.get(key)?)
}

/// `{x, y[, z]}` field.
pub fn vec2(v: &Value, key: &str) -> Option<V2> {
    let o = v.get(key)?;
    Some(v2(num(o, "x")?, num(o, "y")?))
}

/// `{x, y, z}` field's z.
pub fn vec_z(v: &Value, key: &str) -> Option<f32> {
    num(v.get(key)?, "z")
}

/// A raw Unity `AnimationCurve` value.
pub fn is_curve(v: &Value) -> bool {
    v.get("m_Curve").is_some_and(Value::is_array)
}

/// Converts a raw `AnimationCurve` (`{"m_Curve": [...]}`) to the sim's curve.
pub fn curve(v: &Value) -> Option<Curve> {
    let keys = v.get("m_Curve")?.as_array()?;
    Some(Curve {
        keys: keys
            .iter()
            .map(|k| Key {
                time: f(k, "time", 0.0),
                value: f(k, "value", 0.0),
                in_slope: finite(f(k, "inSlope", 0.0)),
                out_slope: finite(f(k, "outSlope", 0.0)),
            })
            .collect(),
    })
}

/// Unity stores infinite tangents for stepped keys; the importer writes
/// non-finite floats as `null` (read as 0 by `f`). Keep values finite.
fn finite(x: f32) -> f32 {
    if x.is_finite() { x } else { 0.0 }
}

/// Curve field of a component.
pub fn field_curve(v: &Value, key: &str) -> Option<Curve> {
    curve(v.get(key)?)
}

/// What a reference id resolves to.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// A spawnable prefab converted to an `ObjectDef`/`ProjectileDef`.
    Prefab(String),
    /// An object (and optionally a component of it) in the same hierarchy.
    Local {
        path: String,
        component: Option<String>,
    },
    Audio(String),
    Sprite(String),
}

/// Resolves reference ids while rewriting parameters.
pub trait Resolve {
    fn resolve(&self, id: &str) -> Option<Target>;
}

impl<F: Fn(&str) -> Option<Target>> Resolve for F {
    fn resolve(&self, id: &str) -> Option<Target> {
        self(id)
    }
}

fn rewrite_ref(id: &str, r: &dyn Resolve) -> Value {
    let mut m = Map::new();
    m.insert("$ref".into(), Value::String(id.into()));
    match r.resolve(id) {
        Some(Target::Prefab(n)) => {
            m.insert("$prefab".into(), Value::String(n));
        }
        Some(Target::Local { path, component }) => {
            m.insert("$path".into(), Value::String(path));
            if let Some(c) = component {
                m.insert("$component".into(), Value::String(c));
            }
        }
        Some(Target::Audio(n)) => {
            m.insert("$audio".into(), Value::String(n));
        }
        Some(Target::Sprite(n)) => {
            m.insert("$sprite".into(), Value::String(n));
        }
        None => {}
    }
    Value::Object(m)
}

/// Rewrites raw fields into the parameter form (see the module docs).
pub fn rewrite(v: &Value, r: &dyn Resolve) -> Value {
    match v {
        Value::Array(a) => Value::Array(a.iter().map(|x| rewrite(x, r)).collect()),
        Value::Object(o) => {
            if let Some(id) = ref_id(v)
                && o.len() == 1
            {
                return rewrite_ref(id, r);
            }
            if is_curve(v) {
                let weighted = v["m_Curve"]
                    .as_array()
                    .is_some_and(|k| k.iter().any(|k| i(k, "weightedMode", 0) != 0));
                let c = curve(v).unwrap_or_default();
                return json!({
                    "keys": serde_json::to_value(&c.keys).unwrap_or(Value::Null),
                    "pre": i(v, "m_PreInfinity", 2),
                    "post": i(v, "m_PostInfinity", 2),
                    "weighted": weighted,
                });
            }
            if let Some(pc) = o.get("m_PersistentCalls") {
                let calls = pc
                    .get("m_Calls")
                    .and_then(Value::as_array)
                    .map(|calls| calls.iter().map(|c| rewrite_call(c, r)).collect())
                    .unwrap_or_default();
                return json!({ "calls": Value::Array(calls) });
            }
            Value::Object(o.iter().map(|(k, x)| (k.clone(), rewrite(x, r))).collect())
        }
        _ => v.clone(),
    }
}

fn rewrite_call(c: &Value, r: &dyn Resolve) -> Value {
    let mode = i(c, "m_Mode", 1);
    let args = c.get("m_Arguments").cloned().unwrap_or(Value::Null);
    let arg = match mode {
        2 => args
            .get("m_ObjectArgument")
            .map(|a| rewrite(a, r))
            .unwrap_or(Value::Null),
        3 => args.get("m_IntArgument").cloned().unwrap_or(Value::Null),
        4 => args.get("m_FloatArgument").cloned().unwrap_or(Value::Null),
        5 => args.get("m_StringArgument").cloned().unwrap_or(Value::Null),
        6 => args.get("m_BoolArgument").cloned().unwrap_or(Value::Null),
        _ => Value::Null,
    };
    json!({
        "method": c.get("m_MethodName").cloned().unwrap_or(Value::Null),
        "target": c.get("m_Target").map(|t| rewrite(t, r)).unwrap_or(Value::Null),
        "mode": mode,
        "arg": arg,
        "state": i(c, "m_CallState", 2),
    })
}

/// Every reference id inside a raw value (depth first, in order).
pub fn refs_in(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Array(a) => a.iter().for_each(|x| refs_in(x, out)),
        Value::Object(o) => {
            if let Some(id) = ref_id(v)
                && o.len() == 1
            {
                out.push(id.to_string());
                return;
            }
            o.values().for_each(|x| refs_in(x, out));
        }
        _ => {}
    }
}

/// Replaces every `{"$ref": id}` whose id is in `names` by the name string
/// (prefab references become the names the sim's serde types expect), and
/// any other reference by `null`.
pub fn refs_to_names(v: &Value, names: &BTreeMap<String, String>) -> Value {
    match v {
        Value::Array(a) => Value::Array(a.iter().map(|x| refs_to_names(x, names)).collect()),
        Value::Object(o) => {
            if let Some(id) = ref_id(v)
                && o.len() == 1
            {
                return names
                    .get(id)
                    .map(|n| Value::String(n.clone()))
                    .unwrap_or(Value::Null);
            }
            Value::Object(
                o.iter()
                    .map(|(k, x)| (k.clone(), refs_to_names(x, names)))
                    .collect(),
            )
        }
        _ => v.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curve_and_event_rewrite() {
        let raw = json!({
            "curve": {"m_Curve": [
                {"time": 0.0, "value": 1.0, "inSlope": 0.0, "outSlope": 2.0, "weightedMode": 0},
                {"time": 1.0, "value": 3.0, "inSlope": null, "outSlope": 0.0, "weightedMode": 0}
            ], "m_PreInfinity": 2, "m_PostInfinity": 2},
            "ev": {"m_PersistentCalls": {"m_Calls": [
                {"m_Arguments": {"m_FloatArgument": 0.5}, "m_CallState": 2, "m_MethodName": "Go",
                 "m_Mode": 4, "m_Target": {"$ref": "f:7"}}
            ]}},
            "spawn": [{"$ref": "f:9"}, null],
            "flag": 1
        });
        let r = |id: &str| match id {
            "f:7" => Some(Target::Local {
                path: "Child".into(),
                component: Some("Explosion".into()),
            }),
            "f:9" => Some(Target::Prefab("E_Boom".into())),
            _ => None,
        };
        let p = rewrite(&raw, &r);
        let c: Curve = serde_json::from_value(p["curve"].clone()).unwrap();
        assert_eq!(c.keys.len(), 2);
        assert_eq!(c.keys[0].out_slope, 2.0);
        assert_eq!(c.keys[1].in_slope, 0.0);
        assert_eq!(p["ev"]["calls"][0]["method"], "Go");
        assert_eq!(p["ev"]["calls"][0]["arg"], 0.5);
        assert_eq!(p["ev"]["calls"][0]["target"]["$path"], "Child");
        assert_eq!(p["ev"]["calls"][0]["target"]["$component"], "Explosion");
        assert_eq!(p["spawn"][0]["$prefab"], "E_Boom");
        assert_eq!(p["spawn"][1], Value::Null);
        assert_eq!(p["flag"], 1);
        let mut refs = vec![];
        refs_in(&raw, &mut refs);
        assert_eq!(refs, vec!["f:7".to_string(), "f:9".to_string()]);
    }

    #[test]
    fn names_for_serde() {
        let mut names = BTreeMap::new();
        names.insert("a:1".to_string(), "A_Explosion".to_string());
        let v = refs_to_names(
            &json!({"effect": {"$ref": "a:1"}, "other": {"$ref": "a:2"}}),
            &names,
        );
        assert_eq!(v, json!({"effect": "A_Explosion", "other": null}));
    }
}
