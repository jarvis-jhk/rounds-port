//! Full lobby + GGRS matches between several sessions in one process, over
//! in-memory rooms (no Warpzone server needed).

use rounds_net::toy::ToyMachine;
use rounds_net::{
    LinkSim, MatchSettings, MemHub, NetConfig, NetEvent, NetSession, SlotRequest, TeamMode,
};
use std::time::{Duration, Instant};

fn slots(m: usize, n: usize) -> Vec<SlotRequest> {
    (0..n)
        .map(|i| SlotRequest {
            extra: Vec::new(),
            name: format!("p{m}.{i}"),
            colour: (m * 3 + i) as u8,
            team: Some((i % 2) as u8),
        })
        .collect()
}

struct Run {
    rollbacks: u64,
    checksums: Vec<u128>,
}

/// `leave`: (machine, frame, crash) — crash = just stop polling (no goodbye).
fn run(locals: &[usize], link: LinkSim, target: i32, leave: &[(usize, i32, bool)]) -> Run {
    let hub = MemHub::new();
    let mut ms: Vec<ToyMachine> = locals
        .iter()
        .enumerate()
        .map(|(i, &n)| {
            let mut cfg = NetConfig::new("test");
            cfg.link_sim = link;
            let mut net = NetSession::memory(hub.join(&format!("m{i}")), cfg);
            net.set_local_slots(slots(i, n));
            ToyMachine::new(net, target, i as u64 + 7)
        })
        .collect();
    ms[0]
        .net
        .set_settings(MatchSettings {
            team_mode: TeamMode::Teams(2),
            game: vec![1, 2, 3],
        })
        .unwrap();
    let t0 = Instant::now();
    let mut started = false;
    let mut next = Instant::now();
    let mut left = vec![false; ms.len()];
    loop {
        assert!(t0.elapsed() < Duration::from_secs(120), "timeout");
        for (i, m) in ms.iter_mut().enumerate() {
            if left[i] {
                continue;
            }
            for e in m.poll() {
                if let NetEvent::Desync { .. } = e {
                    panic!("machine {i}: {e:?}");
                }
            }
        }
        if !started
            && ms[0].net.lobby().machines.len() == ms.len()
            && ms[1..].iter().all(|m| m.net.me().is_some())
        {
            let start = ms[0].net.start_match(Some(1234)).unwrap();
            assert_eq!(start.players.len(), locals.iter().sum::<usize>().min(12));
            started = true;
        }
        if Instant::now() >= next {
            next += Duration::from_millis(14);
            for (i, m) in ms.iter_mut().enumerate() {
                if left[i] {
                    continue;
                }
                m.step();
                for &(who, at, crash) in leave {
                    if who == i && !left[i] && m.net.current_frame() >= at {
                        if !crash {
                            m.net.leave();
                        }
                        left[i] = true;
                    }
                }
            }
        }
        if started
            && ms
                .iter()
                .enumerate()
                .all(|(i, m)| left[i] || m.result.is_some())
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let mut checksums = Vec::new();
    let mut rollbacks = 0;
    for (i, m) in ms.iter().enumerate() {
        if left[i] {
            continue;
        }
        let (reports, agreed) = m.result.clone().unwrap();
        assert!(agreed, "reports disagree: {reports:?}");
        assert_eq!(
            reports.len(),
            ms.len() - left.iter().filter(|l| **l).count()
        );
        checksums.push(m.target_checksum().unwrap());
        rollbacks += m.net.stats().rollbacks;
    }
    assert!(checksums.windows(2).all(|w| w[0] == w[1]));
    Run {
        rollbacks,
        checksums,
    }
}

#[test]
fn three_machines_mixed_locals() {
    let r = run(&[2, 1, 3], LinkSim::default(), 300, &[]);
    assert_eq!(r.checksums.len(), 3);
}

#[test]
fn rollback_under_latency_and_loss() {
    let r = run(
        &[3, 3, 3, 3],
        LinkSim {
            delay_ms: 40,
            jitter_ms: 15,
            loss: 0.05,
        },
        400,
        &[],
    );
    assert!(r.rollbacks > 0, "latency should cause rollbacks");
}

#[test]
fn machine_leaves_mid_match() {
    let r = run(
        &[1, 2, 1],
        LinkSim {
            delay_ms: 20,
            jitter_ms: 5,
            loss: 0.0,
        },
        500,
        &[(2, 150, false)],
    );
    assert_eq!(r.checksums.len(), 2);
}

#[test]
fn twelve_local_one_machine() {
    let r = run(&[12], LinkSim::default(), 200, &[]);
    assert_eq!(r.checksums.len(), 1);
}

#[test]
fn machines_crash_mid_match() {
    let r = run(
        &[2, 1, 3, 1, 2],
        LinkSim {
            delay_ms: 30,
            jitter_ms: 10,
            loss: 0.03,
        },
        700,
        &[(1, 150, true), (3, 300, true), (4, 310, false)],
    );
    assert_eq!(r.checksums.len(), 2);
}
