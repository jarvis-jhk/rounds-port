//! `rounds-sim`: the deterministic, rollback-able game simulation.
//! See docs/sim-design.md and docs/spec/*.md.
//!
//! Entry points: [`Content`] (immutable data), [`MatchSetup`], [`World`]
//! (`new` / `step` / `events` / `checksum`, `Clone` for rollback) and
//! [`PlayerInput`] (8-byte `Pod` input per player per tick).

pub mod cards;
pub mod content;
pub mod effects;
pub mod events;
pub mod flow;
pub mod geom;
pub mod gun;
pub mod input;
pub mod map;
pub mod math;
pub mod objects;
pub mod physics;
pub mod player;
pub mod projectile;
pub mod rng;
pub mod scene;
pub mod script;
pub mod stats;
pub mod view;
pub mod world;

// Regression tests for docs/spec/review/sim-vs-spec.md; the real-data fixture of
// the integration tests is shared through the crate's own name.
#[cfg(test)]
extern crate self as rounds_sim;
#[cfg(test)]
mod review_tests;
#[cfg(test)]
#[path = "../tests/fixture.rs"]
mod test_fixture;

pub use content::{CardId, Content, ContentDefs, MapId, ObjectId};
pub use events::{Event, EventKind};
pub use flow::{PickHand, PickStage, PickState};
pub use input::PlayerInput;
pub use math::{V2, v2};
pub use world::{DT, GameMode, MAX_PLAYERS, MatchSetup, PickMode, SlotSetup, TeamMode, World};
