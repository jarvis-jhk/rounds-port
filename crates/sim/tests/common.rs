//! Shared helpers for the sim integration tests.
#![allow(dead_code)]

use rounds_sim::content::test_maps::test_content;
use rounds_sim::{Content, GameMode, MapId, MatchSetup, PlayerInput, World, v2};

pub fn content() -> Content {
    test_content()
}

/// Sandbox world on test map `map` with `n` players; stepped until everyone
/// stands (flight done + settle).
pub fn sandbox(c: &Content, map: u16, n: usize) -> World {
    let mut s = MatchSetup::ffa(n, 7);
    s.mode = GameMode::Sandbox;
    s.map_rotation = vec![MapId(map)];
    let mut w = World::new(c, &s);
    let idle = vec![PlayerInput::default(); n];
    for _ in 0..300 {
        w.step(c, &idle);
    }
    assert!(
        w.players().iter().all(|p| p.playing),
        "players should be playing"
    );
    w
}

pub fn input(mx: f32, my: f32, buttons: u16) -> PlayerInput {
    PlayerInput::from_axes(v2(mx, my), v2(0.0, 0.0), buttons)
}

pub fn aim_input(mx: f32, ax: f32, ay: f32, buttons: u16) -> PlayerInput {
    PlayerInput::from_axes(v2(mx, 0.0), v2(ax, ay), buttons)
}
