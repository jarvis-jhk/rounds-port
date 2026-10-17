//! The application loop: fixed-step simulation, interpolated rendering, and
//! the two hosts (SDL window, headless offscreen).
//!
//! A [`Scene`] is what the loop runs. The real game will implement it with
//! an adapter around `rounds-sim`: `tick` builds the per-player inputs from
//! the device states (or GGRS) and advances the sim state one 0.014 s step
//! (keeping the previous state); `draw` interpolates previous -> current by
//! `alpha` and fills the draw list; sim events become sounds and particles.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use rounds_render::{
    DrawList, FrameSettings, Gpu, OFFSCREEN_FORMAT, OffscreenTarget, Renderer, SurfaceTarget,
};
use sdl2::event::{Event, WindowEvent};
use sdl2::keyboard::{Keycode, Mod};
use sdl2::video::FullscreenType;

use crate::audio::Audio;
use crate::input::{DeviceId, InputHub, InputState};
use crate::timing::{FIXED_DT, FixedStep};

/// What the host knows about the platform (for menus).
#[derive(Clone, Debug, Default)]
pub struct PlatformInfo {
    /// No window (offscreen run).
    pub headless: bool,
    /// Fullscreen resolutions of the display, largest first.
    pub resolutions: Vec<(u32, u32)>,
    /// Connected controllers.
    pub controllers: Vec<crate::input::ControllerInfo>,
}

/// Display configuration a scene asks the host to apply.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisplayRequest {
    pub mode: crate::settings::DisplayMode,
    pub resolution: Option<(u32, u32)>,
    pub vsync: bool,
}

/// Requests from a scene to its host, collected during a tick.
#[derive(Debug, Default)]
pub struct AppRequests {
    /// Leave the application.
    pub quit: bool,
    /// Process exit code when quitting (headless test runs).
    pub exit_code: i32,
    pub display: Option<DisplayRequest>,
    /// Start (true) or stop SDL text input (text fields).
    pub text_input: Option<bool>,
    /// Headless: render this frame and save it here.
    pub screenshot: Option<PathBuf>,
    /// Apply remapped controls to the input hub.
    pub bindings: Option<crate::input::Bindings>,
    /// Put this text on the system clipboard.
    pub clipboard: Option<String>,
}

/// Passed to [`Scene::tick`].
pub struct TickContext<'a> {
    /// Input of every local device for this tick.
    pub inputs: &'a [(DeviceId, InputState)],
    /// Text typed since the previous tick.
    pub text: &'a str,
    /// Keys / buttons that went down since the previous tick (binding
    /// capture).
    pub raw: &'a [crate::input::RawPress],
    pub audio: &'a mut Audio,
    /// Index of this tick (0-based).
    pub tick: u64,
    pub dt: f32,
    pub platform: &'a PlatformInfo,
    pub requests: &'a mut AppRequests,
}

/// Passed to [`Scene::draw`].
pub struct DrawContext<'a> {
    pub list: &'a mut DrawList,
    /// Camera, clear colour, lighting, post; persistent across frames.
    pub frame: &'a mut FrameSettings,
    /// Interpolation factor between the previous and current tick.
    pub alpha: f32,
    /// Target size in pixels.
    pub size: (u32, u32),
    pub renderer: &'a Renderer,
}

/// Passed to [`Scene::load`] once at start.
pub struct LoadContext<'a> {
    pub gpu: &'a Gpu,
    pub renderer: &'a mut Renderer,
    pub audio: &'a mut Audio,
}

pub trait Scene {
    fn load(&mut self, ctx: &mut LoadContext<'_>) -> Result<()>;
    /// Called once per frame before the ticks, with GPU access (create or
    /// replace resources, finish background loading).
    fn poll(&mut self, _ctx: &mut LoadContext<'_>) -> Result<()> {
        Ok(())
    }
    fn tick(&mut self, ctx: &mut TickContext<'_>);
    fn draw(&mut self, ctx: &mut DrawContext<'_>);
}

/// Options for the headless host.
pub struct HeadlessOptions {
    pub size: (u32, u32),
    pub frames: u32,
    /// Simulated frame rate (virtual time; no wall clock involved).
    pub fps: f64,
    pub screenshot: Option<PathBuf>,
    pub wav: Option<PathBuf>,
    /// Also save screenshots after these frames (1-based), named
    /// `<screenshot stem>-<frame>.png` next to `screenshot`.
    pub shots: Vec<u32>,
    /// Render every frame (false: only the screenshot frames; the scene
    /// still ticks at the virtual frame rate).
    pub render_all: bool,
    /// Pace by the wall clock instead of virtual time (online runs); then
    /// `frames` = 0 runs until the scene asks to quit.
    pub realtime: bool,
}

/// Path of the screenshot taken after `frame`.
pub fn shot_path(base: &std::path::Path, frame: u32) -> PathBuf {
    let stem = base
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "shot".into());
    base.with_file_name(format!("{stem}-{frame}.png"))
}

/// Renders `frames` frames offscreen at a virtual frame rate, then saves a
/// screenshot of the last frame. Deterministic: no wall clock, no window.
pub fn run_headless(scene: &mut dyn Scene, opts: &HeadlessOptions) -> Result<()> {
    let gpu = Gpu::headless().context("creating a headless GPU device")?;
    println!("GPU: {}", gpu.describe());
    let mut renderer = Renderer::new(&gpu, OFFSCREEN_FORMAT);
    let target = OffscreenTarget::new(&gpu, opts.size, OFFSCREEN_FORMAT);
    let mut audio = Audio::offline(opts.wav.as_deref())?;
    println!("audio: {}", audio.description());
    scene.load(&mut LoadContext {
        gpu: &gpu,
        renderer: &mut renderer,
        audio: &mut audio,
    })?;
    let mut list = DrawList::new();
    let mut frame = FrameSettings::default();
    let mut step = FixedStep::new(FIXED_DT);
    let inputs: Vec<(DeviceId, InputState)> = [
        DeviceId::KeyboardMouse,
        DeviceId::KeyboardLeft,
        DeviceId::KeyboardRight,
    ]
    .into_iter()
    .map(|d| (d, InputState::default()))
    .collect();
    let platform = PlatformInfo {
        headless: true,
        resolutions: vec![opts.size],
        controllers: Vec::new(),
    };
    let mut requests = AppRequests::default();
    let frame_dt = 1.0 / opts.fps;
    let started = Instant::now();
    let mut last = Instant::now();
    let mut f = 0u32;
    loop {
        f += 1;
        if opts.frames > 0 && f > opts.frames {
            break;
        }
        let is_shot = opts.shots.contains(&f);
        scene.poll(&mut LoadContext {
            gpu: &gpu,
            renderer: &mut renderer,
            audio: &mut audio,
        })?;
        let dt = if opts.realtime {
            let now = Instant::now();
            let dt = (now - last).as_secs_f64();
            last = now;
            dt
        } else {
            frame_dt
        };
        for _ in 0..step.advance(dt) {
            scene.tick(&mut TickContext {
                inputs: &inputs,
                text: "",
                raw: &[],
                audio: &mut audio,
                tick: step.ticks(),
                dt: FIXED_DT as f32,
                platform: &platform,
                requests: &mut requests,
            });
        }
        let requested = requests.screenshot.take();
        let render = opts.render_all
            || is_shot
            || requested.is_some()
            || f == opts.frames
            || (requests.quit && opts.screenshot.is_some());
        if render {
            list.clear();
            scene.draw(&mut DrawContext {
                list: &mut list,
                frame: &mut frame,
                alpha: step.alpha(),
                size: opts.size,
                renderer: &renderer,
            });
            renderer.render(&gpu, target.target(), &frame, &list);
            let mut paths = Vec::new();
            if is_shot {
                paths.push(shot_path(
                    opts.screenshot
                        .as_deref()
                        .unwrap_or(std::path::Path::new("scratch/shot.png")),
                    f,
                ));
            }
            paths.extend(requested);
            for path in paths {
                if let Some(d) = path.parent() {
                    std::fs::create_dir_all(d).ok();
                }
                target.capture(&gpu)?.save_png(&path)?;
                println!(
                    "screenshot: {} (frame {f}, tick {})",
                    path.display(),
                    step.ticks()
                );
            }
        }
        if opts.realtime {
            audio.advance_offline(dt)?;
            std::thread::sleep(Duration::from_millis(2));
        } else {
            audio.advance_offline(frame_dt)?;
        }
        if requests.quit {
            break;
        }
    }
    let s = renderer.stats();
    println!(
        "rendered {} frames ({} ticks) at {}x{} in {:.2?}; last frame: {} quads, {} triangles, {} draws, {} lights, {} shadow edges",
        f.saturating_sub(1),
        step.ticks(),
        opts.size.0,
        opts.size.1,
        started.elapsed(),
        s.quads,
        s.mesh_triangles,
        s.draw_calls,
        s.lights,
        s.shadow_edges
    );
    if let Some(path) = &opts.screenshot {
        target.capture(&gpu)?.save_png(path)?;
        println!("screenshot: {}", path.display());
    }
    audio.finish()?;
    if requests.quit && requests.exit_code != 0 {
        std::process::exit(requests.exit_code);
    }
    Ok(())
}

/// Options for the windowed host.
pub struct WindowOptions {
    pub size: (u32, u32),
    pub fullscreen: bool,
    pub vsync: bool,
    /// Frame cap when vsync is off (0 = uncapped).
    pub fps_cap: f64,
    /// `--virtual-pads FILE`: simulated controllers (tests).
    pub virtual_pads: Option<PathBuf>,
}

/// Opens the SDL window and runs until it is closed.
/// F11 or Alt+Enter toggles borderless fullscreen.
pub fn run_windowed(scene: &mut dyn Scene, opts: &WindowOptions) -> Result<()> {
    let sdl = sdl2::init().map_err(anyhow::Error::msg)?;
    crate::platform::set_hints();
    let video = sdl.video().map_err(anyhow::Error::msg)?;
    let controllers = sdl.game_controller().map_err(anyhow::Error::msg)?;
    let sdl_audio = sdl.audio().map_err(anyhow::Error::msg)?;
    let mut events = sdl.event_pump().map_err(anyhow::Error::msg)?;

    let mut win = video.window("ROUNDS (rounds-port)", opts.size.0, opts.size.1);
    win.position_centered().resizable().allow_highdpi();
    let mut window = win.build().context("creating window")?;
    if opts.fullscreen {
        window
            .set_fullscreen(FullscreenType::Desktop)
            .map_err(anyhow::Error::msg)?;
    }
    crate::platform::check_native_handles(&window)?;
    let instance = rounds_render::new_instance();
    // SAFETY: the surface is dropped before the window (declared later in
    // this function, dropped in reverse order), so the handles stay valid.
    let surface = unsafe {
        instance.create_surface_unsafe(
            rounds_render::wgpu::SurfaceTargetUnsafe::from_display_and_window(&window, &window)?,
        )
    }
    .context("creating wgpu surface")?;
    let gpu = Gpu::new(instance, Some(&surface))?;
    log::info!("GPU: {}", gpu.describe());
    let mut surface = SurfaceTarget::new(
        &gpu,
        surface,
        crate::platform::window_pixels(&window),
        opts.vsync,
    )?;
    let mut renderer = Renderer::new(&gpu, surface.format());

    let mut audio = match Audio::open_device(&sdl_audio) {
        Ok(a) => a,
        Err(e) => {
            log::warn!("{e:#}; continuing without sound");
            Audio::offline(None)?
        }
    };
    let mut input = InputHub::new(controllers);
    let mut vpads = opts.virtual_pads.clone().map(crate::vpad::VirtualPads::new);
    scene.load(&mut LoadContext {
        gpu: &gpu,
        renderer: &mut renderer,
        audio: &mut audio,
    })?;

    let mut list = DrawList::new();
    let mut frame = FrameSettings::default();
    let mut step = FixedStep::new(FIXED_DT);
    let mut inputs = Vec::new();
    let mut last = Instant::now();
    let mut platform = PlatformInfo {
        headless: false,
        resolutions: display_resolutions(&video),
        controllers: Vec::new(),
    };
    let mut requests = AppRequests::default();
    let mut vsync = opts.vsync;
    video.text_input().stop();
    // Frame-rate log (RUST_LOG=info): frames and ticks per 10 s.
    let mut stats_t = Instant::now();
    let (mut stats_frames, mut stats_ticks0) = (0u32, 0u64);
    'main: loop {
        stats_frames += 1;
        if stats_t.elapsed() >= Duration::from_secs(10) {
            let secs = stats_t.elapsed().as_secs_f64();
            log::info!(
                "{:.1} fps, {:.1} ticks/s (target {:.1})",
                stats_frames as f64 / secs,
                (step.ticks() - stats_ticks0) as f64 / secs,
                1.0 / FIXED_DT
            );
            stats_t = Instant::now();
            stats_frames = 0;
            stats_ticks0 = step.ticks();
        }
        if let Some(v) = &mut vpads {
            v.poll();
        }
        for event in events.poll_iter() {
            match &event {
                Event::Quit { .. } => break 'main,
                Event::Window {
                    win_event: WindowEvent::SizeChanged(..) | WindowEvent::Resized(..),
                    ..
                } => surface.resize(&gpu, crate::platform::window_pixels(&window)),
                Event::KeyDown {
                    keycode: Some(k),
                    keymod,
                    repeat: false,
                    ..
                } if *k == Keycode::F11
                    || (*k == Keycode::Return
                        && keymod.intersects(Mod::LALTMOD | Mod::RALTMOD)) =>
                {
                    let next = match window.fullscreen_state() {
                        FullscreenType::Off => FullscreenType::Desktop,
                        _ => FullscreenType::Off,
                    };
                    if let Err(e) = window.set_fullscreen(next) {
                        log::warn!("fullscreen toggle: {e}");
                    }
                    continue;
                }
                _ => {}
            }
            input.handle_event(&event);
        }

        scene.poll(&mut LoadContext {
            gpu: &gpu,
            renderer: &mut renderer,
            audio: &mut audio,
        })?;
        let now = Instant::now();
        let dt = (now - last).as_secs_f64();
        last = now;
        // Mouse positions arrive in window points; the scene works in
        // drawable pixels (differs with high-DPI scaling).
        let (pw, ph) = crate::platform::window_pixels(&window);
        let (ww, wh) = window.size();
        let dpi = glam::Vec2::new(pw as f32 / ww.max(1) as f32, ph as f32 / wh.max(1) as f32);
        platform.controllers = input.controllers();
        let mut text = input.take_text();
        let mut raw = input.take_raw();
        for _ in 0..step.advance(dt) {
            input.snapshot(&mut inputs);
            for (_, s) in inputs.iter_mut() {
                if let Some(p) = &mut s.pointer {
                    *p *= dpi;
                }
            }
            scene.tick(&mut TickContext {
                inputs: &inputs,
                text: &text,
                raw: &raw,
                audio: &mut audio,
                tick: step.ticks(),
                dt: FIXED_DT as f32,
                platform: &platform,
                requests: &mut requests,
            });
            text.clear();
            raw.clear();
        }
        if let Some(b) = requests.bindings.take() {
            input.set_bindings(b);
        }
        if let Some(t) = requests.clipboard.take()
            && let Err(e) = video.clipboard().set_clipboard_text(&t)
        {
            log::warn!("clipboard: {e}");
        }
        if requests.quit {
            break 'main;
        }
        if let Some(on) = requests.text_input.take() {
            if on {
                video.text_input().start();
            } else {
                video.text_input().stop();
            }
        }
        if let Some(d) = requests.display.take() {
            apply_display(&mut window, &d);
            if d.vsync != vsync {
                vsync = d.vsync;
                surface.set_vsync(&gpu, vsync);
            }
            surface.resize(&gpu, crate::platform::window_pixels(&window));
        }

        let Some(out) = surface.acquire(&gpu) else {
            std::thread::sleep(Duration::from_millis(5));
            continue;
        };
        list.clear();
        scene.draw(&mut DrawContext {
            list: &mut list,
            frame: &mut frame,
            alpha: step.alpha(),
            size: out.size,
            renderer: &renderer,
        });
        renderer.render(&gpu, out.target(), &frame, &list);
        out.present(&gpu);

        if !vsync && opts.fps_cap > 0.0 {
            let budget = Duration::from_secs_f64(1.0 / opts.fps_cap);
            let spent = now.elapsed();
            if spent < budget {
                std::thread::sleep(budget - spent);
            }
        }
    }
    drop(surface);
    drop(window);
    audio.finish()
}

/// Fullscreen sizes of the first display, largest first, unique.
fn display_resolutions(video: &sdl2::VideoSubsystem) -> Vec<(u32, u32)> {
    let mut out: Vec<(u32, u32)> = Vec::new();
    let n = video.num_display_modes(0).unwrap_or(0);
    for i in 0..n {
        if let Ok(m) = video.display_mode(0, i) {
            let r = (m.w.max(1) as u32, m.h.max(1) as u32);
            if !out.contains(&r) {
                out.push(r);
            }
        }
    }
    out.sort_by_key(|r| std::cmp::Reverse(r.0 * r.1));
    out
}

/// Applies a window mode / resolution request.
fn apply_display(window: &mut sdl2::video::Window, d: &DisplayRequest) {
    use crate::settings::DisplayMode;
    let res = match d.mode {
        DisplayMode::Exclusive => {
            if let Some((w, h)) = d.resolution {
                let _ = window.set_size(w, h);
                if let Ok(mut m) = window.display_mode() {
                    m.w = w as i32;
                    m.h = h as i32;
                    let _ = window.set_display_mode(Some(m));
                }
            }
            window.set_fullscreen(FullscreenType::True)
        }
        DisplayMode::FullscreenWindow => window.set_fullscreen(FullscreenType::Desktop),
        DisplayMode::Maximized => window.set_fullscreen(FullscreenType::Off).map(|_| {
            window.maximize();
        }),
        DisplayMode::Windowed => window.set_fullscreen(FullscreenType::Off).map(|_| {
            window.restore();
            if let Some((w, h)) = d.resolution
                && window.size() != (w, h)
            {
                let _ = window.set_size(w, h);
                // Keep the resized window on the screen.
                window.set_position(
                    sdl2::video::WindowPos::Centered,
                    sdl2::video::WindowPos::Centered,
                );
            }
        }),
    };
    if let Err(e) = res {
        log::warn!("display mode {:?}: {e}", d.mode);
    }
}
