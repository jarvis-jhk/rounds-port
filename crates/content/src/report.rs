//! What the conversion did: counts, components it could not represent,
//! deliberately ignored (presentation/network/editor) components, warnings,
//! and the comparison of the sim's hard-coded tuning with the imported data.

use std::collections::BTreeMap;
use std::fmt;

/// One compared tuning value.
#[derive(Clone, Debug, PartialEq)]
pub struct TuningDiff {
    /// What, e.g. `"PlayerMovement.force"`.
    pub name: String,
    /// The imported (serialized) value.
    pub imported: String,
    /// What the sim uses now.
    pub sim: String,
    /// Where the sim's value lives.
    pub sim_source: String,
    pub matches: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Report {
    /// Fatal problems (`build` returns `Err` when non-empty).
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    /// Totals (`maps`, `statics`, `props`, `ropes`, `cards`, ...).
    pub counts: BTreeMap<String, usize>,
    /// Gameplay-relevant components found in the data with no sim
    /// representation (dropped).
    pub unsupported: BTreeMap<String, usize>,
    /// Components skipped on purpose (rendering, audio, networking, editor
    /// tooling), with counts.
    pub ignored: BTreeMap<String, usize>,
    /// Behaviour scripts handed to the sim as `EffectDesc`s (card effect
    /// objects, projectile add-ons), with counts.
    pub effects: BTreeMap<String, usize>,
    /// Imported values vs. the sim's current hard-coded ones.
    pub tuning: Vec<TuningDiff>,
}

impl Report {
    pub fn count(&mut self, key: &str, n: usize) {
        *self.counts.entry(key.to_string()).or_default() += n;
    }
    pub fn unsupported(&mut self, key: &str) {
        *self.unsupported.entry(key.to_string()).or_default() += 1;
    }
    pub fn ignored(&mut self, key: &str) {
        *self.ignored.entry(key.to_string()).or_default() += 1;
    }
    pub fn effect(&mut self, key: &str) {
        *self.effects.entry(key.to_string()).or_default() += 1;
    }
    pub fn warn(&mut self, w: impl Into<String>) {
        self.warnings.push(w.into());
    }
    pub fn error(&mut self, e: impl Into<String>) {
        self.errors.push(e.into());
    }
}

fn table(f: &mut fmt::Formatter<'_>, title: &str, m: &BTreeMap<String, usize>) -> fmt::Result {
    writeln!(
        f,
        "{title} ({} kinds, {} total):",
        m.len(),
        m.values().sum::<usize>()
    )?;
    let mut v: Vec<_> = m.iter().collect();
    v.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    for (k, n) in v {
        writeln!(f, "  {n:6}  {k}")?;
    }
    Ok(())
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "counts:")?;
        for (k, n) in &self.counts {
            writeln!(f, "  {k:32} {n}")?;
        }
        table(f, "unsupported components (dropped)", &self.unsupported)?;
        table(
            f,
            "effect scripts passed to the sim as EffectDesc",
            &self.effects,
        )?;
        table(
            f,
            "ignored components (presentation/network/editor)",
            &self.ignored,
        )?;
        let mism: Vec<_> = self.tuning.iter().filter(|t| !t.matches).collect();
        writeln!(
            f,
            "tuning: {} values compared, {} differ:",
            self.tuning.len(),
            mism.len()
        )?;
        for t in &self.tuning {
            writeln!(
                f,
                "  {} {:44} imported {:>14}  sim {:>14}  ({})",
                if t.matches { " " } else { "!" },
                t.name,
                t.imported,
                t.sim,
                t.sim_source
            )?;
        }
        writeln!(f, "warnings ({}):", self.warnings.len())?;
        for w in &self.warnings {
            writeln!(f, "  {w}")?;
        }
        writeln!(f, "errors ({}):", self.errors.len())?;
        for e in &self.errors {
            writeln!(f, "  {e}")?;
        }
        Ok(())
    }
}
