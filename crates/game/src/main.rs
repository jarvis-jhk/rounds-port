//! rounds-port: clean-room reimplementation of ROUNDS.
//!
//! Without arguments: the game (menus, local and online play). Developer
//! entry points: map viewer, renderer demo, bot matches, headless menu
//! screenshots, automated online runs; see `docs/platform.md`.

// Release builds for Windows are GUI programs (no console window pops up
// when started from Explorer); `console::attach` re-attaches to the parent
// console so `--help`, `--version` and logs still print from cmd/PowerShell.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod art;
mod audio;
mod boot;
mod crash;
mod data;
mod demo;
mod faces;
mod fx_preview;
mod input;
mod map_scene;
mod map_view;
mod menu;
mod online;
mod paths;
mod platform;
mod play;
mod psys;
mod settings;
mod sysfonts;
mod timing;
mod ui;
mod vpad;

use std::path::PathBuf;
use std::rc::Rc;

use anyhow::{Context, Result, bail};

const USAGE: &str = "\
rounds-port

USAGE:
    rounds-port [OPTIONS]                       (the game: menus, local and online play)
    rounds-port --headless --screen <NAME> --screenshot scratch/menu.png [--frames N]
    rounds-port --headless --online host|join [--code C | --room ID] --bots N [--expect P]
                [--rounds R] [--points P] [--teams T] [--no-picks] [--username U]
                [--shots F1,F2] [--shot-prefix scratch/x] [--leave-at F] [--crash-at F]
    rounds-port --map-viewer [--map N]          (windowed map viewer)
    rounds-port --headless --map <N|NAME> --screenshot scratch/map.png [--size 1920x1080]
    rounds-port --headless --map-sheet scratch/maps.png [--size 640x360]
    rounds-port --headless --frames 120 --screenshot scratch/demo.png   (renderer demo)
    rounds-port --headless --bot-match [--players N] [--map NAME] [--seed S] --frames F
                [--screenshot scratch/x.png] [--shots 100,400,900]
    rounds-port --play [--players N] [--keyboard-only]   (local match, dev entry point)

OPTIONS:
    --screen NAME          Start on a menu screen: main, local, options, controls, maps,
                           lobby, lobby12, lobby-teams, editor, register, online, host,
                           browse, code, onlinelobby, pause
    --online host|join     Headless automated online run with bot players (tests);
                           prints ROOM <id> <code> (host) and RESULT ... at the end
    --code C / --room ID   --online join: by code / through --warpzone-join ID
    --bots N               --online: local bot players of this machine (default 1)
    --expect P             --online host: start when the room has P players
    --rounds R, --points P, --teams T, --no-picks   --online host: match rules
    --username U           Register this Warpzone username if the identity has none
    --shot-prefix PATH     --online: screenshots of confirmed frames --shots as
                           PATH-f<frame>.png (identical on every machine)
    --leave-at F, --crash-at F   --online: leave / exit without a word at sim frame F
    --timeout S            --online: give up after S seconds (default 600)
    --warpzone-join ROOM / --warpzone-host   Launcher / invite link: open online
    --map-viewer           Windowed map viewer (Left/Right: map, Up/Down: art)
    --map N|NAME           Show a map: number 0..69 or scene name (e.g. Balance_M)
    --art N                Art (background/fill style) 0..8 (default: map number mod 9)
    --map-sheet PATH       Headless: render all maps into one contact sheet PNG
                           (--size is the size of one map)
    --bot-match            Headless: a match of bots (all slots bots)
    --play                 Start a local match: keyboard+mouse = player 1, each
                           controller another player, remaining slots bots
    --players N            Players in the match (1..12, default 2 / 4 bots)
    --keyboard-only        --play: add a keyboard-only player (arrows, I/J/K/L aim,
                           Right Shift jump, . fire, , block)
    --seed S               Match seed (default 1)
    --cards A,B            Debug: every player starts with these cards (names)
    --shots LIST           Headless: also save screenshots at these frames
                           (comma separated; files <screenshot>-<frame>.png)
    --demo                 Run the renderer demo scene (default when headless
                           without --map)
    --game DIR             ROUNDS install (folder with ROUNDS_Data)
    --data DIR             Use an existing import dir as is (no install needed)
    --import               Import the game data (if needed) and exit
    --headless             Render offscreen, no window, no audio device
    --frames N             Headless: frames to render (default 120, maps 30)
    --fps F                Headless: virtual frame rate (default 60)
    --screenshot PATH      Headless: save the last frame as PNG
    --wav PATH             Headless: write the mixed audio to a WAV file
    --size WxH             Target size (default 1920x1080 headless, 1280x720 window)
    --fullscreen           Start in borderless fullscreen
    --no-vsync             Disable vsync (use --fps-cap to limit)
    --fps-cap F            Frame cap without vsync (default 0 = none)
    --play-ogg PATH.ogg    Demo: play an Ogg Vorbis file on the music bus (looping)
    --list-devices         Print GPUs, controllers and audio devices, then exit
    --pick-mode M          Card picks: auto (together with > 3 pickers), turn (the
                           original, one after another), together (--bot-match,
                           --online host)
    --fx-preview NAMES     Headless dev scene: imported prefab particle effects by
                           root name (A_Explosion,PlayerDeath,...; all = every one)
    --virtual-pads FILE    Dev/tests: simulated controllers driven by commands
                           appended to FILE (attach N, detach P, tap P a, axis P lx 1,
                           button P x 1; see crates/game/src/vpad.rs)
    -V, --version          Print the version and exit
    -h, --help             Show this help

ENVIRONMENT:
    ROUNDS_GAME_DIR=DIR           ROUNDS install (after --game)
    WARPZONE_DATA_ROUNDS=DIR      ROUNDS install (set by the Warpzone launcher)
    ROUNDS_PORT_DATA_DIR=DIR      User data dir (import cache, settings)
    WGPU_BACKEND=vulkan|dx12|gl   Force a graphics backend
    RUST_LOG=info|debug           Log level
";

#[derive(Debug, Default)]
struct Args {
    headless: bool,
    frames: Option<u32>,
    fps: Option<f64>,
    screenshot: Option<PathBuf>,
    wav: Option<PathBuf>,
    size: Option<(u32, u32)>,
    fullscreen: bool,
    no_vsync: bool,
    fps_cap: Option<f64>,
    play: Option<PathBuf>,
    list_devices: bool,
    map: Option<String>,
    art: Option<usize>,
    map_sheet: Option<PathBuf>,
    demo: bool,
    game: Option<PathBuf>,
    data: Option<PathBuf>,
    import: bool,
    bot_match: bool,
    play_match: bool,
    players: Option<usize>,
    keyboard_only: bool,
    seed: Option<u64>,
    shots: Vec<u32>,
    cards: Vec<String>,
    screen: Option<String>,
    online: Option<String>,
    code: Option<String>,
    room: Option<String>,
    bots: Option<usize>,
    expect: Option<usize>,
    rounds: Option<u8>,
    points: Option<u8>,
    teams: Option<u8>,
    no_picks: bool,
    username: Option<String>,
    shot_prefix: Option<PathBuf>,
    leave_at: Option<u32>,
    crash_at: Option<u32>,
    timeout: Option<u64>,
    map_viewer: bool,
    launch: Vec<String>,
    virtual_pads: Option<PathBuf>,
    version: bool,
    /// Hidden: panic on purpose (tests the crash report path).
    crash_test: bool,
    fx_preview: Option<String>,
    pick_mode: Option<settings::PickModeSetting>,
}

fn parse_size(s: &str) -> Result<(u32, u32)> {
    let (w, h) = s.split_once(['x', 'X']).context("size must be WxH")?;
    let (w, h): (u32, u32) = (w.parse()?, h.parse()?);
    if w == 0 || h == 0 || w > 16384 || h > 16384 {
        bail!("size out of range: {s}");
    }
    Ok((w, h))
}

fn parse_args(mut it: impl Iterator<Item = String>) -> Result<Option<Args>> {
    let mut a = Args::default();
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().with_context(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--headless" => a.headless = true,
            "--frames" => a.frames = Some(value("--frames")?.parse()?),
            "--fps" => a.fps = Some(value("--fps")?.parse()?),
            "--screenshot" => a.screenshot = Some(value("--screenshot")?.into()),
            "--wav" => a.wav = Some(value("--wav")?.into()),
            "--size" => a.size = Some(parse_size(&value("--size")?)?),
            "--fullscreen" => a.fullscreen = true,
            "--no-vsync" => a.no_vsync = true,
            "--fps-cap" => a.fps_cap = Some(value("--fps-cap")?.parse()?),
            "--play-ogg" => a.play = Some(value("--play-ogg")?.into()),
            "--list-devices" => a.list_devices = true,
            "--crash-test" => a.crash_test = true,
            "--map" => a.map = Some(value("--map")?),
            "--art" => a.art = Some(value("--art")?.parse()?),
            "--map-sheet" => a.map_sheet = Some(value("--map-sheet")?.into()),
            "--demo" => a.demo = true,
            "--game" => a.game = Some(value("--game")?.into()),
            "--data" => a.data = Some(value("--data")?.into()),
            "--import" => a.import = true,
            "--bot-match" => a.bot_match = true,
            "--cards" => {
                a.cards = value("--cards")?
                    .split(',')
                    .map(|x| x.trim().to_owned())
                    .filter(|x| !x.is_empty())
                    .collect()
            }
            "--play" => a.play_match = true,
            "--players" => a.players = Some(value("--players")?.parse()?),
            "--keyboard-only" => a.keyboard_only = true,
            "--seed" => a.seed = Some(value("--seed")?.parse()?),
            "--shots" => {
                a.shots = value("--shots")?
                    .split(',')
                    .filter(|x| !x.trim().is_empty())
                    .map(|x| x.trim().parse())
                    .collect::<Result<_, _>>()?
            }
            "--screen" => a.screen = Some(value("--screen")?),
            "--online" => a.online = Some(value("--online")?),
            "--code" => a.code = Some(value("--code")?.to_uppercase()),
            "--room" => a.room = Some(value("--room")?),
            "--bots" | "--bot" => a.bots = Some(value("--bots")?.parse()?),
            "--expect" => a.expect = Some(value("--expect")?.parse()?),
            "--rounds" => a.rounds = Some(value("--rounds")?.parse()?),
            "--points" => a.points = Some(value("--points")?.parse()?),
            "--teams" => a.teams = Some(value("--teams")?.parse()?),
            "--no-picks" => a.no_picks = true,
            "--username" => a.username = Some(value("--username")?),
            "--shot-prefix" => a.shot_prefix = Some(value("--shot-prefix")?.into()),
            "--leave-at" => a.leave_at = Some(value("--leave-at")?.parse()?),
            "--crash-at" => a.crash_at = Some(value("--crash-at")?.parse()?),
            "--timeout" => a.timeout = Some(value("--timeout")?.parse()?),
            "--map-viewer" => a.map_viewer = true,
            "--pick-mode" => {
                use settings::PickModeSetting as M;
                a.pick_mode = Some(match value("--pick-mode")?.as_str() {
                    "auto" => M::Auto,
                    "turn" | "sequential" => M::Sequential,
                    "together" | "simultaneous" => M::Simultaneous,
                    other => bail!("--pick-mode expects auto, turn or together, not {other:?}"),
                })
            }
            "--fx-preview" => a.fx_preview = Some(value("--fx-preview")?),
            "--virtual-pads" => a.virtual_pads = Some(value("--virtual-pads")?.into()),
            "-V" | "--version" => a.version = true,
            "--warpzone-join" => {
                let r = value("--warpzone-join")?;
                a.launch.extend(["--warpzone-join".to_string(), r]);
            }
            "--warpzone-host" => a.launch.push("--warpzone-host".into()),
            x if x.starts_with("warpzone://") || x.starts_with("https://") => {
                a.launch.push(x.into())
            }
            "-h" | "--help" => return Ok(None),
            other => bail!("unknown argument {other:?} (see --help)"),
        }
    }
    Ok(Some(a))
}

fn init_logging() {
    struct Logger(log::LevelFilter);
    impl log::Log for Logger {
        fn enabled(&self, m: &log::Metadata) -> bool {
            m.level() <= self.0
        }
        fn log(&self, r: &log::Record) {
            // Keep wgpu/naga internals quiet unless asked for.
            let noisy = r.target().starts_with("wgpu") || r.target().starts_with("naga");
            if self.enabled(r.metadata())
                && (!noisy || r.level() <= log::Level::Warn || self.0 >= log::LevelFilter::Debug)
            {
                eprintln!("[{:5}] {}", r.level(), r.args());
            }
        }
        fn flush(&self) {}
    }
    let level = match std::env::var("RUST_LOG").as_deref() {
        Ok("trace") => log::LevelFilter::Trace,
        Ok("debug") => log::LevelFilter::Debug,
        Ok("info") => log::LevelFilter::Info,
        Ok("error") => log::LevelFilter::Error,
        _ => log::LevelFilter::Warn,
    };
    if log::set_logger(Box::leak(Box::new(Logger(level)))).is_ok() {
        log::set_max_level(level);
    }
}

/// `--list-devices`: GPUs (wgpu), controllers and audio (SDL, no window).
fn list_devices() -> Result<()> {
    let instance = rounds_render::new_instance();
    let adapters =
        pollster::block_on(instance.enumerate_adapters(rounds_render::wgpu::Backends::all()));
    println!("GPU adapters ({}):", adapters.len());
    for a in &adapters {
        let i = a.get_info();
        println!(
            "  {} [{:?}, {:?}] driver: {} {}",
            i.name, i.backend, i.device_type, i.driver, i.driver_info
        );
    }

    platform::set_hints();
    let sdl = sdl2::init().map_err(anyhow::Error::msg)?;
    let v = sdl2::version::version();
    println!("SDL {}.{}.{}", v.major, v.minor, v.patch);
    let gc = sdl.game_controller().map_err(anyhow::Error::msg)?;
    // Let SDL deliver the initial device-added events (hot-plug path).
    let mut pump = sdl.event_pump().map_err(anyhow::Error::msg)?;
    let mut hub = input::InputHub::new(gc.clone());
    for _ in 0..10 {
        for e in pump.poll_iter() {
            hub.handle_event(&e);
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let n = gc.num_joysticks().map_err(anyhow::Error::msg)?;
    let controllers = hub.controllers();
    println!(
        "Controllers ({} game controllers, {} joysticks):",
        controllers.len(),
        n
    );
    for c in &controllers {
        println!(
            "  {:?} {} (rumble: {})",
            c.id,
            c.name,
            if c.has_rumble { "yes" } else { "no" }
        );
    }
    for i in 0..n {
        if !gc.is_game_controller(i) {
            let joy = sdl.joystick().map_err(anyhow::Error::msg)?;
            let name = joy.name_for_index(i).unwrap_or_else(|_| "?".into());
            println!(
                "  joystick #{i} {name} (no controller mapping; add one to gamecontrollerdb.txt)"
            );
        }
    }

    match sdl.audio() {
        Ok(audio) => {
            println!("Audio driver: {}", audio.current_audio_driver());
            let n = audio.num_audio_playback_devices().unwrap_or(0);
            println!("Audio playback devices ({n}):");
            for i in 0..n {
                let name = audio.audio_playback_device_name(i).unwrap_or_else(|e| e);
                println!("  #{i} {name}");
            }
        }
        Err(e) => println!("Audio: unavailable ({e})"),
    }
    let drivers: Vec<_> = sdl2::audio::drivers().collect();
    println!("Audio drivers compiled into SDL: {}", drivers.join(", "));
    Ok(())
}

/// Windows GUI-subsystem builds: attach to the console of the process that
/// started us (if any) and point stdout/stderr at it unless they were
/// redirected (pipes and files are inherited and stay as they are).
#[cfg(all(windows, not(debug_assertions)))]
mod console {
    use std::ffi::c_void;
    type Handle = *mut c_void;
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
    const STD_ERROR_HANDLE: u32 = -12i32 as u32;
    const INVALID: Handle = -1isize as Handle;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn AttachConsole(pid: u32) -> i32;
        fn GetStdHandle(which: u32) -> Handle;
        fn SetStdHandle(which: u32, h: Handle) -> i32;
        fn CreateFileW(
            name: *const u16,
            access: u32,
            share: u32,
            security: *mut c_void,
            disposition: u32,
            flags: u32,
            template: Handle,
        ) -> Handle;
    }

    pub fn attach() {
        // SAFETY: plain Win32 calls with valid arguments; handles we open
        // live for the rest of the process.
        unsafe {
            let missing = |w| {
                let h = GetStdHandle(w);
                h.is_null() || h == INVALID
            };
            if !(missing(STD_OUTPUT_HANDLE) || missing(STD_ERROR_HANDLE)) {
                return; // redirected: keep the inherited handles
            }
            if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
                return; // started from Explorer / a launcher: no console
            }
            let name: Vec<u16> = "CONOUT$\0".encode_utf16().collect();
            let h = CreateFileW(
                name.as_ptr(),
                0xC000_0000, // GENERIC_READ | GENERIC_WRITE
                3,           // FILE_SHARE_READ | FILE_SHARE_WRITE
                std::ptr::null_mut(),
                3, // OPEN_EXISTING
                0,
                std::ptr::null_mut(),
            );
            if h.is_null() || h == INVALID {
                return;
            }
            for w in [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
                if missing(w) {
                    SetStdHandle(w, h);
                }
            }
            // The shell already printed its prompt; start on a fresh line.
            eprintln!();
        }
    }
}

fn main() -> Result<()> {
    #[cfg(all(windows, not(debug_assertions)))]
    console::attach();
    init_logging();
    crash::install();
    let Some(args) = parse_args(std::env::args().skip(1))? else {
        print!("{USAGE}");
        return Ok(());
    };
    if args.version {
        println!("rounds-port {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.crash_test {
        panic!("crash test requested with --crash-test");
    }
    if args.list_devices {
        return list_devices();
    }
    let data_opts = data::DataOptions {
        game: args.game.clone(),
        import_dir: args.data.clone(),
    };
    if args.import {
        let plan = data::prepare(&data_opts)?;
        if let Some(i) = &plan.install {
            println!("ROUNDS install: {} ({})", i.dir.display(), i.source);
        }
        data::run_import(&plan)?;
        let d = data::load(&plan.import_dir)?;
        println!(
            "{}: {} maps, {} cards",
            plan.import_dir.display(),
            d.maps.len(),
            d.cards.len()
        );
        return Ok(());
    }
    if let Some(out) = &args.map_sheet {
        let d = Rc::new(data::prepare_and_load(&data_opts)?);
        return map_scene::map_sheet(d, args.size.unwrap_or((640, 360)), args.art, out);
    }
    let headless_opts = |frames: u32| app::HeadlessOptions {
        size: args.size.unwrap_or((1920, 1080)),
        frames: args.frames.unwrap_or(frames),
        fps: args.fps.unwrap_or(60.0),
        screenshot: args.screenshot.clone(),
        wav: args.wav.clone(),
        shots: args.shots.clone(),
        render_all: !args.bot_match,
        realtime: false,
    };
    let window_opts = app::WindowOptions {
        size: args.size.unwrap_or((1280, 720)),
        fullscreen: args.fullscreen,
        vsync: !args.no_vsync,
        fps_cap: args.fps_cap.unwrap_or(0.0),
        virtual_pads: args.virtual_pads.clone(),
    };
    if let Some(names) = &args.fx_preview {
        let d = Rc::new(data::prepare_and_load(&data_opts)?);
        let mut scene = fx_preview::FxPreview::new(d, names);
        return app::run_headless(&mut scene, &headless_opts(120));
    }
    if args.bot_match || args.play_match {
        return run_match(&args, &data_opts, headless_opts, window_opts);
    }
    let game_mode = args.screen.is_some()
        || args.online.is_some()
        || (!args.headless && !args.demo && !args.map_viewer && args.map.is_none());
    if game_mode {
        return run_game(&args, &data_opts, headless_opts, window_opts);
    }
    let demo = args.demo || (args.headless && args.map.is_none());
    if demo {
        let mut scene = demo::Demo::new(args.play.clone());
        return if args.headless {
            app::run_headless(&mut scene, &headless_opts(120))
        } else {
            app::run_windowed(&mut scene, &window_opts)
        };
    }
    let map_key = args.map.clone().unwrap_or_else(|| "0".into());
    let art = args.art;
    if args.headless {
        let d = Rc::new(data::prepare_and_load(&data_opts)?);
        let index = map_scene::find_map(&d, &map_key)?;
        let mut scene = map_scene::MapScene::new(d, index, art);
        app::run_headless(&mut scene, &headless_opts(30))
    } else {
        let plan = data::prepare(&data_opts)?;
        let mut scene = boot::BootScene::new(
            plan,
            Box::new(move |d| {
                let index = map_scene::find_map(&d, &map_key)?;
                Ok(Box::new(map_scene::MapScene::new(d, index, art)) as Box<dyn app::Scene>)
            }),
        );
        app::run_windowed(&mut scene, &window_opts)
    }
}

/// App options from the command line.
fn app_options(args: &Args) -> Result<menu::AppOptions> {
    let auto = match args.online.as_deref() {
        None => None,
        Some(m @ ("host" | "join")) => {
            let mut rules = settings::MatchRules::default();
            if let Some(r) = args.rounds {
                rules.rounds_to_win = r.clamp(1, 20);
            }
            if let Some(p) = args.points {
                rules.points_to_win_round = p.clamp(1, 10);
            }
            if let Some(t) = args.teams {
                rules.teams = if t >= 2 { t.min(6) } else { 0 };
            }
            rules.pick_phase = !args.no_picks;
            if let Some(m) = args.pick_mode {
                rules.pick_mode = m;
            }
            Some(menu::online::AutoPlan {
                host: m == "host",
                code: args.code.clone(),
                room: args.room.clone(),
                bots: args.bots.unwrap_or(1).clamp(1, 12),
                expect_players: args.expect.unwrap_or(2),
                rules,
                shots: args.shots.clone(),
                shot_prefix: args.shot_prefix.clone(),
                leave_at: args.leave_at,
                crash_at: args.crash_at,
                timeout_s: args.timeout.unwrap_or(600),
                seed: args.seed,
            })
        }
        Some(other) => bail!("--online expects host or join, not {other:?}"),
    };
    Ok(menu::AppOptions {
        screen: args.screen.clone(),
        auto,
        launch_args: args.launch.clone(),
        username: args.username.clone(),
        sound: !args.headless || args.wav.is_some(),
    })
}

/// The game: menus, lobbies, matches (window or headless).
fn run_game(
    args: &Args,
    data_opts: &data::DataOptions,
    headless_opts: impl Fn(u32) -> app::HeadlessOptions,
    window_opts: app::WindowOptions,
) -> Result<()> {
    let opts = app_options(args)?;
    let make =
        move |d: Rc<rounds_import::GameData>, opts: menu::AppOptions| -> Result<menu::GameApp> {
            let content = play::build_content(&d)?;
            Ok(menu::GameApp::new(d, content, opts))
        };
    if args.headless {
        let d = Rc::new(data::prepare_and_load(data_opts)?);
        let online = opts.auto.is_some();
        let mut scene = make(d, opts)?;
        let mut h = headless_opts(if online { 0 } else { 30 });
        if online {
            h.frames = args.frames.unwrap_or(0);
            h.realtime = true;
            h.render_all = false;
            h.screenshot = None;
            h.shots.clear(); // --shots are sim frames here (confirmed snapshots)
        } else {
            h.render_all = false;
        }
        app::run_headless(&mut scene, &h)
    } else {
        let plan = data::prepare(data_opts)?;
        let mut scene = boot::BootScene::new(
            plan,
            Box::new(move |d| Ok(Box::new(make(d, opts)?) as Box<dyn app::Scene>)),
        );
        app::run_windowed(&mut scene, &window_opts)
    }
}

/// `--bot-match` (headless) and `--play` (window).
fn run_match(
    args: &Args,
    data_opts: &data::DataOptions,
    headless_opts: impl Fn(u32) -> app::HeadlessOptions,
    window_opts: app::WindowOptions,
) -> Result<()> {
    use play::input_map::{SlotControl, default_controls};
    let seed = args.seed.unwrap_or(1);
    let players = args
        .players
        .unwrap_or(if args.bot_match { 4 } else { 2 })
        .clamp(1, rounds_sim::MAX_PLAYERS);
    let cards = args.cards.clone();
    let pick_mode = args.pick_mode.unwrap_or_default();
    let make = move |d: Rc<rounds_import::GameData>,
                     sound: bool,
                     map: Option<String>,
                     bot: bool,
                     keyboard_only: bool|
          -> Result<play::MatchScene> {
        let content = play::build_content(&d)?;
        let mut setup = rounds_sim::MatchSetup::ffa(players, seed);
        if let Some(m) = &map {
            setup.map_rotation = vec![rounds_sim::MapId(map_scene::find_map(&d, m)? as u16)];
        }
        let controls = if bot {
            vec![SlotControl::Bot; players]
        } else {
            default_controls(players, keyboard_only)
        };
        let mut scene = play::MatchScene::new(
            d,
            content,
            play::MatchConfig {
                setup,
                controls,
                bot_seed: seed,
                sound,
                cards: cards.clone(),
                faces: Vec::new(),
                options: Default::default(),
                bot_rematch: None,
                pick_mode: pick_mode.to_sim(),
            },
        );
        scene.verbose = bot;
        Ok(scene)
    };
    if args.headless {
        if !args.bot_match {
            bail!("--play needs a window; use --bot-match with --headless");
        }
        let d = Rc::new(data::prepare_and_load(data_opts)?);
        let frames = args
            .frames
            .unwrap_or(args.shots.iter().copied().max().unwrap_or(600) + 1);
        let mut scene = make(d, args.wav.is_some(), args.map.clone(), true, false)?;
        let opts = headless_opts(frames);
        let opts = app::HeadlessOptions { frames, ..opts };
        app::run_headless(&mut scene, &opts)?;
        let s = &scene.stats;
        println!(
            "match: {} sim frames, {} shots, {} hits, {} deaths, {} explosions, {} picks, {} points, {} rounds",
            s.frames, s.shots, s.hits, s.deaths, s.explosions, s.picks, s.points, s.rounds
        );
        // Cross-platform determinism checks compare this line.
        let w = scene.driver().current();
        println!("checksum: frame {} {:016x}", w.frame(), w.checksum());
        scene.report_sounds();
        Ok(())
    } else {
        let plan = data::prepare(data_opts)?;
        let map = args.map.clone();
        let bot = args.bot_match;
        let keyboard_only = args.keyboard_only;
        let mut scene = boot::BootScene::new(
            plan,
            Box::new(move |d| {
                Ok(Box::new(make(d, true, map.clone(), bot, keyboard_only)?)
                    as Box<dyn app::Scene>)
            }),
        );
        app::run_windowed(&mut scene, &window_opts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Result<Option<Args>> {
        parse_args(s.split_whitespace().map(String::from))
    }

    #[test]
    fn cli() {
        let a = parse("--headless --frames 10 --screenshot scratch/x.png --size 640x360")
            .unwrap()
            .unwrap();
        assert!(a.headless);
        assert_eq!(a.frames, Some(10));
        assert_eq!(a.size, Some((640, 360)));
        assert_eq!(
            a.screenshot.as_deref(),
            Some(std::path::Path::new("scratch/x.png"))
        );
        assert!(parse("--help").unwrap().is_none());
        assert!(parse("--bogus").is_err());
        assert!(parse("--size 0x10").is_err());
        assert!(parse("--frames").is_err());
        let m = parse("--headless --map Balance_M --art 3 --screenshot x.png")
            .unwrap()
            .unwrap();
        assert_eq!(m.map.as_deref(), Some("Balance_M"));
        assert_eq!(m.art, Some(3));
        let s = parse("--map-sheet s.png --data cache/import")
            .unwrap()
            .unwrap();
        assert!(s.map_sheet.is_some() && s.data.is_some());
    }
}
