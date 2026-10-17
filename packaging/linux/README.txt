rounds-port - a clean-room reimplementation of ROUNDS (Linux x86_64)
====================================================================

You need your own copy of ROUNDS (Steam: https://store.steampowered.com/app/1557740/ROUNDS/).
This package contains no game data; the first start converts the art, sounds,
maps and cards of your installation into rounds-port's own data folder
(~/.local/share/rounds-port). The Steam install is only read, never modified.
Not affiliated with Landfall Games.

Start
-----
    ./play.sh

The first start looks for ROUNDS in your Steam libraries (including Flatpak and
Snap Steam). If it is somewhere else:

    ./play.sh --import --game /path/to/ROUNDS      # folder that contains ROUNDS_Data
    ./play.sh

Requirements
------------
- 64-bit Linux with glibc 2.35 or newer (Ubuntu 22.04+, Debian 12+, Fedora 36+,
  SteamOS 3 / Steam Deck).
- SDL2 2.26 or newer (libSDL2-2.0.so.0). Steam Deck and nearly every desktop
  already have it.
- A Vulkan driver (or OpenGL as fallback): Mesa (AMD, Intel), NVIDIA proprietary.
  WGPU_BACKEND=gl ./play.sh forces OpenGL, WGPU_BACKEND=vulkan forces Vulkan.

Steam Deck
----------
Desktop Mode: unpack the archive anywhere in your home folder, right-click
play.sh -> "Add to Steam" (or Steam -> Games -> Add a Non-Steam Game ->
Browse, choose play.sh; set the file filter to "All Files"). Controllers work in
Game Mode; in Steam Input choose the "Gamepad" template. Up to 12 players: any
number of extra controllers, or share the screen with the keyboard.

Controls (also in the game: Options -> Controls)
------------------------------------------------
Controller:      left stick move, right stick aim, A / LB / RB jump,
                 X / RT shoot, B / LT block, Start pause.
Keyboard+mouse:  A D W S move, Space jump, mouse aim, left button shoot,
                 right button block, Esc pause.
Keyboard only:   left side W A S D, Space jump, F shoot, G block;
                 right side: arrows move, I J K L aim, Right Shift jump,
                 . shoot, , block.
Fullscreen:      F11 or Alt+Enter.

Online play
-----------
Online -> Host room / Browse rooms / Join by code. Rooms are provided by
Warpzone (https://github.com/jarvis-jhk/warpzone); you pick a username the
first time. Up to 12 players in any mix of local and online players.
If you installed rounds-port through the Warpzone launcher, it can also start
the game straight into a room from an invite link.

Troubleshooting
---------------
- "needs the SDL2 library": install your distribution's SDL2 package.
- Black window / "no suitable GPU": update your graphics driver; try
  WGPU_BACKEND=gl ./play.sh. ./play.sh --list-devices shows what the game sees.
- No controller: ./play.sh --list-devices lists controllers; a pad without an
  SDL mapping can be added with a gamecontrollerdb.txt next to the binary.
- Log output: RUST_LOG=info ./play.sh

Licenses
--------
rounds-port is MIT OR Apache-2.0 (LICENSE-MIT, LICENSE-APACHE). Third-party
components: THIRD_PARTY_NOTICES.md and the licenses/ folder.
Source and documentation: https://github.com/jarvis-jhk/rounds-port
