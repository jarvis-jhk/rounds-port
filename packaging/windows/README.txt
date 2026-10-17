rounds-port - a clean-room reimplementation of ROUNDS (Windows x86_64)
=====================================================================

You need your own copy of ROUNDS (Steam: https://store.steampowered.com/app/1557740/ROUNDS/).
This package contains no game data; the first start converts the art, sounds,
maps and cards of your installation into rounds-port's own data folder
(%APPDATA%\rounds-port). The Steam install is only read, never modified.
Not affiliated with Landfall Games.

Start
-----
Unzip the folder anywhere (not into Program Files) and run rounds-port.exe.
SDL2.dll must stay next to it. A console window opens next to the game; it
shows log messages and can be ignored.

The first start looks for ROUNDS through Steam (registry and library folders).
If it is somewhere else, open a command prompt in this folder:

    rounds-port.exe --import --game "D:\Games\ROUNDS"
    rounds-port.exe

(the folder that contains ROUNDS.exe and ROUNDS_Data).

Requirements
------------
- Windows 10 or 11, 64-bit.
- A graphics driver with Vulkan 1.1 or DirectX 12 (anything from the last ~8
  years). WGPU_BACKEND=dx12 or WGPU_BACKEND=vulkan (set in the environment)
  forces one; rounds-port.exe --list-devices shows what the game sees.
- Windows SmartScreen may warn about an unsigned download: "More info" ->
  "Run anyway". The checksum is in SHA256SUMS next to the download.

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
More than four Xbox pads work through raw input; any SDL-supported pad works.

Online play
-----------
Online -> Host room / Browse rooms / Join by code. Rooms are provided by
Warpzone (https://github.com/jarvis-jhk/warpzone); you pick a username the
first time. Up to 12 players in any mix of local and online players. If you
installed rounds-port through the Warpzone launcher, it can also start the
game straight into a room from an invite link. Allow rounds-port.exe in the
Windows firewall prompt for direct connections; otherwise the Warpzone relay
is used.

Troubleshooting
---------------
- Window does not open / "no suitable GPU": update the graphics driver; try
  WGPU_BACKEND=dx12 or vulkan.
- Run from a command prompt to see messages; RUST_LOG=info for more.

Licenses
--------
rounds-port is MIT OR Apache-2.0 (LICENSE-MIT, LICENSE-APACHE). Third-party
components, including SDL2 (zlib license): THIRD_PARTY_NOTICES.md and the
licenses\ folder.
Source and documentation: https://github.com/jarvis-jhk/rounds-port
