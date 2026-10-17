//! Match flow (match-flow.md): picks, points, rounds, game over.
mod common;
use common::*;
use rounds_sim::flow::Phase;
use rounds_sim::{EventKind, MatchSetup, PlayerInput, World};

/// Press jump every other tick (confirms picks), nothing else.
fn picker_inputs(frame: u32, n: usize) -> Vec<PlayerInput> {
    let b = if frame.is_multiple_of(2) {
        PlayerInput::BTN_JUMP
    } else {
        0
    };
    vec![input(0.0, 0.0, b); n]
}

fn run_until(
    w: &mut World,
    c: &rounds_sim::Content,
    max: u32,
    mut f: impl FnMut(&World) -> bool,
) -> u32 {
    for k in 0..max {
        let inp = picker_inputs(w.frame(), w.players().len());
        w.step(c, &inp);
        if f(w) {
            return k;
        }
    }
    panic!(
        "condition not reached in {max} ticks; phase {:?}",
        w.phase()
    );
}

#[test]
fn start_sequence_picks_then_battle() {
    let c = content();
    let mut w = World::new(&c, &MatchSetup::ffa(2, 3));
    let mut picked = 0;
    run_until(&mut w, &c, 2000, |w| {
        picked += w
            .events()
            .iter()
            .filter(|e| matches!(e.kind, EventKind::CardPicked { .. }))
            .count();
        w.phase() == Phase::Battle
    });
    assert_eq!(picked, 2, "every player picks once before round 1");
    assert!(w.players().iter().all(|p| p.cards.len() == 1));
    // Players fly in and become playable.
    run_until(&mut w, &c, 200, |w| w.players().iter().all(|p| p.playing));
    for p in w.players() {
        assert!(p.pos.y < 0.0, "on the map: {:?}", p.pos);
    }
}

fn kill(w: &mut World, c: &rounds_sim::Content, i: usize) {
    let pos = w.player(i).pos;
    w.take_damage(c, i, rounds_sim::v2(0.0, 1000.0), pos, None, true, true);
}

#[test]
fn points_rounds_and_game_over() {
    let c = content();
    let mut s = MatchSetup::ffa(2, 11);
    s.rounds_to_win_game = 2;
    let mut w = World::new(&c, &s);
    let mut loser_picks = 0;
    for round in 0..4 {
        run_until(&mut w, &c, 4000, |w| {
            w.phase() == Phase::Battle && w.players().iter().all(|p| p.playing)
        });
        let before: Vec<usize> = w.players().iter().map(|p| p.cards.len()).collect();
        kill(&mut w, &c, 1);
        w.step(&c, &picker_inputs(w.frame(), 2));
        match w.phase() {
            Phase::PointOver { winner, .. }
            | Phase::RoundOver { winner, .. }
            | Phase::GameOver { winner, .. } => assert_eq!(winner, 0),
            p => panic!("round {round}: unexpected phase {p:?}"),
        }
        if let Phase::GameOver { .. } = w.phase() {
            assert_eq!(w.rounds()[0], 2);
            run_until(&mut w, &c, 400, |w| {
                matches!(w.phase(), Phase::Rematch { .. })
            });
            assert_eq!(loser_picks, 1, "the round loser picked once");
            return;
        }
        if let Phase::RoundOver { .. } = w.phase() {
            assert_eq!(w.points(), &[0, 0]);
            run_until(&mut w, &c, 4000, |w| w.phase() == Phase::Battle);
            let after: Vec<usize> = w.players().iter().map(|p| p.cards.len()).collect();
            assert_eq!(after[0], before[0], "winner picks nothing");
            assert_eq!(after[1], before[1] + 1, "loser picks one card");
            loser_picks += 1;
        }
    }
    panic!("no game over");
}

#[test]
fn twelve_player_ffa_spawns_distinct() {
    let c = content();
    let mut w = World::new(&c, &MatchSetup::ffa(12, 5));
    run_until(&mut w, &c, 6000, |w| {
        w.phase() == Phase::Battle && w.players().iter().all(|p| p.playing)
    });
    let ps: Vec<_> = w.players().iter().map(|p| p.pos).collect();
    for a in 0..12 {
        for b in (a + 1)..12 {
            assert!(
                (ps[a] - ps[b]).len() > 0.5,
                "players {a} and {b} overlap at {:?}",
                ps[a]
            );
        }
    }
}

// ---- Simultaneous card picks (match-flow.md 16, PORT DECISION) ----

use rounds_sim::{PickMode, PickStage};

fn press(b: u16) -> PlayerInput {
    input(0.0, 0.0, b)
}

/// Steps idle until the phase is a simultaneous `Picking` and
/// every hand is choosing.
fn to_simultaneous_choosing(w: &mut World, c: &rounds_sim::Content) {
    let n = w.players().len();
    for _ in 0..2000 {
        w.step(c, &vec![PlayerInput::default(); n]);
        if w.simultaneous_pick()
            && !w.pick_hands().is_empty()
            && w.pick_hands()
                .iter()
                .all(|h| h.state.stage == PickStage::Choosing)
        {
            return;
        }
    }
    panic!("no simultaneous choosing; phase {:?}", w.phase());
}

#[test]
fn simultaneous_picks_deal_every_hand_at_once() {
    let c = content();
    let mut w = World::new(&c, &MatchSetup::ffa(6, 21));
    assert_eq!(w.pick_mode(), PickMode::Auto);
    to_simultaneous_choosing(&mut w, &c);
    let hands = w.pick_hands().to_vec();
    assert_eq!(hands.len(), 6);
    for (k, h) in hands.iter().enumerate() {
        assert_eq!(h.state.player as usize, k);
        assert_eq!(h.state.hand.len(), 5);
        assert!(!h.done && h.picked.is_none());
    }
    // `pick()` stays usable: the first hand that is not done.
    assert_eq!(w.pick().map(|p| p.player), Some(0));
    // Independent cursors: player k moves right k times (one edge per two
    // ticks), then everybody confirms on the same tick.
    let n = 6;
    for step in 0..10u32 {
        let inp: Vec<PlayerInput> = (0..n)
            .map(|k| {
                if step % 2 == 0 && (step / 2) < k as u32 {
                    press(PlayerInput::BTN_MENU_RIGHT)
                } else {
                    PlayerInput::default()
                }
            })
            .collect();
        w.step(&c, &inp);
    }
    let start = hands[0].selected as usize;
    for (k, h) in w.pick_hands().iter().enumerate() {
        assert_eq!(h.selected as usize, (start + k).min(4), "cursor of {k}");
    }
    w.step(&c, &vec![press(PlayerInput::BTN_JUMP); n]);
    let picked: Vec<(u8, rounds_sim::CardId, u8)> = w
        .events()
        .iter()
        .filter_map(|e| match e.kind {
            EventKind::CardPicked { player, card, slot } => Some((player, card, slot)),
            _ => None,
        })
        .collect();
    assert_eq!(picked.len(), 6, "all confirm on one tick");
    for (k, (player, card, slot)) in picked.iter().enumerate() {
        assert_eq!(*player as usize, k);
        assert_eq!(*slot as usize, (start + k).min(4));
        assert_eq!(*card, hands[k].state.hand[*slot as usize]);
        assert_eq!(w.player(k).cards, vec![*card]);
        assert_eq!(w.pick_hand(k).unwrap().picked, Some(*slot));
    }
    // End animation 0.9 s + 0.1 s gap, then the battle starts.
    let mut ticks_to_resume = 0;
    while matches!(w.phase(), Phase::Picking { .. }) {
        w.step(&c, &vec![PlayerInput::default(); n]);
        ticks_to_resume += 1;
        assert!(ticks_to_resume < 200);
    }
    assert!(w.pick_hands().is_empty() && !w.simultaneous_pick());
    assert!((70..=75).contains(&ticks_to_resume), "{ticks_to_resume}");
}

#[test]
fn simultaneous_hands_are_deterministic_and_input_independent() {
    let c = content();
    let hands_of = |nav: bool| {
        let mut w = World::new(&c, &MatchSetup::ffa(5, 33));
        w.set_pick_mode(PickMode::Simultaneous);
        for f in 0..400u32 {
            let mut inp = vec![PlayerInput::default(); 5];
            if nav && f % 2 == 0 {
                inp[0] = press(PlayerInput::BTN_MENU_RIGHT);
            }
            w.step(&c, &inp);
            if w.pick_hands().len() == 5 && w.pick_hands().iter().all(|h| h.state.hand.len() == 5) {
                return w
                    .pick_hands()
                    .iter()
                    .map(|h| h.state.hand.clone())
                    .collect::<Vec<_>>();
            }
        }
        panic!("hands not dealt");
    };
    let a = hands_of(false);
    assert_eq!(a, hands_of(false));
    assert_eq!(a, hands_of(true), "navigation does not touch the draws");
}

#[test]
fn sequential_mode_keeps_the_queue() {
    let c = content();
    let mut w = World::new(&c, &MatchSetup::ffa(6, 21));
    w.set_pick_mode(PickMode::Sequential);
    let mut started = Vec::new();
    run_until(&mut w, &c, 6000, |w| {
        assert!(w.pick_hands().is_empty() && !w.simultaneous_pick());
        for e in w.events() {
            if let EventKind::PickStarted { player } = e.kind {
                started.push(player);
            }
        }
        w.phase() == Phase::Battle
    });
    assert_eq!(started, vec![0, 1, 2, 3, 4, 5]);
    assert!(w.players().iter().all(|p| p.cards.len() == 1));
}

#[test]
fn auto_mode_threshold() {
    let c = content();
    for (n, simultaneous) in [(3usize, false), (4, true)] {
        let mut w = World::new(&c, &MatchSetup::ffa(n, 5));
        run_until(&mut w, &c, 400, |w| {
            matches!(w.phase(), Phase::Picking { .. })
        });
        w.step(&c, &vec![PlayerInput::default(); n]);
        assert_eq!(w.simultaneous_pick(), simultaneous, "{n} pickers");
    }
}

#[test]
fn simultaneous_pick_skips_disconnected_player() {
    let c = content();
    let mut w = World::new(&c, &MatchSetup::ffa(5, 8));
    to_simultaneous_choosing(&mut w, &c);
    w.set_disconnected(2);
    w.step(&c, &[PlayerInput::default(); 5]);
    assert!(
        w.events()
            .iter()
            .any(|e| matches!(e.kind, EventKind::PickEnded { player: 2 }))
    );
    let mut k = 0;
    while matches!(w.phase(), Phase::Picking { .. }) {
        let b = if k % 2 == 0 { PlayerInput::BTN_JUMP } else { 0 };
        w.step(&c, &[press(b); 5]);
        k += 1;
        assert!(k < 300, "phase did not end");
    }
    assert!(w.player(2).cards.is_empty());
    for i in [0, 1, 3, 4] {
        assert_eq!(w.player(i).cards.len(), 1);
    }
}
