# Online play (`rounds-net`)

rounds-port plays online with GGRS rollback sessions whose packets travel
over [Warpzone](https://github.com/jarvis-jhk/warpzone) peer channels.
`warpzone-core` is used directly from Rust (git dependency pinned to
`c967f5c1`); the game is a tier-2 Warpzone game
(`packaging/warpzone/rounds-port.toml`).

* Each process is a **machine** with 0..12 local player **slots**; a match
  has at most 12 slots in any mix (12 × 1, 1 × 12, 3 × 4, 2 + 4 + 6, ...).
* Sim step 0.014 s; GGRS is told 71 fps.
* The room host is authoritative for the lobby: it assigns machine ids,
  global slots, colours and teams, picks the seed, starts and ends matches,
  and decides when a machine has dropped out.

## Topology

Warpzone rooms are a **star**: a guest is connected only to the host
(directly after NAT punching, or through the Warpzone relay). GGRS is a full
mesh, so every GGRS datagram carries `dst`/`src` machine ids and the host
forwards guest-to-guest datagrams unchanged (it checks that `src` is the
sending guest). Consequences: guest-to-guest latency is two hops; the host's
upload carries `O(machines²)` streams (measured: about 900 datagrams/s of
forwarding for 4 machines; fine for 12 on a home connection, but a weak host
uplink hurts everyone).

## Channels and framing

All encodings are bincode 1 with varint integers and a 64 KiB limit.

| Channel | Mode | Content |
|---|---|---|
| 1 `CH_GGRS` | unreliable | `kind:u8 dst:u8 src:u8 tag:u16le` + one bincode `ggrs::Message`; kind 1 = whole, kind 2 = fragment (`msg_id:u16le idx:u8 count:u8` + part; reassembled, incomplete ones expire after 2 s). `tag` = `match_id << 4 | epoch`: datagrams of another match or session are dropped. |
| 2 `CH_LOBBY` | reliable, ordered | `LobbyMsg`, split into chunks of `more:u8` + ≤ 1099 bytes |

Inputs: the sim supplies any `bytemuck::Pod` type; on the wire it is its
raw bytes (`input::Wire<I>`, no length prefix), zero bytes = no input.
The game uses `rounds_sim::PlayerInput` (8 bytes: `move_x/move_y: i8`,
`buttons: u16`, `aim_x/aim_y: i16`); `rounds_net::PlayerInput` (6 bytes)
is only the toy game's.

## Lobby and match protocol

| Message | Direction | Meaning |
|---|---|---|
| `Hello {protocol, game_version, slots}` | guest → host | first message; mismatching versions get `Reject` and a kick (`PROTOCOL` = 2 since `extra`) |
| `SetSlots`, `SetReady` | guest → host | local players (`name`, colour request, team request), ready flag |
| `Welcome {machine}` | host → guest | our machine id (host = 0, then 1, 2, ... in join order, never reused in a room) |
| `State(LobbyState)` | host → all | phase, machines with their slots, settings; on every change |
| `Start(MatchStart)` | host → players | `match_id`, `seed`, settings (team mode + opaque game bytes), the slot table, GGRS parameters |
| `Done {match_id, frame, checksum}` | guest → host | the sim ended at confirmed `frame` |
| `End {match_id, reports}` | host → all | every playing machine reported (or dropped): drop the session; reports let everyone compare checksums |
| `Freeze {epoch, gone}` / `Frozen {epoch, frame}` / `Resume {epoch, frame, gone}` / `Dropped` | both | dropping machines mid-match, see below |
| `App {from, to, data}` | any → any via host | game data outside the sim (chat, menus) |

`SlotRequest`/`PlayerSlot` carry `extra: Vec<u8>` (≤ 64 bytes, game-defined
per-player data shown to everyone; the game puts the face there).

Slot assignment (`lobby::assign`, unit-tested): machines by id, each
machine's slots in its order, until 12 (the rest sit out; `accepted_slots`
says how many fit). A colour request is honoured if free, else the lowest
free colour (12 colours). Free-for-all: team = slot. `Teams(n)`: a valid
request is honoured, otherwise the smallest team (lowest index on ties); the
host can override with `set_team`.

Everybody builds the same GGRS P2P session from `MatchStart`: players =
slots, own slots `Local`, others `Remote(machine)`. Machines that join while
a match runs are marked `waiting` and play from the next `start_match`.

**Ending a match**: when the sim decides the match is over at a *confirmed*
frame, the game calls `finish_match(frame, checksum)` and keeps calling
`advance` (others may still need our inputs) until `MatchEnded` arrives.
Leaving the GGRS session earlier would make the others see us drop.

## Machines dropping out (freeze/resume)

GGRS 0.13 cannot drop a peer reliably with more than two machines: every
machine disconnects it at the last frame *it* received, the views then
converge on the minimum by gossip, and the resulting rollback can reach
behind inputs GGRS already discarded or behind its save window — it panics
(`input_queue.rs` / `sync_layer.rs` assertions; reproduced here with 4
machines and 2 % loss). It also panics when a player is disconnected while
its last input is exactly the previous frame. rounds-net therefore never
lets GGRS disconnect anyone (GGRS timeout 600 s) and does it itself:

1. The host notices a machine is gone: Warpzone `PeerLeft`, or no GGRS
   datagram from it for `disconnect_timeout_ms` (2 s; 10 s before the
   session is synchronised).
2. It sends `Freeze {epoch, gone}` to the others (and `Dropped` to the gone
   machine if it can still be reached). Every machine stops advancing and
   answers `Frozen {frame}`: the newest frame `r` whose inputs were all
   confirmed when it last simulated and whose successor state `r + 1` is
   saved (GGRS keeps `rollback_window + 1 = 25` states; we predict at most
   `max_prediction = 8` frames, so the frame is always still there).
3. When all answered (3 s timeout: the silent ones are dropped too, new
   epoch), the host sends `Resume {frame: min(r) + 1, gone}`.
4. Every machine loads its saved state of that frame and continues in a new
   GGRS session (new `tag`) in which the gone machines' slots are dummy local
   players of the host that always send zero. `advance` reports them as
   `InputStatus::Disconnected`; their real inputs count up to `frame - 1` on
   every machine. Frame numbers keep counting (`StateCell` hides the GGRS
   frame offset). The first `input_delay` frames of a new session have zero
   inputs for everybody (GGRS fills its input delay with defaults) — the same
   on every machine.

The game sees `MatchFrozen {gone}` (show "player left, resyncing…"), then
`PlayersDropped {machines, slots, from_frame}`, and must handle the `Load`
request that comes with the next `advance`. The game's online driver turns
the `Disconnected` inputs of the dropped slots into the sim's
`PlayerInput::BTN_FORFEIT` (`online/driver.rs` `gone_input`): from frame
`from_frame` on every machine sees the bit at the same frame, so
`World::set_disconnected` happens identically everywhere (match-flow.md 16;
the dropped players leave the match, picks skip them, their team stops
counting). Doing it through the input rather than calling
`set_disconnected` after the `Load` keeps rollbacks safe (a re-loaded state
that already has the removal is not changed twice).
A freeze costs about one round trip plus a GGRS re-synchronisation (a few
hundred ms). **The host leaving ends the room** (Warpzone has no host
migration): guests get `RoomLeft` / `MatchAborted`.

## Desync detection and statistics

GGRS desync detection is on (`desync_interval` = 10 frames): the checksum
the game passes to `StateCell::save` is compared between machines; a
mismatch gives `NetEvent::Desync`. `finish_match` checksums are compared by
the host (`MatchEnded {agreed}`).

`stats()` → `NetStats`: current/confirmed frame, frames ahead, rollbacks
(Load requests), re-simulated frames, deepest rollback, prediction stalls,
desyncs, resumes, transport counters (datagrams, bytes, fragments,
forwarded, dropped by the link simulator) and per machine: GGRS ping / kbps
/ frames behind, Warpzone RTT of our link to it, `relayed`, `via_host`.

## API for the game crate

```rust
use rounds_net::{LaunchAction, MatchSettings, NetConfig, NetEvent, NetSession, PlayerInput, Request, SlotRequest};
use rounds_net::warpzone_core::client::{Event, HostOptions};

let mut cfg = NetConfig::new(env!("CARGO_PKG_VERSION"));
cfg.auto_register = None;                     // or Some(name) after asking the player
let mut net: NetSession<PlayerInput, SimState> = NetSession::new(cfg)?;
match net.handle_launch(&std::env::args().collect::<Vec<_>>()) {
    LaunchAction::Joining { .. } => show_connecting(),
    LaunchAction::HostScreen => open_host_screen(), // then net.host(HostOptions {..})
    LaunchAction::None => {}
}
net.set_local_slots(vec![SlotRequest { name: "Ann".into(), colour: 0, team: None }; n]);
// host: net.set_settings(MatchSettings { team_mode, game: bytes }); net.start_match(None)
// browse: net.list_rooms(true) -> NetEvent::Warpzone(Event::RoomList(..)); net.join(id, None, None)
// code:   net.join_code("K7QM3X")

// every frame
net.poll();
while let Some(e) = net.next_event() {
    match e {
        NetEvent::MatchStarted(ms) => sim = SimState::new(&ms),   // seed, settings, slot table
        NetEvent::Warpzone(e) => match *e { Event::RoomHosted { code, invite, .. } => .., _ => {} },
        NetEvent::MatchEnded { .. } | NetEvent::MatchAborted { .. } => back_to_lobby(),
        _ => {}
    }
}
// every sim step (0.014 s; run ~10 % slower while net.frames_ahead() > 0)
for &slot in net.local_slots() { net.add_local_input(slot, read_pad(slot))?; }
for r in net.advance() {
    match r {
        Request::Save { cell, frame } => cell.save(frame, Some(sim.clone()), Some(sim.checksum())),
        Request::Load { cell, .. } => sim = cell.load().unwrap(),
        Request::Advance { inputs } => sim.step(&inputs),      // inputs[slot] = (input, status)
    }
}
if sim.over() && sim.frame <= net.confirmed_frame() { net.finish_match(sim.frame, sim.checksum())?; }
```

Other calls: `register`, `host`, `join`, `join_code`, `list_rooms`,
`set_room_metadata(key, value)` (host: extra room browser columns, e.g. `map`),
`answer_knock`, `kick`, `set_room_closed`, `leave`, `set_ready`,
`set_team`, `end_match` (host abort), `send_app`, `lobby()`, `me()`,
`is_host()`, `current_match()`, `is_running()`, `current_frame()`,
`confirmed_frame()`, `stats()`, `last_match_stats()`, `with_warpzone(|c| ..)`
(the raw `warpzone_core::client::Client`). Everything is single-threaded:
call it from the game thread (the Warpzone client runs its own HTTP threads).

`NetSession::memory(MemHub::join(name), cfg)` gives the same API over an
in-process room (no server) — used by the tests and usable for local
multi-session debugging.

## Game integration (`crates/game/src/online/`, `menu/online.rs`, `menu/lobby.rs`)

* **Session**: one `NetSession<rounds_sim::PlayerInput, rounds_sim::World>`
  per process (`online::Net`, shared as `Rc<RefCell<..>>` between the
  pages and the driver), created when the Online page opens. Its
  `game_version` is `<crate version>+<12 chars of the import's source
  fingerprint>`: machines with different game data are rejected instead of
  desyncing. `--username U` sets `auto_register`; otherwise the first
  `LoggedIn` without an account opens the **Choose a name** page
  (`register`).
* **Pages**: Online home (signed-in line, Host room, Browse rooms, Join by
  code, Back), Host (public/unlisted, entry open/code/approve, preset
  `ffa`/`teams`; `HostOptions.max_players` 12 machines), Browse
  (`list_rooms` every 4 s; columns room, players, mode, entry; code rooms
  ask for the code), Join by code (6 characters), Connecting (prints
  `ROOM <id> <code> <invite>` on `RoomHosted`, waits for `RoomJoined`,
  shows knocking/declined), then the **online lobby** = the lobby page with
  the session: this machine's joined players are its `set_local_slots`
  (name = Warpzone username, + index with several; colour; team request;
  face in `extra`); every other machine's players appear as cards
  (colours/teams from a local `lobby::assign` preview, ready = the
  machine's flag); our ready flag = all local players ready; only the host
  edits the settings (`MatchSettings { team_mode, game: NetRules JSON }`
  with `MatchRules` + sandbox flag) and starts 1.5 s after every machine
  with players is ready; knocks (entry "approve") pop up for the host
  (Enter / click accepts, Delete / click declines). `--warpzone-join
  <room|invite>` / `--warpzone-host` / an invite link open the Online page
  and act on it once signed in (`handle_launch`).
* **Match start**: `NetEvent::MatchStarted(ms)` → `online::setup_from_start`
  (identical everywhere: slot order = global slots, FFA team = slot and
  skin = assigned colour, teams = team id and skin `team colour + 12 *
  member index`, rules and map pool from `NetRules`, seed from the host),
  `World::new`, controls = our devices/bots for our slots, `Remote` for the
  others, faces from `extra`.
* **`OnlineDriver: MatchDriver`** (`online/driver.rs`): every 0.014 s
  tick: `poll`, events, `add_local_input` for our slots, `advance`, then the
  requests: `Save` = `World::clone` + `World::checksum` (u64 → u128) for
  GGRS desync detection, `Load` = restore (rollback; `Advance::
  rolled_back_to`), `Advance` = `World::step` with every slot's input;
  `Disconnected` slots get `gone_input(world, slot)` (zero, except
  confirming a card pick so a dropped picker never blocks the match - a
  pure function of the world, so every machine agrees). Every tenth tick is
  skipped while `frames_ahead() > 0` (≈ 10 % slower) and
  `WaitRecommendation` skips up to 8 ticks. Events go through
  `EventFilter` keyed by (frame, seq, kind discriminant): a correctly
  predicted re-simulation re-produces the same keys (no second sound or
  particle); looping sounds follow the world state (`retain_loops`), so a
  rolled-back object stops its loop. **End**: when the flow reaches `Ended`
  ("No" at the rematch prompt; "Yes" restarts inside the same session) at
  a frame ≤ `confirmed_frame()`, the driver calls `finish_match(frame,
  checksum of that frame)` and keeps advancing until `MatchEnded`; then
  the lobby shows again (rematch = everybody readies again, the host's
  next `start_match`). `MatchAborted` / `RoomLeft` end the match with a
  message. Pausing online does not stop the sim (local devices are muted).
  Leaving the match from the pause menu leaves the room.
* **Overlay** (`menu/overlay.rs`): bottom left, per remote machine name,
  GGRS ping (or Warpzone RTT) and DIRECT / RELAYED / VIA HOST; frame,
  confirmed, frames ahead, rollbacks (max depth), stalls, desyncs; centred
  "SYNCHRONISING" before `Running` and "PLAYER LEFT - RESYNCING" while
  frozen.
* **Confirmed snapshots**: `MatchDriver::request_snapshot(frame)` keeps
  the world of a frame (overwritten by re-simulation) and hands it out
  once that frame is confirmed; headless runs draw it (without the
  overlay) and save `<prefix>-f<frame>.png`, so every machine renders the
  same sim state (presentation state such as particles, trails and limb
  springs still differs slightly).

### Automated online test

`--online host|join` runs `menu::online::AutoPage` instead of the menus:
sign in (`--username`), host an unlisted room (code + invite) or join by
`--code` / `--room <invite>` (the launcher path), announce `--bots N`
bot players, ready; the host starts when `--expect P` players are in.
Bots play the match (`bot_rematch = no` ends it at the prompt); the
driver prints `RESULT machine= frame= checksum= winner= agreed= reports=
desyncs= rollbacks= resim= maxdepth= stalls= resumes= ping= relayed_links=
gone_slots=` on `MatchEnded`; exit 0 = agreed. `--leave-at F` /
`--crash-at F` drop out at sim frame F; `--shots` + `--shot-prefix` save
confirmed-frame screenshots.

```sh
~/.claude/skills/perf-mode/scripts/perf-mode run -r "online test" -- scripts/net/online-test.sh [12p 1p2 teams drop relay win]
ONLINE_TEST_WINDOWS="1 3" scripts/net/online-test.sh drop   # machines 1 and 3 run the Windows build
ONLINE_TEST_SHOTS=40,600 scripts/net/online-test.sh 1p2    # other screenshot frames
```

`ONLINE_TEST_WINDOWS` runs those machine indices with the Windows build
under Proton (`scripts/run-windows.sh`, needs `scripts/build-windows.sh`);
case `win` = 3 machines × 2 bots with machine 1 on Windows. Note that the
launch index is not the wire `MachineId` (machines get ids in join order;
the slower-starting Windows instance often joins last).

The script starts `scripts/net/dev-server.sh` on `ROUNDS_WZ_PORT`/
`ROUNDS_WZ_UDP_PORT` (default 3998/3479) unless one answers, runs each case
with `ROUNDS_NET_DELAY_MS`/`JITTER_MS`/`LOSS` per process and one
`WARPZONE_HOME` per machine under `scratch/online/<case>/`, and checks:
one RESULT per remaining machine, all `agreed=true`, `desyncs=0`, identical
frame/checksum/winner. Screenshots: `scratch/online/<case>/m<i>-f<frame>.png`.

Results (2026-10-17, release build, one laptop, real sim with bots, full
matches ending at the rematch prompt):

| Case | Machines × bots | Link | Rules | End frame | Result | Rollbacks / machine | Max depth |
|---|---|---|---|---|---|---|---|
| 12p | 3 × 4 = 12 | 40 ms ± 15, 3 % loss | FFA, 1 round | 2839 | PASS, same checksum + winner on 3 | ≈ 1800 | 8 |
| 1p2 | 1 + 2 | 60 ms ± 20, 5 % loss | FFA, 2 rounds | 6023 | PASS (2) | ≈ 3000 | 8 |
| teams | 3 × 2, 2 teams | 25 ms ± 10, 2 % loss | teams, 2 rounds | 4464 | PASS (3) | ≈ 2500 | 8 |
| drop | 4 × 2 | 30 ms ± 10, 2 % loss | FFA, 1 round of 2 points | 3763 | PASS (2 left; leave at 1200 during card picks, crash at 2000; 2 resumes) | ≈ 2200 | 8 |
| relay | 2 + 2 + 1 | forced relay, 20 ms ± 5, 1 % | FFA, 1 round | 2885 | PASS (3, all links relayed) | ≈ 1750 | 8 |

No desync in any case; confirmed-frame screenshots show the same scene on
every machine (pixel differences come from presentation-only state).

Cross-platform (2026-10-17, later, with forfeit-on-drop and the pick-mode
setting): `win` (Linux, **Windows/Proton**, Linux; 30 ms ± 10, 2 % loss)
PASS, end frame 1363, same checksum on all three, 0 desyncs; `drop` with
machines 1 and 3 on Windows (machine 2 leaves at 1200, machine 3 crashes
at 2000) PASS, the Linux host and the Windows survivor agree; `12p` PASS.
One earlier `win` run reported three transient desync checks on a Linux
guest (frames 1110-1130, then equal again; final results agreed); it did
not reproduce in three later runs (two with Windows).

## Link simulation

Outgoing GGRS datagrams can be delayed, jittered and dropped by the machine
that originates them (the host's forwarding adds nothing, so a
guest-to-guest path gets one delay): `NetConfig::link_sim` or
`ROUNDS_NET_DELAY_MS` (one way), `ROUNDS_NET_JITTER_MS` (uniform ±),
`ROUNDS_NET_LOSS` (percent). `ROUNDS_NET_FORCE_RELAY=1` routes every
Warpzone connection through the server relay.

## Testing

```sh
source env.sh
cargo test -p rounds-net                       # unit tests + in-process matches (memory rooms)
scripts/net/dev-server.sh                      # local warpzone-server, http://127.0.0.1:2998
#   ROUNDS_WZ_PORT=3998 ROUNDS_WZ_UDP_PORT=3479 scripts/net/dev-server.sh   if 2998 is taken
export WARPZONE_SERVER=http://127.0.0.1:2998
cargo run -p rounds-net --example netsim -- run --locals 4x3 --frames 3000
cargo run -p rounds-net --example netsim -- run --locals 12x1 --delay 75 --jitter 25 --loss 5
cargo run -p rounds-net --example netsim -- run --locals 4x3 --relay
cargo run -p rounds-net --example netsim -- run --locals 4x1 --leave 2@900 --crash 3@1500
~/.claude/skills/perf-mode/scripts/perf-mode run -r "net matrix" -- scripts/net/netsim-matrix.sh
```

`dev-server.sh` builds `warpzone-server` and `wz` from `ext/warpzone` into
`scratch/net/wz-target/`, writes its config and database to
`scratch/net/wz-dev/` (registration and login limits raised: tests create a
dozen identities from one IP) and serves `packaging/warpzone/` as its games
directory (validated with `warpzone-server check-games` on every start).

`netsim` runs the toy game of `rounds_net::toy` (up to 12 bouncing dots,
integer physics, bots that hold an input for 4–40 frames) with one OS
process per machine, each with its own `WARPZONE_HOME` under
`scratch/net/homes/`. Machine 0 hosts a public room; odd machines join by
code, even ones through `--warpzone-join <room>` (the launcher path). At the
target frame every machine reports its checksum through `finish_match`; the
orchestrator prints `SUMMARY PASS` when all remaining machines report the
same one and no desync was detected. `--leave M@F` leaves gracefully,
`--crash M@F` exits without a word.

### Results (2026-10-17, 3000 frames ≈ 43 s per case, one laptop)

| Machines × slots | Link | Checksums | Rollbacks (sum) | Max depth |
|---|---|---|---|---|
| 4 × 3 | none | PASS | 0 | 0 |
| 12 × 1 | none | PASS | 61 | 2 |
| 2 + 4 + 6 | none | PASS | 0 | 0 |
| 4 × 3 | 25 ms ± 10, 5 % loss (RTT ≈ 50) | PASS | 2028 | 6 |
| 4 × 3 | 75 ms ± 25, 5 % loss (RTT ≈ 150) | PASS | 3829 | 8 |
| 12 × 1 | 50 ms ± 20, 5 % loss (RTT ≈ 100) | PASS | 13207 | 8 |
| 4 × 3 | forced relay (all 3 links relayed) | PASS | 0 | 0 |
| 6 × 2 | forced relay + 50 ms ± 15, 5 % loss | PASS | 6320 | 8 |
| 4 × 1 | 30 ms ± 10, 2 % loss; m2 leaves @900, m3 crashes @1500 (2 resumes) | PASS (2 left) | 344 | 8 |
| 6 × 2 | 50 ms ± 15, 5 % loss; leave @800, crash @1600 (2 resumes) | PASS (4 left) | 3242 | 8 |

No desync event in any case. `cargo test -p rounds-net` additionally runs
in-process matches (2+1+3, 4 × 3 with 40 ms/5 % loss, 1 × 12, a graceful
leave, two crashes plus a leave in a 5-machine match).

Delays are one way per originating machine, so the round trip between two
guests is about twice the value plus the host hop.

## Open issues

* **No guest-to-guest links in Warpzone** (rooms are a star): GGRS traffic
  between guests is forwarded by the host. A mesh option in `warpzone-core`
  (peer info for all members, punching between guests) would halve
  guest-to-guest latency and remove the host's `O(n²)` forwarding load.
* **No host migration**: the host leaving closes the room and ends the
  match.
* **GGRS 0.13 drop handling** (see above) is worked around by freeze/resume;
  a GGRS release that drops peers at an agreed frame would make the
  workaround unnecessary.
* `Client::force_relay` is a no-op until the client has its UDP side (after
  the first `LoggedIn`); rounds-net re-applies it on `LoggedIn` and before
  host/join.
* Spectators (zero-slot machines watching) are not implemented; they get
  `WaitingForNextMatch`. The host must have at least one slot
  (`start_match` → `HostNotPlaying`): it coordinates drops and plays the
  dropped slots' dummies.
* The descriptor's `[[data]]` hands the ROUNDS install to the game as
  `WARPZONE_DATA_ROUNDS`; `rounds-import` should read it. The release asset
  names in `[install]`/`[launch]` are placeholders until packaging (M11).
