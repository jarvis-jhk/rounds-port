//! A small player for one layer of an imported [`format::AnimController`]:
//! enters the default state, loops or holds clips, takes transitions (exit
//! time and parameter conditions, any-state transitions, triggers) and
//! cross-fades. Presentation only; enough for the card art animations.
//!
//! ```ignore
//! let mut a = AnimatorPlayer::new(ctrl, 0);
//! a.update(ctrl, &data.animations, dt);
//! for s in a.sample(ctrl, &data.animations) {
//!     // apply clip `s.clip` at time `s.time` with weight `s.weight`
//! }
//! ```

use std::collections::BTreeMap;

use crate::format::{AnimClip, AnimController, AnimState, AnimTransition, Animations, ObjId};

/// One clip to apply with its local time and blend weight.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipSample {
    /// Clip id (key of [`Animations::clips`]).
    pub clip: ObjId,
    /// Time inside the clip (seconds, between its `start` and `stop`).
    pub time: f32,
    /// Blend weight (the weights of one sample sum to 1).
    pub weight: f32,
}

/// Playback state of one controller layer.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimatorPlayer {
    /// Layer index.
    pub layer: usize,
    /// Current state index.
    pub state: usize,
    /// Seconds spent in the current state (before speed).
    pub time: f32,
    /// Cross-fade source: (state, its time), fade elapsed and duration in
    /// seconds.
    pub fade_from: Option<(usize, f32, f32, f32)>,
    /// Parameter values by name (bools and triggers as 0/1).
    pub params: BTreeMap<String, f32>,
}

fn clip_len(c: &AnimClip) -> f32 {
    (c.stop - c.start).max(1e-6)
}

fn state_len(st: &AnimState, anims: &Animations) -> f32 {
    st.clip
        .as_ref()
        .and_then(|c| anims.clips.get(c))
        .map_or(1.0, clip_len)
}

impl AnimatorPlayer {
    /// Starts `layer` of `ctrl` in its default state with default parameters.
    pub fn new(ctrl: &AnimController, layer: usize) -> AnimatorPlayer {
        let state = ctrl.layers.get(layer).map_or(0, |l| l.default_state);
        AnimatorPlayer {
            layer,
            state,
            time: 0.0,
            fade_from: None,
            params: ctrl
                .parameters
                .iter()
                .map(|p| (p.name.clone(), p.default))
                .collect(),
        }
    }

    /// Sets a parameter (float, int, bool as 0/1).
    pub fn set(&mut self, name: &str, value: f32) {
        self.params.insert(name.to_owned(), value);
    }

    /// Sets a trigger (reset when a transition consumes it).
    pub fn trigger(&mut self, name: &str) {
        self.set(name, 1.0);
    }

    fn speed(&self, st: &AnimState) -> f32 {
        let k = st
            .speed_parameter
            .as_ref()
            .and_then(|p| self.params.get(p))
            .copied()
            .unwrap_or(1.0);
        st.speed * k
    }

    /// Normalised time of a state after `t` seconds (cycle offset included).
    fn normalized(&self, st: &AnimState, anims: &Animations, t: f32) -> f32 {
        t * self.speed(st) / state_len(st, anims) + st.cycle_offset
    }

    fn conditions_hold(&self, tr: &AnimTransition) -> bool {
        tr.conditions.iter().all(|c| {
            let v = self.params.get(&c.parameter).copied().unwrap_or(0.0);
            match c.mode.as_str() {
                "if" => v != 0.0,
                "if_not" => v == 0.0,
                "greater" => v > c.threshold,
                "less" => v < c.threshold,
                "equals" => v == c.threshold,
                "not_equal" => v != c.threshold,
                _ => true,
            }
        })
    }

    /// Whether `tr` fires while the source state goes from normalised time
    /// `n0` to `n1` (looping states test the exit time in every cycle when
    /// it is below 1).
    fn fires(&self, tr: &AnimTransition, n0: f32, n1: f32, looping: bool) -> bool {
        if tr.destination.is_none() {
            return false;
        }
        if !tr.has_exit_time && tr.conditions.is_empty() {
            return false;
        }
        if tr.has_exit_time {
            let e = tr.exit_time;
            let crossed = if looping && e < 1.0 {
                let (f0, f1) = (n0 - n0.floor(), n1 - n1.floor());
                n1.floor() > n0.floor() && f1 >= e || (f0 < e && f1 >= e)
            } else {
                n0 < e && n1 >= e
            };
            if !crossed {
                return false;
            }
        }
        self.conditions_hold(tr)
    }

    /// Advances by `dt` seconds and takes at most one transition.
    pub fn update(&mut self, ctrl: &AnimController, anims: &Animations, dt: f32) {
        let Some(layer) = ctrl.layers.get(self.layer) else {
            return;
        };
        let Some(st) = layer.states.get(self.state) else {
            return;
        };
        if let Some((s, t, e, d)) = self.fade_from {
            let e = e + dt;
            self.fade_from = (e < d).then_some((s, t + dt, e, d));
        }
        let n0 = self.normalized(st, anims, self.time);
        self.time += dt;
        let n1 = self.normalized(st, anims, self.time);
        let any = layer
            .any_state_transitions
            .iter()
            .filter(|t| t.can_transition_to_self || t.destination != Some(self.state));
        let Some(tr) = any
            .chain(st.outgoing.iter())
            .find(|t| self.fires(t, n0, n1, st.looping))
            .cloned()
        else {
            return;
        };
        for c in &tr.conditions {
            let is_trigger = ctrl
                .parameters
                .iter()
                .any(|p| p.name == c.parameter && p.kind == "trigger");
            if is_trigger {
                self.params.insert(c.parameter.clone(), 0.0);
            }
        }
        let dest = tr.destination.unwrap_or(self.state);
        let dur = if tr.fixed_duration {
            tr.duration
        } else {
            tr.duration * state_len(st, anims) / self.speed(st).abs().max(1e-6)
        };
        self.fade_from = (dur > 0.0).then_some((self.state, self.time, 0.0, dur));
        self.state = dest;
        let dst = &layer.states[dest];
        self.time = tr.offset * state_len(dst, anims) / self.speed(dst).abs().max(1e-6);
    }

    fn clip_sample(
        &self,
        st: &AnimState,
        anims: &Animations,
        t: f32,
        w: f32,
    ) -> Option<ClipSample> {
        let id = st.clip.as_ref()?;
        let c = anims.clips.get(id)?;
        let len = clip_len(c);
        let n = self.normalized(st, anims, t);
        let local = if st.looping || c.looping {
            n - n.floor()
        } else {
            n.clamp(0.0, 1.0)
        };
        Some(ClipSample {
            clip: id.clone(),
            time: c.start + local * len,
            weight: w,
        })
    }

    /// The clips to apply now (the current state, and the fade source while
    /// cross-fading).
    pub fn sample(&self, ctrl: &AnimController, anims: &Animations) -> Vec<ClipSample> {
        let Some(layer) = ctrl.layers.get(self.layer) else {
            return vec![];
        };
        let Some(st) = layer.states.get(self.state) else {
            return vec![];
        };
        let mut out = Vec::new();
        let w = match self.fade_from {
            Some((s, t, e, d)) => {
                let w = (e / d).clamp(0.0, 1.0);
                if let Some(from) = layer.states.get(s) {
                    out.extend(self.clip_sample(from, anims, t, 1.0 - w));
                }
                w
            }
            None => 1.0,
        };
        out.extend(self.clip_sample(st, anims, self.time, w));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::{AnimCondition, AnimLayer, AnimParameter};

    fn setup() -> (AnimController, Animations) {
        let mut anims = Animations::default();
        for (id, looping) in [("a", true), ("b", false)] {
            anims.clips.insert(
                id.into(),
                AnimClip {
                    name: id.into(),
                    start: 0.0,
                    stop: 2.0,
                    looping,
                    ..Default::default()
                },
            );
        }
        let to_b = AnimTransition {
            destination: Some(1),
            destination_raw: 1,
            duration: 0.25,
            has_exit_time: false,
            conditions: vec![AnimCondition {
                mode: "if".into(),
                parameter: "Go".into(),
                threshold: 0.0,
            }],
            ..Default::default()
        };
        let back = AnimTransition {
            destination: Some(0),
            destination_raw: 0,
            exit_time: 1.0,
            has_exit_time: true,
            fixed_duration: true,
            ..Default::default()
        };
        let state = |name: &str, clip: &str, looping, outgoing| AnimState {
            name: name.into(),
            speed: 1.0,
            looping,
            clip: Some(clip.into()),
            transitions: 1,
            outgoing,
            ..Default::default()
        };
        let ctrl = AnimController {
            name: "c".into(),
            clips: vec!["a".into(), "b".into()],
            layers: vec![AnimLayer {
                name: "Base Layer".into(),
                default_weight: 1.0,
                default_state: 0,
                states: vec![
                    state("A", "a", true, vec![to_b]),
                    state("B", "b", false, vec![back]),
                ],
                any_state_transitions: vec![],
            }],
            parameters: vec![AnimParameter {
                name: "Go".into(),
                kind: "trigger".into(),
                default: 0.0,
            }],
        };
        (ctrl, anims)
    }

    #[test]
    fn loops_and_transitions() {
        let (ctrl, anims) = setup();
        let mut a = AnimatorPlayer::new(&ctrl, 0);
        for _ in 0..30 {
            a.update(&ctrl, &anims, 0.1);
        }
        // Looping state wraps: 3.0 s into a 2 s clip.
        let s = a.sample(&ctrl, &anims);
        assert_eq!(s.len(), 1);
        assert!((s[0].time - 1.0).abs() < 1e-4, "{s:?}");
        // Trigger -> B with a 0.25 * 2 s cross-fade; trigger consumed.
        a.trigger("Go");
        a.update(&ctrl, &anims, 0.1);
        assert_eq!(a.state, 1);
        assert_eq!(a.params["Go"], 0.0);
        let s = a.sample(&ctrl, &anims);
        assert_eq!(s.len(), 2);
        assert!((s.iter().map(|x| x.weight).sum::<f32>() - 1.0).abs() < 1e-5);
        // B holds at its end... then the exit-time transition returns to A.
        for _ in 0..19 {
            a.update(&ctrl, &anims, 0.1);
        }
        assert_eq!(a.state, 1);
        a.update(&ctrl, &anims, 0.11);
        assert_eq!(a.state, 0);
    }
}
