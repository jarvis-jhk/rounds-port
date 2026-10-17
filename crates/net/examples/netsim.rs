//! netsim: the toy game (bouncing dots) over real Warpzone rooms, one OS
//! process per machine, to test rounds-net end to end.
//!
//! ```sh
//! scripts/net/dev-server.sh &                       # local warpzone-server
//! cargo run -p rounds-net --example netsim -- run --locals 3,3,3,3 --frames 3000
//! cargo run -p rounds-net --example netsim -- run --locals 1x12 --delay 75 --jitter 25 --loss 5
//! cargo run -p rounds-net --example netsim -- run --locals 4x3 --relay
//! cargo run -p rounds-net --example netsim -- run --locals 1x4 --leave 2@900 --crash 3@1500
//! ```
//!
//! `run` starts machine 0 as host, waits for its room, then starts the
//! others (odd machines join by code, even ones through `--warpzone-join`,
//! like the launcher), and compares the checksums every machine reports for
//! the final frame. Each machine gets its own `WARPZONE_HOME` under
//! `scratch/net/homes/`. Exit status 0 = all equal.

use rounds_net::toy::ToyMachine;
use rounds_net::warpzone_core::client::{Event, HostOptions};
use rounds_net::warpzone_core::protocol::api::{Entry, Visibility};
use rounds_net::{LinkSim, NetConfig, NetEvent, NetSession, SlotRequest};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Warpzone events arrive boxed; unbox them for matching.
#[allow(clippy::large_enum_variant)]
enum Ev {
    Wz(Event),
    Net(NetEvent),
}

const STEP: Duration = Duration::from_micros(14_000);

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("run") => std::process::exit(orchestrate(&args)),
        Some("peer") => std::process::exit(peer(&args)),
        _ => {
            eprintln!(
                "usage: netsim run [--locals 3,3,3,3 | 4x3] [--frames N] [--delay MS] [--jitter MS] [--loss PCT] [--relay] [--leave M@F] [--crash M@F] [--tag NAME]"
            );
            std::process::exit(2);
        }
    }
}

fn parse_locals(s: &str) -> Vec<usize> {
    if let Some((m, n)) = s.split_once('x') {
        return vec![n.parse().expect("locals"); m.parse().expect("machines")];
    }
    s.split(',')
        .map(|x| x.trim().parse().expect("locals"))
        .collect()
}

fn parse_at(s: Option<String>) -> Option<(usize, i32)> {
    let s = s?;
    let (m, f) = s.split_once('@')?;
    Some((m.parse().ok()?, f.parse().ok()?))
}

fn root() -> PathBuf {
    std::env::var_os("ROUNDS_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
}

// ------------------------------------------------------------------ orchestrator

fn orchestrate(args: &[String]) -> i32 {
    let locals = parse_locals(&arg(args, "--locals").unwrap_or_else(|| "3,3,3,3".into()));
    let frames: i32 = arg(args, "--frames")
        .and_then(|v| v.parse().ok())
        .unwrap_or(3000);
    let tag = arg(args, "--tag").unwrap_or_else(|| "ns".into());
    let leave = parse_at(arg(args, "--leave"));
    let crash = parse_at(arg(args, "--crash"));
    let server =
        std::env::var("WARPZONE_SERVER").unwrap_or_else(|_| "http://127.0.0.1:2998".into());
    let exe = std::env::current_exe().expect("exe");
    let homes = root().join("scratch/net/homes");
    let mut passthrough: Vec<String> = Vec::new();
    for k in [
        "--delay",
        "--jitter",
        "--loss",
        "--input-delay",
        "--max-prediction",
    ] {
        if let Some(v) = arg(args, k) {
            passthrough.push(k.into());
            passthrough.push(v);
        }
    }
    if flag(args, "--relay") {
        passthrough.push("--relay".into());
    }
    println!(
        "netsim: {} machines, slots {:?}, {} frames, {}",
        locals.len(),
        locals,
        frames,
        passthrough.join(" ")
    );

    let (tx, rx) = mpsc::channel::<(usize, String)>();
    let mut children: Vec<Child> = Vec::new();
    let spawn = |i: usize, extra: Vec<String>, children: &mut Vec<Child>| {
        let home = homes.join(format!("{tag}-m{i}"));
        std::fs::create_dir_all(&home).expect("home");
        let mut c = Command::new(&exe);
        c.arg("peer").args([
            "--index",
            &i.to_string(),
            "--slots",
            &locals[i].to_string(),
            "--frames",
            &frames.to_string(),
        ]);
        c.args(&passthrough).args(extra);
        if leave.is_some_and(|(m, _)| m == i) {
            c.args(["--leave-at", &leave.unwrap().1.to_string()]);
        }
        if crash.is_some_and(|(m, _)| m == i) {
            c.args(["--crash-at", &crash.unwrap().1.to_string()]);
        }
        c.env("WARPZONE_HOME", &home)
            .env("WARPZONE_SERVER", &server)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        let mut child = c.spawn().expect("spawn peer");
        let out = child.stdout.take().unwrap();
        let tx = tx.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                let _ = tx.send((i, line));
            }
        });
        children.push(child);
    };
    spawn(
        0,
        vec!["--host".into(), "--expect".into(), locals.len().to_string()],
        &mut children,
    );
    let t0 = Instant::now();
    let (mut room, mut code) = (String::new(), String::new());
    while room.is_empty() {
        match rx.recv_timeout(Duration::from_secs(30)) {
            Ok((i, l)) => {
                println!("[m{i}] {l}");
                if let Some(rest) = l.strip_prefix("ROOM ") {
                    let mut it = rest.split_whitespace();
                    room = it.next().unwrap_or_default().into();
                    code = it.next().unwrap_or_default().into();
                }
            }
            Err(_) => {
                eprintln!(
                    "netsim: host did not open a room (is the server running? scripts/net/dev-server.sh)"
                );
                for c in &mut children {
                    let _ = c.kill();
                }
                return 1;
            }
        }
    }
    for i in 1..locals.len() {
        let how = if i % 2 == 1 && code != "-" {
            vec!["--code".into(), code.clone()]
        } else {
            vec!["--warpzone-join".into(), room.clone()]
        };
        spawn(i, how, &mut children);
    }
    drop(tx);
    let mut results: Vec<(usize, String)> = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(60) + STEP * frames as u32 * 2;
    while let Ok((i, l)) = rx.recv_timeout(
        deadline
            .saturating_duration_since(Instant::now())
            .max(Duration::from_millis(1)),
    ) {
        println!("[m{i}] {l}");
        if l.starts_with("RESULT") {
            results.push((i, l));
        }
        if Instant::now() > deadline {
            break;
        }
    }
    for c in &mut children {
        let _ = c.kill();
        let _ = c.wait();
    }
    let field = |l: &str, k: &str| {
        l.split_whitespace()
            .find_map(|w| w.strip_prefix(&format!("{k}=")).map(String::from))
            .unwrap_or_default()
    };
    let expected: Vec<usize> = (0..locals.len())
        .filter(|i| leave.map(|l| l.0) != Some(*i) && crash.map(|c| c.0) != Some(*i))
        .collect();
    let mut sums: Vec<String> = results.iter().map(|(_, l)| field(l, "checksum")).collect();
    sums.sort();
    sums.dedup();
    let got: Vec<usize> = {
        let mut v: Vec<usize> = results.iter().map(|r| r.0).collect();
        v.sort();
        v
    };
    let rollbacks: u64 = results
        .iter()
        .map(|(_, l)| field(l, "rollbacks").parse::<u64>().unwrap_or(0))
        .sum();
    let resim: u64 = results
        .iter()
        .map(|(_, l)| field(l, "resim").parse::<u64>().unwrap_or(0))
        .sum();
    let depth: u64 = results
        .iter()
        .map(|(_, l)| field(l, "maxdepth").parse::<u64>().unwrap_or(0))
        .max()
        .unwrap_or(0);
    let desyncs: u64 = results
        .iter()
        .map(|(_, l)| field(l, "desyncs").parse::<u64>().unwrap_or(0))
        .sum();
    let ok = got == expected && sums.len() == 1 && desyncs == 0;
    println!(
        "SUMMARY {} machines={} reported={} checksums_equal={} checksum={} rollbacks={} resim={} maxdepth={} desyncs={} time={:.1}s",
        if ok { "PASS" } else { "FAIL" },
        locals.len(),
        got.len(),
        sums.len() == 1,
        sums.first().cloned().unwrap_or_default(),
        rollbacks,
        resim,
        depth,
        desyncs,
        t0.elapsed().as_secs_f32()
    );
    if ok { 0 } else { 1 }
}

// ------------------------------------------------------------------ one machine

fn say(s: String) {
    println!("{s}");
}

fn peer(args: &[String]) -> i32 {
    let index: usize = arg(args, "--index")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let nslots: usize = arg(args, "--slots")
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    let frames: i32 = arg(args, "--frames")
        .and_then(|v| v.parse().ok())
        .unwrap_or(3000);
    let host = flag(args, "--host");
    let expect: usize = arg(args, "--expect")
        .and_then(|v| v.parse().ok())
        .unwrap_or(2);
    let leave_at: Option<i32> = arg(args, "--leave-at").and_then(|v| v.parse().ok());
    let crash_at: Option<i32> = arg(args, "--crash-at").and_then(|v| v.parse().ok());
    let num = |k: &str| {
        arg(args, k)
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(0.0)
    };

    let mut cfg = NetConfig::new("netsim-1");
    cfg.link_sim = LinkSim {
        delay_ms: num("--delay") as u32,
        jitter_ms: num("--jitter") as u32,
        loss: num("--loss") / 100.0,
    };
    cfg.force_relay = flag(args, "--relay");
    if let Some(d) = arg(args, "--input-delay").and_then(|v| v.parse().ok()) {
        cfg.ggrs.input_delay = d;
    }
    if let Some(d) = arg(args, "--max-prediction").and_then(|v| v.parse().ok()) {
        cfg.ggrs.max_prediction = d;
    }
    let home = std::env::var("WARPZONE_HOME").unwrap_or_default();
    let h = home
        .bytes()
        .fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    let mut name = format!("ns{index}x{:04x}", h & 0xffff);
    cfg.auto_register = Some(name.clone());
    let net: NetSession<_, _> = match NetSession::new(cfg) {
        Ok(n) => n,
        Err(e) => {
            say(format!("ERROR {e}"));
            return 1;
        }
    };
    let mut m = ToyMachine::new(net, frames, index as u64 * 1000 + 17);
    m.net.set_local_slots(
        (0..nslots)
            .map(|i| SlotRequest {
                extra: Vec::new(),
                name: format!("m{index}p{i}"),
                colour: (index * 2 + i) as u8,
                team: None,
            })
            .collect(),
    );
    m.net.set_ready(true);

    let t0 = Instant::now();
    let mut logged_in = false;
    let mut requested_room = false;
    let mut started = false;
    let mut next = Instant::now();
    let mut ended_at: Option<Instant> = None;
    let mut last_report = Instant::now();
    let mut match_t0 = Instant::now();
    loop {
        if t0.elapsed() > Duration::from_secs(60) + STEP * frames as u32 * 3 {
            say("ERROR timeout".into());
            return 1;
        }
        for e in m.poll() {
            let e = match e {
                NetEvent::Warpzone(b) => Ev::Wz(*b),
                other => Ev::Net(other),
            };
            match e {
                Ev::Wz(Event::LoggedIn {
                    account: Some(a), ..
                }) => {
                    say(format!("logged in as {}", a.username));
                    logged_in = true;
                }
                Ev::Wz(Event::Registered(a)) => {
                    say(format!("registered {}", a.username));
                    logged_in = true;
                }
                Ev::Wz(Event::Error { op, code, message }) => {
                    say(format!("warpzone error {op}/{code}: {message}"));
                    if op == "register" {
                        name = format!(
                            "{}{}",
                            &name[..name.len().min(12)],
                            t0.elapsed().as_nanos() % 1000
                        );
                        m.net.register(&name);
                    } else if op == "join" || op == "host" {
                        return 1;
                    }
                }
                Ev::Wz(Event::RoomHosted { room, code, .. }) => {
                    say(format!(
                        "ROOM {} {}",
                        room.id,
                        code.unwrap_or_else(|| "-".into())
                    ));
                }
                Ev::Wz(Event::PeerConnected { peer, relayed }) => say(format!(
                    "peer {peer} connected {}",
                    if relayed { "via relay" } else { "direct" }
                )),
                Ev::Wz(Event::PeerPathChanged { peer, relayed }) => say(format!(
                    "peer {peer} now {}",
                    if relayed { "relayed" } else { "direct" }
                )),
                Ev::Net(NetEvent::Welcome { machine }) => say(format!("welcome as {machine}")),
                Ev::Net(NetEvent::MatchStarted(ms)) => {
                    say(format!(
                        "match {} started: {} slots, seed {:#x}, local {:?}",
                        ms.match_id,
                        ms.players.len(),
                        ms.seed,
                        m.net.local_slots()
                    ));
                    next = Instant::now();
                    match_t0 = Instant::now();
                }
                Ev::Net(NetEvent::Running) => say(format!(
                    "running ({} ms after match start)",
                    match_t0.elapsed().as_millis()
                )),
                Ev::Net(NetEvent::MachineLeft { machine }) => {
                    say(format!("{machine} left the room"))
                }
                Ev::Net(NetEvent::MatchFrozen { gone }) => {
                    say(format!("frozen, dropping {gone:?}"))
                }
                Ev::Net(NetEvent::PlayersDropped {
                    machines,
                    slots,
                    from_frame,
                }) => say(format!(
                    "resumed at frame {from_frame} without {machines:?} (slots {slots:?})"
                )),
                Ev::Net(NetEvent::Disconnected { machine }) => {
                    say(format!("{machine} disconnected (ggrs)"))
                }
                Ev::Net(NetEvent::Desync { frame, machine, .. }) => {
                    say(format!("DESYNC at frame {frame} with {machine}"))
                }
                Ev::Net(NetEvent::MatchEnded {
                    agreed,
                    ref reports,
                    ..
                }) => {
                    say(format!(
                        "match ended, {} reports, agreed={agreed}",
                        reports.len()
                    ));
                    ended_at = Some(Instant::now());
                }
                Ev::Net(NetEvent::MatchAborted { reason } | NetEvent::RoomLeft { reason }) => {
                    if ended_at.is_none() {
                        say(format!("ERROR aborted: {reason}"));
                        return 1;
                    }
                }
                Ev::Net(NetEvent::Rejected { reason }) => {
                    say(format!("ERROR rejected: {reason}"));
                    return 1;
                }
                _ => {}
            }
        }
        if logged_in && !requested_room {
            requested_room = true;
            if host {
                m.net.host(HostOptions {
                    max_players: 12,
                    visibility: Visibility::Public,
                    entry: Entry::Open,
                    preset: Some("ffa".into()),
                    ..Default::default()
                });
            } else if let Some(c) = arg(args, "--code") {
                m.net.join_code(&c);
            } else if let Some(r) = arg(args, "--warpzone-join") {
                let action =
                    m.net
                        .handle_launch(&["netsim".to_string(), "--warpzone-join".into(), r]);
                say(format!("launch: {action:?}"));
            }
        }
        if host && !started && m.net.lobby().machines.len() >= expect {
            match m.net.start_match(None) {
                Ok(_) => started = true,
                Err(e) => say(format!("start: {e}")),
            }
        }
        if let Some(t) = ended_at {
            // Host: linger so the End message is retransmitted if needed.
            if !host || t.elapsed() > Duration::from_secs(2) {
                let s = m
                    .net
                    .last_match_stats()
                    .cloned()
                    .unwrap_or_else(|| m.net.stats());
                let relayed = s
                    .machines
                    .iter()
                    .filter(|x| x.relayed == Some(true))
                    .count();
                let rtt = s
                    .machines
                    .iter()
                    .filter_map(|x| x.ggrs.as_ref().map(|g| g.ping))
                    .max()
                    .unwrap_or(0);
                let agreed = m.result.as_ref().is_some_and(|r| r.1);
                say(format!(
                    "RESULT machine={} slots={nslots} frame={} checksum={:032x} agreed={agreed} rollbacks={} resim={} maxdepth={} stalls={} desyncs={} resumes={} relayed_links={relayed} sent={} fwd={} simdrop={} frag={}{}",
                    index,
                    frames,
                    m.target_checksum().unwrap_or(0),
                    s.rollbacks,
                    s.resimulated_frames,
                    s.max_rollback_depth,
                    s.prediction_stalls,
                    s.desyncs,
                    s.resumes,
                    s.transport.sent_datagrams,
                    s.transport.forwarded,
                    s.transport.sim_dropped,
                    s.transport.fragmented,
                    if rtt > 0 {
                        format!(" ping={rtt}")
                    } else {
                        String::new()
                    },
                ));
                return 0;
            }
        }
        let now = Instant::now();
        if now >= next {
            // Ahead of the others: run 10% slower until they catch up.
            let step = if m.net.frames_ahead() > 0 {
                STEP * 11 / 10
            } else {
                STEP
            };
            next += step;
            if now > next + STEP * 10 {
                next = now; // we were stalled (e.g. synchronising); don't sprint
            }
            m.step();
            let f = m.net.current_frame();
            if leave_at.is_some_and(|l| f >= l) {
                say(format!("leaving at frame {f}"));
                m.net.leave();
                let t = Instant::now();
                while t.elapsed() < Duration::from_millis(500) {
                    m.net.poll();
                    std::thread::sleep(Duration::from_millis(5));
                }
                return 0;
            }
            if crash_at.is_some_and(|c| f >= c) {
                say(format!("crashing at frame {f}"));
                std::process::exit(3);
            }
            if last_report.elapsed() > Duration::from_secs(10) && m.net.current_match().is_some() {
                last_report = Instant::now();
                let s = m.net.stats();
                let links: Vec<String> = s
                    .machines
                    .iter()
                    .map(|x| {
                        format!(
                            "{}:{}ms{}{}",
                            x.machine,
                            x.ggrs.as_ref().map(|g| g.ping).unwrap_or(0),
                            if x.relayed == Some(true) { "R" } else { "" },
                            if x.via_host { "H" } else { "" }
                        )
                    })
                    .collect();
                say(format!(
                    "frame {} confirmed {} ahead {} rollbacks {} stalls {} links [{}]",
                    s.current_frame,
                    s.confirmed_frame,
                    s.frames_ahead,
                    s.rollbacks,
                    s.prediction_stalls,
                    links.join(" ")
                ));
            }
        }
        let wait = next
            .saturating_duration_since(Instant::now())
            .min(Duration::from_millis(2));
        if !wait.is_zero() {
            std::thread::sleep(wait);
        }
    }
}
