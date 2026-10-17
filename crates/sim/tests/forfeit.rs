//! Forfeit / disconnect (match-flow.md 13): `World::set_disconnected` and
//! `PlayerInput::BTN_FORFEIT`.
mod common;
use common::*;
use rounds_sim::flow::{Phase, PickStage};
use rounds_sim::{Content, EventKind, MapId, MatchSetup, PlayerInput, SlotSetup, TeamMode, World};

/// Press jump every other tick (confirms picks), nothing else.
fn picker(frame: u32, n: usize) -> Vec<PlayerInput> {
    let b = if frame.is_multiple_of(2) {
        PlayerInput::BTN_JUMP
    } else {
        0
    };
    vec![input(0.0, 0.0, b); n]
}

/// Deterministic noisy inputs for everybody.
struct Noise(u64);

impl Noise {
    fn next(&mut self, n: usize) -> Vec<PlayerInput> {
        (0..n)
            .map(|_| {
                self.0 ^= self.0 << 13;
                self.0 ^= self.0 >> 7;
                self.0 ^= self.0 << 17;
                let r = (self.0 >> 20) as u32;
                let mut b = 0;
                if r & 3 == 0 {
                    b |= PlayerInput::BTN_JUMP | PlayerInput::BTN_MENU_OK;
                }
                if r & 12 == 0 {
                    b |= PlayerInput::BTN_FIRE;
                }
                let mx = [-1.0, 0.0, 1.0][((r >> 4) % 3) as usize];
                let a = ((r >> 8) % 628) as f32 / 100.0;
                PlayerInput::from_axes(rounds_sim::v2(mx, 0.0), rounds_sim::v2(a.cos(), a.sin()), b)
            })
            .collect()
    }
}

fn run_until(w: &mut World, c: &Content, max: u32, mut f: impl FnMut(&World) -> bool) -> u32 {
    for k in 0..max {
        let inp = picker(w.frame(), w.players().len());
        w.step(c, &inp);
        if f(w) {
            return k;
        }
    }
    panic!("not reached in {max} ticks; phase {:?}", w.phase());
}

fn in_battle(w: &World) -> bool {
    w.phase() == Phase::Battle && w.players().iter().all(|p| p.playing || p.disconnected)
}

fn has(w: &World, f: impl Fn(&EventKind) -> bool) -> bool {
    w.events().iter().any(|e| f(&e.kind))
}

fn setup(n: usize, seed: u64) -> MatchSetup {
    let mut s = MatchSetup::ffa(n, seed);
    s.map_rotation = vec![MapId(0), MapId(1), MapId(2), MapId(3)];
    s
}

fn kill(w: &mut World, c: &Content, i: usize) {
    let pos = w.player(i).pos;
    w.take_damage(c, i, rounds_sim::v2(0.0, 1000.0), pos, None, true, true);
}

#[test]
fn disconnect_is_deterministic_and_rollback_safe() {
    let c = content();
    let n = 4;
    let mut a = World::new(&c, &setup(n, 77));
    let mut b = World::new(&c, &setup(n, 77));
    let mut noise = Noise(0x9e37_79b9);
    const N: usize = 1500;
    let mut saved = None;
    let mut seq = Vec::new();
    for f in 0..4000usize {
        let inp = noise.next(n);
        if f == N {
            // Same decision at the same frame on both peers.
            a.set_disconnected(2);
            b.set_disconnected(2);
            b.set_disconnected(2); // idempotent
            assert!(a.is_disconnected(2) && !a.is_disconnected(1));
            saved = Some(a.clone());
        }
        a.step(&c, &inp);
        b.step(&c, &inp);
        if f >= N {
            seq.push((inp, a.checksum()));
        }
        assert_eq!(a.checksum(), b.checksum(), "desync at frame {f}");
        if f >= N {
            let p = a.player(2);
            assert!(p.disconnected && p.dead && !p.playing, "frame {f}");
        }
    }
    // The match carried on without player 2.
    assert!(
        a.points().iter().chain(a.rounds()).any(|x| *x > 0) || !matches!(a.phase(), Phase::Battle)
    );
    // Rollback to the frame before the disconnect and re-simulate.
    let mut s = saved.unwrap();
    for (k, (inp, sum)) in seq.iter().enumerate() {
        s.step(&c, inp);
        assert_eq!(s.checksum(), *sum, "re-simulation diverged at +{k}");
    }
    // The pending removal is part of the checksum.
    let mut x = World::new(&c, &setup(n, 77));
    let y = x.clone();
    x.set_disconnected(1);
    assert_ne!(x.checksum(), y.checksum());
}

#[test]
fn forfeit_input_matches_set_disconnected() {
    let c = content();
    let n = 3;
    let mut a = World::new(&c, &setup(n, 5));
    let mut b = World::new(&c, &setup(n, 5));
    let mut noise = Noise(42);
    for f in 0..800 {
        let mut inp = noise.next(n);
        if f == 300 {
            a.set_disconnected(1);
            inp[1].buttons |= PlayerInput::BTN_FORFEIT;
        }
        let mut inp_a = inp.clone();
        inp_a[1].buttons &= !PlayerInput::BTN_FORFEIT;
        a.step(&c, &inp_a);
        b.step(&c, &inp);
        assert_eq!(a.checksum(), b.checksum(), "frame {f}");
    }
    assert!(a.is_disconnected(1) && b.is_disconnected(1));
}

#[test]
fn disconnected_player_stays_out_of_later_rounds_and_picks() {
    let c = content();
    let mut w = World::new(&c, &setup(3, 11));
    run_until(&mut w, &c, 4000, in_battle);
    w.set_disconnected(2);
    // The round goes on: two teams remain.
    w.step(&c, &picker(w.frame(), 3));
    assert!(has(&w, |k| matches!(
        k,
        EventKind::PlayerDisconnected { player: 2 }
    )));
    assert_eq!(w.phase(), Phase::Battle);
    assert_eq!(w.player(2).cards.len(), 1);
    // Player 0 kills player 1: team 0 wins the point (not a disconnected team).
    kill(&mut w, &c, 1);
    w.step(&c, &picker(w.frame(), 3));
    assert!(
        matches!(w.phase(), Phase::PointOver { winner: 0, .. }),
        "{:?}",
        w.phase()
    );
    // Next rounds: nobody revives or spawns player 2, and it never picks.
    let mut picks_for_2 = 0;
    for _ in 0..3 {
        run_until(&mut w, &c, 4000, |w| {
            picks_for_2 += w
                .events()
                .iter()
                .filter(|e| {
                    matches!(
                        e.kind,
                        EventKind::PickStarted { player: 2 }
                            | EventKind::CardPicked { player: 2, .. }
                            | EventKind::Revived { player: 2, .. }
                            | EventKind::PlayerSpawned { player: 2, .. }
                    )
                })
                .count();
            w.phase() == Phase::Battle && w.player(0).playing && w.player(1).playing
        });
        let p = w.player(2);
        assert!(p.dead && !p.playing && !p.simulated && p.pos.y > 100.0);
        kill(&mut w, &c, 1);
        w.step(&c, &picker(w.frame(), 3));
        if !matches!(w.phase(), Phase::PointOver { .. } | Phase::RoundOver { .. }) {
            break;
        }
    }
    assert_eq!(picks_for_2, 0);
    assert_eq!(w.player(2).cards.len(), 1);
}

#[test]
fn picker_disconnecting_mid_pick_is_skipped() {
    let c = content();
    let mut w = World::new(&c, &setup(3, 21));
    // Wait for player 1's pick to start choosing (the first pick is 0's).
    let mut seen = false;
    for _ in 0..3000 {
        w.step(&c, &[PlayerInput::default(); 3]);
        if let Some(p) = w.pick()
            && p.player == 0
            && p.stage == PickStage::Choosing
        {
            seen = true;
            break;
        }
    }
    assert!(seen);
    w.set_disconnected(0);
    w.step(&c, &[PlayerInput::default(); 3]);
    assert!(has(&w, |k| matches!(k, EventKind::PickEnded { player: 0 })));
    // The others still pick; 0 gets no card.
    run_until(&mut w, &c, 4000, |w| w.phase() == Phase::Battle);
    assert!(w.player(0).cards.is_empty());
    assert_eq!(w.player(1).cards.len(), 1);
    assert_eq!(w.player(2).cards.len(), 1);
}

#[test]
fn match_ends_when_all_opponents_disconnect() {
    let c = content();
    let mut w = World::new(&c, &setup(3, 9));
    run_until(&mut w, &c, 4000, in_battle);
    let points = w.points().to_vec();
    let rounds = w.rounds().to_vec();
    w.set_disconnected(1);
    w.step(&c, &picker(w.frame(), 3));
    assert_eq!(w.phase(), Phase::Battle, "two teams are still connected");
    w.set_disconnected(2);
    w.step(&c, &picker(w.frame(), 3));
    // Forfeit win for the last team, with the game-over text first.
    assert!(
        matches!(w.phase(), Phase::GameOver { winner: 0, .. }),
        "{:?}",
        w.phase()
    );
    assert!(has(&w, |k| matches!(k, EventKind::GameWon { team: 0 })));
    let mut ended = false;
    for _ in 0..400 {
        w.step(&c, &picker(w.frame(), 3));
        if has(&w, |k| matches!(k, EventKind::MatchEnded)) {
            ended = true;
            break;
        }
    }
    assert!(ended, "{:?}", w.phase());
    assert_eq!(w.phase(), Phase::Ended);
    assert!(!has(&w, |k| matches!(k, EventKind::RematchPrompt)));
    assert_eq!((w.points(), w.rounds()), (&points[..], &rounds[..]));
    // Stable afterwards.
    for _ in 0..50 {
        w.step(&c, &picker(w.frame(), 3));
        assert_eq!(w.phase(), Phase::Ended);
    }
}

#[test]
fn everybody_disconnects_ends_at_once() {
    let c = content();
    let mut w = World::new(&c, &setup(2, 3));
    // During the start sequence.
    for _ in 0..10 {
        w.step(&c, &picker(w.frame(), 2));
    }
    w.set_disconnected(0);
    w.set_disconnected(1);
    w.step(&c, &picker(w.frame(), 2));
    assert_eq!(w.phase(), Phase::Ended);
    assert!(has(&w, |k| matches!(k, EventKind::MatchEnded)));
}

#[test]
fn simultaneous_last_deaths_do_not_award_a_disconnected_team() {
    let c = content();
    let mut w = World::new(&c, &setup(3, 15));
    run_until(&mut w, &c, 4000, in_battle);
    // Player 2 leaves while 0 dies in the same step: 1 is the last team
    // alive and takes the point.
    w.set_disconnected(2);
    kill(&mut w, &c, 0);
    w.step(&c, &picker(w.frame(), 3));
    assert!(
        matches!(w.phase(), Phase::PointOver { winner: 1, .. }),
        "{:?}",
        w.phase()
    );
}

#[test]
fn teams_continue_until_a_whole_team_is_gone() {
    let c = content();
    let mut s = setup(4, 31);
    s.teams = TeamMode::Teams;
    s.slots = [0u8, 0, 1, 1]
        .iter()
        .enumerate()
        .map(|(i, t)| SlotSetup {
            team: *t,
            skin: i as u8,
        })
        .collect();
    let mut w = World::new(&c, &s);
    run_until(&mut w, &c, 4000, in_battle);
    // One member of team 1 leaves: still two teams.
    w.set_disconnected(3);
    w.step(&c, &picker(w.frame(), 4));
    assert_eq!(w.phase(), Phase::Battle);
    // The other dies: team 0 takes the point.
    kill(&mut w, &c, 2);
    w.step(&c, &picker(w.frame(), 4));
    assert!(matches!(w.phase(), Phase::PointOver { winner: 0, .. }));
    // After the transition the connected members are back, 3 and only 3 absent.
    run_until(&mut w, &c, 4000, |w| {
        w.phase() == Phase::Battle
            && w.player(0).playing
            && w.player(1).playing
            && w.player(2).playing
    });
    assert!(!w.player(3).playing && w.player(3).dead);
    // Now the remaining team-1 player leaves too: team 0 wins the match.
    w.set_disconnected(2);
    w.step(&c, &picker(w.frame(), 4));
    assert!(matches!(w.phase(), Phase::GameOver { winner: 0, .. }));
    run_until(&mut w, &c, 400, |w| w.phase() == Phase::Ended);
}

#[test]
fn disconnect_during_rematch_prompt_ends_without_vote() {
    let c = content();
    let mut s = setup(2, 8);
    s.rounds_to_win_game = 1;
    s.points_to_win_round = 1;
    let mut w = World::new(&c, &s);
    run_until(&mut w, &c, 4000, in_battle);
    kill(&mut w, &c, 1);
    w.step(&c, &picker(w.frame(), 2));
    run_until(&mut w, &c, 600, |w| {
        matches!(w.phase(), Phase::Rematch { .. })
    });
    w.set_disconnected(1);
    w.step(&c, &picker(w.frame(), 2));
    assert_eq!(w.phase(), Phase::Ended);
}

#[test]
fn sandbox_disconnect_stops_respawns() {
    let c = content();
    let mut w = sandbox(&c, 0, 2);
    w.set_disconnected(1);
    let idle = vec![PlayerInput::default(); 2];
    for _ in 0..400 {
        w.step(&c, &idle);
    }
    assert!(w.player(1).dead && !w.player(1).playing);
    assert!(w.player(0).playing);
    assert_ne!(w.phase(), Phase::Ended);
    w.set_disconnected(0);
    w.step(&c, &idle);
    assert_eq!(w.phase(), Phase::Ended);
}
