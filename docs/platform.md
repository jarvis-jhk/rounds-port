# Platform layer and renderer

`rounds-render` (crates/render) is the wgpu 2D renderer; `rounds-port`
(crates/game) is the binary: SDL2 window, input and audio, the fixed-step
loop, the start-up data flow (install discovery, import), the map
presentation with the original look, and the renderer demo scene.

## Building and running on Linux

```sh
source env.sh
cargo build -p rounds-port                      # or --release
cargo test -p rounds-render -p rounds-port      # offscreen pixel tests need a GPU or lavapipe

target/debug/rounds-port                         # the game: menus, local + online play (window)
target/debug/rounds-port --warpzone-join <room|invite link>   # launcher / invite: straight to the room
target/debug/rounds-port --warpzone-host          # launcher: open the host screen

# Headless: no window, no audio device, deterministic virtual time.
target/debug/rounds-port --headless --screen lobby12 --screenshot scratch/menus/lobby12.png [--frames 40]
target/debug/rounds-port --headless --online host --bots 4 --expect 12 [--rounds 2 --points 1 --teams 0 --no-picks]
target/debug/rounds-port --headless --online join --code K7QM3X --bots 4      # see "Online" in docs/net.md
target/debug/rounds-port --headless --map 0 --screenshot scratch/map-0.png [--size 1920x1080] [--art 3]
target/debug/rounds-port --headless --map Castle_M --screenshot scratch/castle.png
target/debug/rounds-port --headless --map-sheet scratch/maps-hd.png [--size 640x360]   # all 70 maps
target/debug/rounds-port --headless --frames 120 --screenshot scratch/demo.png \
    [--size 1920x1080] [--fps 60] [--wav scratch/demo.wav] [--play-ogg some.ogg]   # renderer demo
target/debug/rounds-port --headless --bot-match [--players N] [--map NAME] [--seed S] \
    [--frames F] --screenshot scratch/m.png [--shots 200,500,900] [--wav scratch/m.wav] \
    [--cards "EXPLOSIVE BULLET,SAW"]          # bot match, screenshots m-200.png, ...
target/debug/rounds-port --play [--players N] [--keyboard-only]   # local match (window)
target/debug/rounds-port --import                # find ROUNDS, import if needed, exit
target/debug/rounds-port --list-devices          # GPUs, controllers, audio devices
target/debug/rounds-port --version
target/debug/rounds-port --headless --fx-preview A_Explosion,PlayerDeath --shots 5,20 --screenshot scratch/fx.png
target/debug/rounds-port --headless --bot-match --players 11 --pick-mode together --shots 140 --screenshot scratch/p.png
target/debug/rounds-port --virtual-pads scratch/pads.cmd   # simulated controllers (tests, see below)
target/debug/rounds-port --map-viewer [--map N] [--size 1280x720] [--fullscreen] [--no-vsync] [--fps-cap 144]
```

- Windowed without arguments starts the game (boot/import screen, then
  the main menu). `--map-viewer` / `--map N` open the map viewer
  (Left/Right: map, Up/Down: art). `--headless` without `--map`,
  `--map-sheet`, `--screen` or `--online` runs the renderer demo.
- `--screen NAME` starts on a menu screen (headless screenshots): `main`,
  `local`, `options`, `controls`, `maps`, `lobby`, `lobby12` (12 joined
  players), `lobby-teams`, `editor` (face editor), `register`, `online`,
  `host`, `browse`, `code`, `onlinelobby` (with fake remote machines; no
  network), `pause` (a 4-bot match with the pause menu open after 400
  ticks; use `--frames 450`).
- `--map N|NAME`: `N` is the map number 0..69 (build order, as in
  `docs/spec/maps.md` section 8), or the scene name (`Balance_M`, `castle`).
- `--art N` picks one of the nine arts (background grade + particles);
  default `map number mod 9` (the original picks one at random per round).
- `--game DIR` / `--data DIR` (use an import dir as is, e.g. `cache/import`).

## Game data: data dir, install discovery, import

- **User data dir**: `$ROUNDS_PORT_DATA_DIR` if set (`env.sh` sets
  `cache/userdata` for development), else `$XDG_DATA_HOME/rounds-port` or
  `~/.local/share/rounds-port` (Linux), `%APPDATA%\rounds-port` (Windows),
  `~/Library/Application Support/rounds-port` (macOS). `paths.rs`.
- **Install discovery** (`paths::find_install`), first that contains
  `ROUNDS_Data/globalgamemanagers`: `--game DIR`, `$ROUNDS_GAME_DIR`,
  `$WARPZONE_DATA_ROUNDS` (Warpzone launcher), then every Steam library:
  Steam roots `~/.steam/steam`, `~/.steam/root`, `~/.local/share/Steam`,
  Flatpak (`~/.var/app/com.valvesoftware.Steam/...`), Snap; on Windows the
  registry (`HKCU\Software\Valve\Steam\SteamPath`, HKLM `InstallPath`, via
  `reg query`) and `Program Files (x86)\Steam`. Each root's
  `steamapps/libraryfolders.vdf` (new and old format) lists libraries;
  libraries whose `apps` contain ROUNDS (1557740) are tried first;
  `steamapps/common/ROUNDS`.
- **Import** (`data.rs`): `rounds_import::import` into
  `<data dir>/import-v<N>`; it returns immediately when `index.json` has the
  current version, the same source fingerprint and all files. Headless runs
  it synchronously; the window shows a progress screen (`boot.rs`, import in
  a thread, then `GameData::load` and the scene's `load`). If the install
  is gone but an import exists, the import is used.

- SDL2 is linked against the system runtime `libSDL2-2.0.so.0` (no -dev
  package): `crates/game/build.rs` adds `tools/lib` (a symlink created by
  `scripts/bootstrap-tools.sh`) to the link path; `ROUNDS_SDL2_LIB_DIR`
  overrides it. The binary's `NEEDED` entry is the soname, so it runs
  against any SDL >= 2.0.20 (Ubuntu 22.04, the CI build host). Functions
  newer than that (`SDL_GetWindowSizeInPixels`, `SDL_JoystickAttachVirtualEx`)
  are looked up at run time (`platform::sdl_optional_fn`) with a fallback.
- `WGPU_BACKEND=vulkan|gl`, `WGPU_ADAPTER_NAME=llvmpipe` (software, useful
  for CI; a case-insensitive substring of the adapter name, honoured by
  `Gpu::new`) and `WGPU_POWER_PREF` select the GPU. `ROUNDS_NO_BLOOM=1`
  skips bloom, `ROUNDS_SHADOW_SCALE=N` renders the shadow mask at 1/N
  (both for comparisons and slow software rasterisers). Mesa's shader cache goes to
  `$XDG_CACHE_HOME`, which `env.sh` points into `tools/`.
- `RUST_LOG=info|debug` for logs (wgpu internals only at debug).
- Window keys: F11 or Alt+Enter toggles borderless fullscreen.

## Testing on a private X server (Xvfb)

Windowed runs are tested on a private, headless X server so nothing shows
on the desktop and nothing plays on the speakers:

```sh
scripts/dev/xvfb.sh              # Xvfb :61 (unpacked into tools/), prints :61
scripts/dev/xvfb-wm.sh           # optional: metacity on :61 (HOME/XDG in tools/wm-home,
                                 # no session D-Bus) so fullscreen/maximize behave like a desktop
env -u WAYLAND_DISPLAY DISPLAY=:61 SDL_VIDEODRIVER=x11 SDL_AUDIODRIVER=disk \
    SDL_DISKAUDIOFILE=scratch/sdl-audio.raw target/debug/rounds-port [--size 960x540]
DISPLAY=:61 xdotool key Down Return; DISPLAY=:61 xdotool mousemove X Y click 1
import -display :61 -window root scratch/win-shot.png
```

- Rendering on Xvfb uses lavapipe (no DRI3, so the GPU cannot present):
  about 10-20 fps in menus and 3-5 fps in a match at 1280x720 (map art
  overdraw and the analytic ACES grading dominate; a real GPU needs ~2 ms).
  Below 4 fps the 0.25 s hitch clamp slows the game down; that is
  expected there. The log prints `fps, ticks/s` every 10 s (`RUST_LOG=info`).
- **Simulated controllers** (`--virtual-pads FILE`, `vpad.rs`): SDL virtual
  joysticks with a standard game-controller layout, driven by commands
  appended to FILE while the game runs: `attach N`, `detach P`, `tap P a`,
  `button P x 1`, `axis P lx 1`, `axis P rt 1`, `center P`. Hot-plug goes
  through SDL's normal device events.
- Verified this way (Oct 2026): boot/import screen and intro, main menu by
  keyboard and mouse (hover, click), options (resolution, fullscreen modes
  under metacity, volume sliders), Controls remapping (key captured, saved,
  used in play), the local lobby (keyboard + mouse and the right keyboard
  half; 4 virtual pads joining, D-pad/stick changes, a 5th pad hot-plugged,
  a pad unplugged in the lobby and mid-match and re-plugged), card picks by
  keyboard and pad, play with injected input (move, jump, mouse aim, shoot,
  block; pad stick/A/RT/LT), pause, leave to the lobby, the online lobby's
  invite-link copy (read back with `xclip`), menu and match music in the
  disk audio file.
- Bugs found and fixed on the way: wgpu surface creation failed without a
  display handle (`from_display_and_window`); menu navigation and match
  buttons missed taps shorter than one tick (now press edges count); a
  join press also readied the new lobby player; exclusive fullscreen went
  iconic on focus loss (SDL minimise hint off on Linux); windowed
  resolution changes now re-centre the window; thin strokes of the Gravity
  SDF fonts vanished at 720p (analytic, supersampled SDF coverage);
  unplugged controllers left lobby players that blocked the start (now
  removed) and match slots dead (now waiting for the next pad);
  `WGPU_ADAPTER_NAME` was ignored; the main camera's post-processing
  profile ("Default": grading, vignette, bloom) was never applied because
  its namespace-qualified script name did not match.

## Windows (cross-compiled from Linux)

Everything lives in `tools/windows/` (about 800 MB unpacked):

```sh
scripts/fetch-windows-toolchain.sh   # llvm-mingw 20261006 (msvcrt) + SDL2 2.32.10 mingw devel, SHA-256 pinned
scripts/build-windows.sh [--debug]   # -> target/dist/windows/{rounds-port.exe, SDL2.dll}
scripts/run-windows.sh --headless --frames 10 --screenshot scratch/win.png
scripts/run-windows.sh --list-devices
```

- Target `x86_64-pc-windows-gnu` with llvm-mingw's clang/lld as linker.
  `scripts/windows-env.sh` (sourced by the build script) sets
  `CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER` and puts llvm-mingw on `PATH`
  (rustc runs `x86_64-w64-mingw32-dlltool` for the windows-rs raw-dylib
  imports). No `.cargo/config.toml` is needed.
- Rust's windows-gnu std links GCC's `libgcc`/`libgcc_eh`; the fetch script
  creates shims in `tools/windows/gcc-shim/` (compiler-rt builtins and LLVM
  libunwind, both static), so the exe only depends on system DLLs and
  `SDL2.dll` (checked with `llvm-readobj --coff-imports`).
- The release exe carries line tables (workspace profile); strip it with
  `tools/windows/llvm-mingw/bin/llvm-strip` for distribution.
- Ship `SDL2.dll` next to the exe. Optional `gamecontrollerdb.txt` next to
  the exe adds controller mappings.
- Release builds use the GUI subsystem (`windows_subsystem = "windows"`: no
  console window from Explorer); at start the game attaches to the parent
  console (`AttachConsole(ATTACH_PARENT_PROCESS)`) and points stdout/stderr
  at it unless they are redirected, so `--help`, `--version` and logs still
  print from cmd/PowerShell. Debug builds keep the console subsystem.
- `crates/game/build.rs` draws a procedural icon (two overlapping discs; no
  game art) into a multi-size `.ico`, writes a `.rc` with it and a
  VERSIONINFO block (version from Cargo) and compiles it with llvm-mingw's
  `x86_64-w64-mingw32-windres --no-preprocess` (`WINDRES` overrides); the
  object is linked into the exe. Without windres the build warns and goes on.

### Running under Proton (verified)

`scripts/run-windows.sh` runs Proton Experimental's `files/bin/wine`
directly inside a bubblewrap sandbox: `/` is read-only, only the project is
writable, `/tmp` is `tools/wine-tmp` (wineserver socket), `/dev/shm` is a
private tmpfs, `HOME`/XDG/`WINEPREFIX` point into `tools/`. Nothing outside
the project can be written. Steam and the Proton launcher script are not
involved; the prefix is `tools/wineprefix`.

- wgpu imports `dxgi.dll`; Wine's own dxgi needs wined3d + vkd3d which only
  Proton's launcher wires up, so the script puts Proton's DXVK `dxgi.dll`
  into the prefix (as Proton does for games). Rendering uses wgpu's Vulkan
  backend through winevulkan on the host Mesa driver.
- **Display:** with Wine's `null` graphics driver, Vulkan instance
  creation hangs (Proton experimental-11.0-20261014), so Wine connects to an
  X server as its user driver: by default the private Xvfb (`:61`, started
  on demand), so windowed runs are fine too. `WINE_USER_DISPLAY=1` uses the
  real `$DISPLAY` (then only `--headless`/`--list-devices`/`--help` unless
  `WINE_ALLOW_WINDOW=1`); `WINE_NULLDRV=1` uses the null driver (only
  `--help` works there today).
- **Cross-platform determinism** (Oct 2026): a 4-bot headless match
  (`--bot-match --players 4 --seed 7 --frames 3000`) ends at frame 3571
  with checksum `c31484e90ac391bc` on Linux and on the Windows build under
  Proton (`checksum:` line of `--bot-match`). Online, `ONLINE_TEST_WINDOWS="1"
  scripts/net/online-test.sh win` mixes a Windows instance with Linux ones;
  see docs/net.md for the results.
- Verified: `--list-devices` (3 adapters, SDL 2.32.10, WASAPI devices) and
  `--headless --frames 10 --screenshot`, output matching
  the Linux build.

## Renderer API (`rounds-render`)

```rust
let gpu = Gpu::headless()?;                         // or Gpu::new(instance, Some(&surface))
let mut r = Renderer::new(&gpu, OFFSCREEN_FORMAT);  // or surface.format()
let tex = r.create_texture(&gpu, &Image::load_png(path)?, TextureOptions::default());
let target = OffscreenTarget::new(&gpu, (1920, 1080), OFFSCREEN_FORMAT);

let mut list = DrawList::new();                     // refill every frame
list.sprite(Layer::World, sort, Sprite { texture: tex, uv, transform, size, pivot, tint, .. });
list.rect(Layer::World, 0, center, size, angle, color);
list.circle / ring (arcs for timers) / line / polygon (concave ok) / text(...)
list.light(Light { position, color, intensity, range, source_radius, casts_shadows });
list.shadow_caster(&polygon_points);

let frame = FrameSettings { camera: Camera2D { position, ortho_size, rotation, shake }, clear,
                            lighting: LightingSettings { enabled, ambient, exposure, light_map_scale },
                            post: PostSettings { vignette, saturation, contrast, brightness, tint, .. } };
r.render(&gpu, target.target(), &frame, &list);
target.capture(&gpu)?.save_png("scratch/x.png")?;
```

- **Coordinates:** world is y-up, Unity-like orthographic camera
  (`ortho_size` = half the visible height; `shake` is an extra world offset
  so shake never reveals texture edges). `Layer::Screen` is in pixels,
  origin top-left, y down. Sprite pivots: world (0,0) = bottom-left like
  Unity; screen (0,0) = top-left. `UvRect` uses top-left texture
  coordinates; `UvRect::from_unity_pixels` converts Unity sprite rects.
- **Layers:** `World` (lit), `WorldUnlit` (after lighting: glows, debug,
  in-world UI), `Screen` (after post: HUD/menus). Within a layer, ascending
  `sort` key, ties in submission order.
- **Colour:** the original uses Unity's *linear* colour space. `Color` is
  sRGB like a Unity `Color`; textures are sRGB; blending happens in linear
  light in an `Rgba16Float` scene buffer; output is an sRGB target.
  Textures are straight alpha and premultiplied in the shader.
- **Batching:** one instanced pipeline draws sprites, glyphs and analytic
  shapes (anti-aliased SDF rect/rounded rect/capsule line, ellipse,
  ring/arc; an SDF-glyph kind is ready for TextMeshPro atlases). Consecutive
  items with the same texture form one draw call; polygons go through an
  indexed mesh pipeline. `Renderer::stats()` reports quads, triangles, draw
  calls, lights and shadow edges.
- **Text:** embedded Noto Sans Bold (SIL OFL 1.1,
  `crates/render/assets/fonts/OFL.txt`) rasterised with fontdue at display
  size into a 2048² atlas; `load_font` / `load_font_indexed` (collections)
  add TTF/OTF/TTC; `set_fallback_fonts` = fonts tried per glyph when the
  requested raster font lacks a character (system CJK fonts);
  `measure_text` for layout. Multi-line, kerning, anchor/alignment. SDF
  fonts (`add_sdf_font`, TextMeshPro atlases) use the typed glyph tables,
  their kerning pairs, and coverage from the screen-space gradient of the
  UVs (atlas padding) with 4 sub-pixel taps, so thin strokes survive small
  sizes.
- **Lighting ("SF" soft shadows):** light map at 1/`light_map_scale` of the
  target (the original uses 8), cleared to ambient; per light a shadow-mask
  pass (each light-facing caster edge is extruded away from the light; its
  fragment shader computes analytically how much of the light's disk the
  edge hides, which gives penumbrae), then an additive radial light.
  Lit layers are multiplied by `light map x exposure`.
- **Render targets:** `SurfaceTarget` (window swapchain: sRGB format,
  vsync = FIFO, otherwise Mailbox/Immediate, resize, reacquire on
  outdated), `OffscreenTarget` (capture to `Image`, also usable as a sprite
  texture via `texture_from_target`).
- **Bloom** (`PostSettings::bloom`, post-processing v2): prefilter at half
  resolution with the quadratic soft threshold (threshold converted to
  linear), a 4-tap box down chain whose length follows the diffusion
  (`log2(max(w,h)/2) + diffusion - 10`, at least 2 levels), tent
  upsampling with the PPv2 sample scale, each level added to the next
  bigger one; the result times `2^(intensity/10) - 1` is added before
  vignette and grading.
- **Post:** simple vignette + grading hooks (brightness, contrast,
  saturation, tint), plus `PostSettings::grading` (`ColorGrading`: Unity
  post-processing v2 HDR grading - post exposure, contrast in ACEScc,
  colour filter, channel mixer, lift/gamma/gain, hue shift, saturation -
  with the ACES tonemapper, evaluated analytically instead of a LUT) and
  `unity_vignette` (PPv2 classic vignette).
- **Background stage** (`FrameSettings::background`): `Layer::Background`
  into its own HDR target cleared to `BackgroundSettings::clear`, then the
  `Fog` layer (SF fog: `(fog.rgb*fog.a + scatter*mix(lightMap,
  shadowMap, softHardMix)) * exposure`, premultiplied over), then its own
  `grading`; the tonemapped result is what `Layer::World` draws onto.
- **Masks:** `Sprite::mask` (`Mask::Write` / `VisibleInside` /
  `VisibleOutside`) uses a stencil buffer in the world pass (Unity's
  `1 <= stencil` rule); writers drop texels with alpha < 0.01.
  `Sprite::blend`: `Alpha` or `Additive`.
- **Lights:** `Light::cookie` + `cookie_size` (SFLight-style textured
  rect); `LightingSettings::shadow_map_scale` (resolution of the shadowed
  light map; the unshadowed one uses `light_map_scale` and is only built
  for the fog) and `multiply_world` (false = lights only via the fog, as in
  the original).

## Map presentation (`rounds-port`)

How the original composes a frame is in `docs/spec/maps.md` 7.9 and 10.5
(clarified from the data): a light camera draws a red-cleared background
with additive art particles, the SF fog layer (lights + soft shadows) and
an "art" grade; the main camera draws the map on top - pieces only write a
stencil mask, the foreground art particles show inside them - and the
"Default" grade goes over everything.

- `art.rs` (`MainLook`): from `main.json` + `post_profiles.json` +
  `materials.json`: camera size, light camera clear colour, `SFRenderer`
  (ambient 0.623, exposure 2.88, light map 1/8, shadow map 1/1, fog,
  scatter, soft/hard mix), `SFLight`s (position, colour, intensity, radius,
  rect size, cookie), both cameras' profiles (PPv2 parameters with
  override semantics), and `ArtHandler`'s nine arts (profile + particle
  systems, parts plus active descendants). `start_art` builds running
  systems (texture, blend from `_Mode`, `_Color`, mask interaction,
  sorting layer/order, z).
- `psys.rs`: particle effects from the import's typed particle systems
  (revision 2) for art, maps, prefabs and the player: an `EffectDef` is
  every system of an object hierarchy (transform relative to its root,
  texture, blend from the material `_Mode`/shader, tint, sorting layer and
  order, `SetTeamColor` selector, death sub-emitters); an `Effect` plays it
  at a 3D root transform (spawned objects use `LookRotation(dir)` like the
  original): emission over time and distance, bursts with cycles and
  probability, start delay, duration/looping, prewarm, shapes (sphere,
  hemisphere, cone(+volume), circle, edge, donut, rectangle, box; mesh and
  sprite shapes as boxes), local/world simulation space, scaling modes,
  3D/2D gravity, velocity (+orbital z, radial), limit velocity + drag,
  force, colour/size/rotation over lifetime, simple noise, texture-sheet
  grid animation, billboard and stretched rendering, sort modes; trails and
  collisions are not done. Own RNG, presentation time only (game time
  unless `use_unscaled_time`). `--fx-preview NAMES` shows prefab effects.
- `map_view.rs` (`MapView`): active objects only; `SpriteMasking.SpriteMask`
  objects -> `Mask::Write` (no colour), other enabled SpriteRenderers with a
  material drawn normally (simple, 9-slice, tiled; flip; trimmed packing);
  `SFPolygon` -> shadow casters; rope `LineRenderer`s -> lines; objects
  with Unity's built-in `SpriteMask` component (saws) draw and write their
  own stencil value, and particle systems below them that are "visible
  inside mask" (the saw's brush-stroke fill) are clipped to it; sprites of
  inactive objects are kept hidden for explicit use (debris pieces). Order:
  sorting layer, order in layer, farther z first, stencil writers first on
  ties. **Positions come from a `TransformProvider`**:
  `body_transform(object)` gives a rigid body's current world transform
  (objects under a body follow it), `rope_ends(object)`, `visible(object)`;
  `InitialPose` uses the import's poses. The sim adapter implements it.
- Map particle systems (play-on-awake, active; e.g. the saws' fill and
  sparks) run as `psys` effects that follow their object's current pose.
- `map_scene.rs`: `MapPresentation` (view + art + cookies; `draw` fills
  `FrameSettings` and the `DrawList`), `MapScene` (the `Scene`/viewer),
  `TextureCache`, `map_sheet`. Camera: ortho size 20 at 16:9; narrower
  targets get letterbox bars, wider ones see more to the sides.

## Platform layer (`rounds-port`)

- `app.rs`: the `Scene` trait (`load`, `tick` at a fixed 0.014 s with all
  local device inputs, `draw` with interpolation `alpha`) and two hosts:
  `run_windowed` (SDL window, wgpu surface from the raw window handle, event
  pump, frame pacing) and `run_headless` (virtual time, offscreen target,
  offline audio).
- `timing.rs`: `FixedStep` accumulator (0.25 s hitch clamp) and `alpha`.
- `input.rs`: `InputHub` keeps keyboard+mouse and any number of SDL
  GameControllers (hot-plug via `ControllerDeviceAdded/Removed`, SDL's
  built-in mapping DB plus optional `gamecontrollerdb.txt`, rumble).
  `snapshot()` yields one `InputState` per device per tick: move/aim axes
  (y-up, radial dead zone), triggers, held/pressed/released buttons (taps
  shorter than a tick are latched). Hints enable background events and
  RawInput/HIDAPI on Windows (beyond XInput's 4 pads). Bindings come from `settings.bindings` (remappable, see "Input devices and schemes").
- `audio/`: our own mixer (256 voices, per-voice volume/pitch/pan
  (constant power)/looping, Music/Sfx/Ui buses + master, click-free gain
  ramps, linear resampling to the device rate), Ogg Vorbis decoding with
  `lewton`, SDL2 output device (f32 stereo, 48 kHz, 512 frames; commands
  over a channel to the audio thread) or the offline mixer writing a 16-bit
  WAV in headless mode.

## Local match (`rounds-port`, `crates/game/src/play/`)

`MatchScene` (a `Scene`) hosts the simulation and presents it with the
original's look. Content is built once from the import
(`rounds_content::build` -> `rounds_sim::Content`, `play::build_content`).

- **Driver** (`driver.rs`): `trait MatchDriver { advance(&mut self, local:
  &[(slot, PlayerInput)]) -> Advance; previous(); current(); drain_events();
  content() }`. `LocalDriver` steps the world once per 0.014 s tick with every
  slot's input and keeps the previous world for interpolation. An online
  driver (GGRS from `rounds-net`) implements the same trait: it gets only the
  local slots' inputs, answers GGRS save/load requests with `World::clone`,
  may step 0..n frames per tick (catch-up after a rollback; `Advance::
  rolled_back_to`) and re-delivers events of re-simulated frames.
  `EventFilter` lets each `(frame, seq)` event through once, so sounds and
  particles of rolled-back frames fire once (window of 128 frames).
- **Slots** (`input_map.rs`): `SlotControl::{Device(id), AnyController, Bot,
  Remote}`. `--play`: keyboard+mouse = slot 0, `--keyboard-only` adds the
  keyboard-only scheme as slot 1, every other slot binds to the next
  controller that is connected (a bot until then). Bots are
  `rounds_content::bots::Bots`.
- **Input -> `PlayerInput`**: bindings per player.md 3.2 (`input.rs`):
  keyboard+mouse A/D/W/S, Space jump, LMB fire, RMB block, aim = unit vector
  from the player (world position) to the cursor converted with the last
  frame's camera (`Camera2D::screen_to_world`, shake included); controllers:
  left stick move, right stick aim, A/LB/RB jump, X/RT fire, B/LT block,
  radial dead zone 0.2 (hub), upper dead zone 0.9 and the Aim script's 0.2
  threshold (`input_map.rs`). Keyboard-only (port decision): arrows move,
  I/J/K/L aim (falls back to the move direction, as "aim with left stick"),
  Right Shift or `/` jump, `.` or Right Ctrl fire, `,` or Right Alt block.
  Menu buttons: left/right/confirm map to the sim's `BTN_MENU_*`.
- **Maps** (`map_play.rs`): the sim's current, loaded (waiting at x = +90)
  and previous map are each a `MapPresentation`; `Poses` implements
  `TransformProvider`: props from `World::props()` (matched by import object
  id), smashers from `map_anims()`, static saws spin (`DamageBox.rotate_
  speed`), ropes follow the props they were attached to at map release,
  destroyed boxes hide and their debris draws as stencil writers, and every
  top-level child slides with its own random delay (0..0.25 s, from the
  sim's map seed) along the 0.7 s slide curve (match-flow.md 7.1).
  `MapView::set_driven` / `TransformProvider::slide` support this.
- **Players** (`player_view.rs`): body = `Target_Circle1` in the skin colour
  writing a per-player stencil value (`Mask::WriteValue`), filled by the
  player's `Skin_PlayerOne..Four` particle system (`effects.rs`; start
  colour random between the skin's two fill shades, damage blink recolours
  the living particles like `PlayerSkinParticle.BlinkColor`) clipped to it
  (`Mask::InsideValue`; procedural `Grass6` strokes without the import's
  skin prefabs), the face (`E_1_Neutral`, `M_2_Neutral`,
  offsets from the `CharacterItem` prefabs) mirrored to the movement, legs
  and arms as two-bone IK chains drawn along a Bezier, feet stepping from
  the sim's leg-step state and planted on the ground the sim's leg rays hit
  (`World::foot_plants`, slopes and props), the two-part gun with the mirror spring, block
  orb + cooldown ring, block flash, health bar with its spring anchor and
  lag bar, stun/silence icons, damage blink, spawn-in scale. Colours: the
  import's skin bank for ids 0..3, the spec's HSV recipe for 4..11.
- **Imported effects** (`effects.rs`): the player prefab's systems on sim
  events (jump/land smoke in the team colour, block flash, recharge, heal
  crosses, snow, out-of-bounds bursts), `PlayerDeath` /
  `PlayerDeath_Phoenix` (the player's colour on `DeathEffect.partToColor`,
  `ROT` systems along the blow) and every spawned object whose prefab has
  particle systems (explosions, fields, sparks; following the object while
  it lives, `SetTeamColor` from its spawner). Events without an imported
  effect keep the procedural ones below. `ROUNDS_NO_PSYS=1` turns them off.
- **Bullets/effects** (`world_view.rs`): tapered bullet trails (0.03 s,
  width 0.3 scaled by damage, team colour), hit sparks, death bursts,
  jump/land dust, explosions, heal/slow particles, out-of-bounds bursts,
  teleport rings; effect objects draw their prefab sprites by node activity
  (`object_node_active`), countdown rings (`effect_progress`) and
  procedural fields (black hole, healing/static/toxic fields, saws, bombs).
  World shake: `Screenshaker` (spring 3000, damper 30, force -10, halved with
  more than 2 players) on the camera; slow motion slows game-time particles.
- **HUD** (`hud.rs`, `ui_prefab.rs`): small round counter (one row per
  team), card bars (one row per player), point/round visualiser (balls per
  team, fills, "HALF ORANGE"/"ROUND ORANGE", winner ball to its pip, loser
  ball to the card face), "LET'S GOO!", "VICTORY!", "REMATCH?" with YES/NO,
  the card choice screen: the imported `CardBase` and card-art UI prefabs
  laid out by a small `RectTransform` resolver (masks via stencil values),
  name, description, stat rows (word amounts), rarity triangles, theme
  colours, selection scale spring, the picker's big face with spring hands,
  the pick fly-out; card art plays its `Animator`s
  (`rounds_import::animator::AnimatorPlayer` per Animator of the art
  prefab; curves on anchored/local position, scale, z rotation, Image
  colour and sprite, activity, CanvasGroup alpha; `ui_prefab::ArtAnim`).
  Simultaneous pick phases (`World::simultaneous_pick`) show every hand at
  once: one panel per picker in a grid sized for up to 12, in the player's
  colours, with its face, its own cursor, the picked card and a tick when
  done. Fonts: TextMeshPro SDF atlases from the import
  (`Renderer::add_sdf_font`; Gravity Light/Regular/Bold, LiberationSans for
  the card texts as in the CardBase prefab).
- **Audio** (`sound.rs`): sim `Sound`/`LoopSound` cues and presentation
  triggers (shots and impacts through each card's `Gun.soundShotModifier` /
  `soundImpactModifier`, up to 3/2 layered by priority, damage-to-explosion
  layers, footsteps, round-visualiser sounds) play through the import's
  SoundEvent bank (all container layers, delays, random clip without
  repeats, volume/pitch jitter, cooldown, probability, polyphony), panned by
  x. Music: a random level track with its Card and Game layers crossfaded by
  the pick state (0.3 s), next track 4 s before the end, ambience loop.
- Ropes follow `World::ropes()` (authored points before the map is
  released; a rope the sim dropped is hidden); debris pieces of destroyed
  boxes draw their own piece sprites (`PieceDef.id`) as stencil writers.
- Headless: `--shots` saves several frames per run; only those frames are
  rendered (the scene still ticks at the virtual frame rate). Sound is
  only decoded with `--wav`.

## App flow and menus (`crates/game/src/menu/`, `ui/`)

`menu::GameApp` is the game's `Scene`: a stack of pages (Back pops) plus
the running match. Starting a match keeps the lobby page on the stack;
while it runs, pages above it are overlays (pause, options); when the
match ends (`MatchDriver::outcome`: the flow's `Ended`, or online
`MatchEnded`/`MatchAborted`) the lobby page gets the outcome and shows
again. Shared state (`menu::Shared`): `GameData`, the sim `Content`
(built once), `MainLook`, `Settings`, localised strings, the face
catalog, one `PlayAssets` (moved into the `MatchScene` while a match runs
and taken back after), the UI sound bank and the online session.

Pages (ui.md 2): **Intro** (first screen of a windowed start: after 1 s
the imported `LandfallLogo` pops in and out, then `Rounds_Logo2_White`,
with the original's CurveAnimation timing and sounds; any input skips),
**Main** (ROUNDS wordmark, Local, Online, Options, Quit, version), **Local** (Versus, Sandbox, Back), **Lobby** (below), **Options**,
**Controls** (the bindings of every scheme as an editable table: pick a
cell, press the new key / button, Esc cancels; Reset defaults), **Maps** (map pool: 70 toggles,
All/None), **Pause** (Resume, Options, Menu = leave the match, Quit;
offline the sim stops, online it keeps running and local devices are
muted), the **face editor** and the online pages (docs/net.md "Game
integration").

**UI toolkit** (`ui/`): immediate mode on a 1920x1080 reference canvas
scaled to the target height (the canvas widens with the aspect ratio; x
from the centre, y down). A page's `tick` lays out, hit-tests the pointer
and every device's navigation and records draw commands (`ui::Frame`:
rects, circles, rings, texts in the Gravity SDF fonts, faces); `draw`
replays them. `ListNav` = vertical lists with the original's behaviour:
focus bar only in keyboard/controller mode (hidden once the mouse moves or
clicks), hover selects in mouse mode, bold focused item, `Nope` red flash
+ shake on disabled items, hover/click sounds. `Router`: per-device
navigation edges with key repeat (first after 0.35 s, then 0.2 s), sticks
count past 0.5, merged navigation for menus, text input mode (letters
type; keyboards then only act with Enter/Esc/Tab/arrows). Texts outside
the Gravity atlases' Latin range (Cyrillic, CJK) fall back to the embedded
Noto Sans. The menu backdrop is `ArtHandler.menuArt` (particle systems
under the light camera's fog and the menu art's grade, `ui/background.rs`).
Labels come from the import's `StringTableDefault` in the chosen locale
(`ui/strings.rs`, TMP tags stripped), English fallbacks for port-only texts.
For Japanese and Chinese (Korean too) the import has no font; the game
loads a system CJK font at runtime (`sysfonts.rs`: `fc-match` and the
font directories on Linux, `%WINDIR%\Fonts` on Windows) as the raster
fallback font; nothing is bundled. Prompts mix text with controller glyphs
from the imported `XboxSeriesButtonAtlas` / `Playstation_4_Buttons_Atlas`
(PlayStation by controller name; drawn circles with letters otherwise).
Music: the main-menu theme (`MUS_Main_Menu_Theme`, faded in the first
time) plays in menus and stops when a match starts. Online matches open
with "Match found" (0.5 s, `UI_Match_Found_SE`) and the sides' names with
"VS" (2 s) over the start of the match (not in confirmed-frame
screenshots). The online lobby has "Copy invite link (C)" (SDL clipboard).

**Window requests**: scenes ask the host through `TickContext::requests`
(`quit` + exit code, `display` = mode/resolution/vsync, `text_input` on/off,
headless `screenshot` of the current frame); `TickContext::platform` lists
the display's resolutions and the connected controllers.

## Settings (`settings.rs`)

`<data dir>/settings.json` (pretty JSON, written on every change through a
temp file; `ROUNDS_NO_SAVE=1` disables writing, used by tests). Missing or
unknown fields fall back to defaults. Fields: `master_volume`,
`sfx_volume`, `music_volume` (0..1; the original's dB mapping is the
slider value as linear gain, silent below 0.001), `display_mode`
(`Exclusive`, `FullscreenWindow`, `Maximized`, `Windowed`), `resolution`,
`vsync`, `screen_shake` (0..1, port option; multiplies the world shake),
`language` (import locale, default = system language if the import has it),
`left_stick_aim` (controllers aim with the left stick while the right one
is centred), `mouse_aim_snap` / `controller_aim_snap` (16 directions,
ui.md 2.7), `show_stat_numbers` (cards show numbers instead of words),
`faces` (10 presets), `profiles` (per device: face preset, colour),
`rules` (teams 0/2..6, points per round, rounds to win, card picks, map
pool exclusions, `pick_mode` Auto / Sequential / Simultaneous: the lobby's
"CARD PICKS" cycles AUTO / IN TURN / TOGETHER / NO, carried to every peer
in the online rules and applied with `World::set_pick_mode` right after
`World::new`), `online` (host defaults: public, entry), `bindings`
(remapped controls per scheme and action: SDL scancode names and
`Mouse1..3`, SDL controller button names and `lefttrigger`/`righttrigger`;
missing entries = defaults). Not exposed:
`OPTION_MUTE` (no chat) and `OPTION_CROSSPLAY` (no platforms).

## Input devices and schemes (`input.rs`)

| Device | Move | Aim | Jump | Fire | Block | Lobby |
|---|---|---|---|---|---|---|
| Keyboard + mouse | WASD | mouse | Space | LMB | RMB | Space joins/readies, E edit, Tab = keys only, Esc leave |
| Keyboard left (keys only) | WASD | move keys | Space | F | G | same keys as keyboard + mouse; Tab = mouse aim |
| Keyboard right | arrows | I/J/K/L, else arrows | RShift, `/`, Num0 | `.`, RCtrl | `,`, RAlt | RShift joins/readies, `'` edit, Delete leave |
| Controller | left stick | right stick (left stick when centred, option) | A, LB, RB | X, RT | B, LT | A joins/readies, Start readies, Y edit, B leave |

Jump, shoot, block, the four moves and edit face are remappable per
scheme (controllers: jump/shoot/block/edit; the left stick always moves);
menu keys (Enter, Esc, Tab, Q/E, D-pad, A/B/Start) stay fixed. The host
applies `TickContext::requests.bindings`; key/button presses for the
capture arrive as `TickContext::raw`.
Keyboard + mouse and keyboard-left use the same keys, so only one of them
plays; two players share a keyboard as left half + right half. Menus take
every device: WASD/arrows/D-pad/stick move, Enter/A confirm, Esc/B back,
Q/E and LB/RB switch tabs, the mouse hovers/clicks, the wheel scrolls.
Pause: Esc (keyboard) or Start (controller). SDL text input is on only
while a text field has focus.

## Lobby (`menu/lobby.rs`) and faces (`faces.rs`, `menu/editor.rs`)

Up to 12 player cards on one screen (4 per row up to 8 players, then 6 per
row; a "Press jump to join" card fills the next free place). Any device's
join press adds a player (sound `UI_Player_Added`); its card shows the
device, the face (preset 1..10, left/right), the colour (12 unique colours,
skipping taken ones) or, with teams, the team (team colours: team t uses
colour t, members get lighter/darker shades of it, `PlayAssets::skin(c +
12 k)`), and the ready state. Up/down moves between the rows and, from the
first row, into the settings bar (mode FFA / 2..6 teams, points per round,
rounds to win, card picks, maps, + bot, - bot) where left/right moves and
jump/confirm changes. The mouse clicks arrows, faces (edit), ready, the bar
(right click = previous value). The match starts 1.5 s after everybody is
ready (at least 2 players; with teams at least 2 teams in use; sandbox: 1).
Faces and colours are remembered per device profile (`keyboard`,
`keyboard-right`, `controller:<name>#<n>`).

The **face editor** (from a card: E / Y / `'` / click on the face) edits a
preset: category tabs (eyes, mouths, detail, detail 2; Q/E or LB/RB),
an item grid 7 per row with previews (D-pad / click; wheel scrolls), the
big preview on which the mouse drags items (press within 2 face units;
offsets clamped to 4.5) and the right stick moves the active category's
item; Save / Start / Back closes and saves. The **catalog** is the main
scene's `CharacterCreatorItemLoader` lists resolved through imported
`CharacterItem` prefabs; the current import only contains the two neutral
ones, so the rest falls back to the imported sprites named `E_*`, `M_*`,
`A_*` (66 eyes, 19 mouths, 27 accessories + "none") with default offsets
(eyes (0, 0.35), mouth (0, -1.3), accessories (0, 0)) and scale 1. Faces
are drawn on the players (`PlayerDraw::face`) and in the UI
(`faces::draw_face_screen`).

## What is left

- Map look: chromatic aberration (0.02) of the main profile is not
  rendered; noise and mesh emitter shapes are approximations; particle
  trails and collisions are missing; the order of art systems that tie in
  Unity's sort is a guess (maps.md 11); the original's 8-bit buffer between
  the two cameras is not reproduced (the background stage is tonemapped,
  then composited in float). Bloom strength follows the profile (35); it
  was not compared with the original on screen.
- Lighting: light penetration into casters (`minLightPenetration`), the
  "soft" (non-linear) blend mode, multiple lights per mask pass.
- Text: outlines/shadows, rich text; SDF (HUD) texts have no CJK fallback
  (the card texts are English in the original's layout too).
- Sprites: mipmaps, sprite meshes from Unity's tight sprite outlines.
- Polygons are not anti-aliased (no MSAA); shapes are.
- Audio: streaming decode for long music, better resampling, device
  hot-swap.
- Input: aim keys of the right keyboard half (I/J/K/L) are fixed; no
  per-controller (only per scheme) bindings.
- Menus: the face catalog uses `faces.json` (all 197 items). Glyph prompts
  are in the lobby only.
- Match presentation: bullet-vs-bullet and projectile add-on visuals
  (homing, drill, thruster) are generic; text reveal particles of banners;
  ammo pips/reload ring on the gun; the card particle backgrounds
  (`CardBase` systems) are not drawn.
- Windows: DX12 backend under vkd3d-proton, a real Windows smoke test (only
  Proton so far).
- Software rendering (lavapipe) is slow (3-5 fps in matches).
