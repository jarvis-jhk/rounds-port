# Playing rounds-port

> **Status: early development.** This page describes what the game is built
> to do; some of it (12 players on one screen, teams, online play in the real
> menus) is still being finished, see [STATUS.md](STATUS.md).

rounds-port is a clean-room reimplementation of ROUNDS for Linux and Windows
(macOS later). It contains none of the original's art, sound or code: you need
your own copy of [ROUNDS on Steam](https://store.steampowered.com/app/1557740/ROUNDS/),
and rounds-port converts what it needs from that install the first time it
starts. The Steam install is only read, never modified. Not affiliated with
Landfall Games.

## Install

Pick one way.

**With Warpzone (easiest, keeps itself up to date).** Install the
[Warpzone launcher](https://github.com/jarvis-jhk/warpzone), open the
"ROUNDS (rounds-port)" page and press Install. The launcher finds your ROUNDS
install itself (Steam libraries, Proton prefixes), has the game convert it, and
verifies the download against GitHub's checksum. It also starts the game
straight into a room from invite links and "Join" buttons.

**By hand.** Download from the
[releases page](https://github.com/jarvis-jhk/rounds-port/releases):

| System | File | Start |
|---|---|---|
| Linux (x86_64, glibc 2.35+) and Steam Deck | `rounds-port-<version>-linux-x86_64.tar.gz` | unpack, run `./play.sh` |
| Windows 10/11 (x86_64) | `rounds-port-<version>-windows-x86_64.zip` | unzip, run `rounds-port.exe` |

Check the download against `SHA256SUMS` (`sha256sum -c --ignore-missing SHA256SUMS`
on Linux, `Get-FileHash` in PowerShell). Linux needs SDL2 (`libSDL2-2.0.so.0`,
already present on Steam Deck and nearly every desktop distribution; otherwise
install `libsdl2-2.0-0` / `SDL2` with your package manager) and a Vulkan or
OpenGL driver. Windows needs a Vulkan or DirectX 12 capable graphics driver;
SmartScreen may warn about the unsigned download ("More info", "Run anyway").

**Steam Deck.** In Desktop Mode unpack the tarball somewhere in your home
folder, right-click `play.sh` and choose "Add to Steam" (or Steam, Add a
Non-Steam Game, Browse, show All Files, pick `play.sh`). Controllers work in
Game Mode with the "Gamepad" Steam Input template. The same tarball serves
every Linux machine; nothing Deck-specific is needed.

## First start

1. Start the game. If it finds ROUNDS in a Steam library it shows an import
   progress screen (a few seconds to a minute) and opens the main menu. The
   converted data goes to `~/.local/share/rounds-port` (Linux) or
   `%APPDATA%\rounds-port` (Windows), about 400 MB. Delete that folder to start
   over; nothing else is touched.
2. If ROUNDS is not found (a Steam library on another drive that Steam does
   not list, a copy from somewhere else), point the game at the folder that
   contains `ROUNDS.exe` and `ROUNDS_Data`:

   ```sh
   ./play.sh --import --game "/path/to/ROUNDS"          # Linux
   rounds-port.exe --import --game "D:\Games\ROUNDS"    # Windows (in a command prompt)
   ```

   or set `ROUNDS_GAME_DIR` to that folder. Then start the game normally. If
   the install is later removed, the converted data keeps working.
3. After a game update that changes the data format, the next start converts
   again by itself.

## Menus

Main menu: **Local** (Versus, Sandbox), **Online**, **Options** (resolution,
fullscreen, VSync, volumes, screen shake, aiming options, Controls),
**Quit**. F11 or Alt+Enter toggles borderless fullscreen. Esc (or Start on a
controller) pauses a match.

## Controls

The same screen is in the game: Options, Controls. There you can remap
jump, shoot, block, the moves and "edit face" for each scheme: pick a cell,
press the new key or button (Esc cancels); "Reset defaults" restores the
table below. The settings are saved.

| | Controller | Keyboard + mouse | Keyboard only, left half | Keyboard only, right half |
|---|---|---|---|---|
| Move | left stick | A D (W S) | W A S D | arrow keys |
| Aim | right stick | mouse | with the move keys | I J K L (or the arrows) |
| Jump | A, LB or RB | Space | Space | Right Shift, `/` |
| Shoot | X or RT | left mouse button | F | `.` or Right Ctrl |
| Block | B or LT | right mouse button | G | `,` or Right Alt |
| Join / ready | A | Space | Space | Right Shift |
| Edit face | Y | E or right click | E | `'` |
| Pause | Start | Esc | Esc | |

* Delete leaves the lobby with the right keyboard half.
* In the lobby, Tab switches a keyboard player between mouse aim and the
  keys-only left half.
* Options has "Aim with left stick" for pads whose right stick you do not
  want to use, and aim snapping to 16 directions.
* Windows reads pads through XInput and raw input, so more than four work;
  any pad SDL knows works on both systems. A pad SDL does not know can be
  added with a `gamecontrollerdb.txt` next to the program.
* Menus take the arrow/WASD keys, Enter, Esc, the mouse, and any pad's D-pad,
  A and B.

## Local multiplayer (up to 12 on one screen)

Local, Versus opens the lobby. Every device presses its jump/join button to
take a seat (one seat per press; the keyboard can seat two players: mouse and
keyboard-only, left and right halves). Each player picks a face, a colour
(12, all different) and, in team mode, a team, then readies up with jump. The
match starts when everybody is ready.

The lobby also has the match settings: mode (free-for-all, or 2 to 6 teams
with any split), points per round, rounds to win, card picks (AUTO, IN TURN,
TOGETHER or NO: in turn is the original, one picker after another; together
gives every picker their own hand at once; auto picks together when more
than three players pick), which
maps are in the rotation, and "+ BOT" / "- BOT" to fill seats with computer
players. With many players the camera and HUD scale so everyone fits;
twelve players on one screen are playable on a TV with a handful of pads.

## Online (Warpzone)

Online play uses [Warpzone](https://github.com/jarvis-jhk/warpzone): accounts
are anonymous device keys with a username, rooms and join codes come from the
Warpzone server, connections are encrypted and direct where possible (NAT
traversal), through Warpzone's relay otherwise. The game itself runs a
rollback simulation, so remote players feel as responsive as local ones as long
as the connection is decent (under ~150 ms round trip is comfortable). A match
has at most 12 players in any mix: for example 12 machines with one player
each, or three couches of four.

Online, Host room / Browse rooms / Join by code. The first time you choose a
username ("Choose a name"); it is shown to other players.

* **Host a room.** Set the room to public (listed) or unlisted, entry to open,
  code required, or host approves (the host accepts or declines each knock),
  and the mode (free-for-all or teams). You land in the online lobby with
  the room code on top: **share the code**, or press "Copy invite link" (C)
  and paste the link to your friends. Everybody, host included, seats
  its local players in the lobby as above and readies up. Only the host
  changes the settings and the map rotation; the match starts when all
  players are ready. The host must play; if the host leaves, the room ends.
* **Browse rooms.** Lists public rooms with their player count, mode and
  entry; pick one to join (host-approval rooms show "waiting for the host").
* **Join by code.** Type the six-character code and press Enter.
* **Invite links.** Warpzone's launcher and website give `warpzone://join/...`
  links for rooms. With the launcher installed and "Open warpzone:// links with
  Warpzone" enabled in its settings, clicking one starts rounds-port straight
  into the room. (The game's lobby shows the room code; copy that for people
  who do not use the launcher.)
* **Joining a running match.** Late joiners wait in the lobby and play from
  the next match.
* **Someone drops out** (crash, quit, connection lost): the match pauses for a
  moment ("resyncing"), then continues without them.

You can play without the launcher: the game talks to the public Warpzone
server by itself. `WARPZONE_SERVER=<url>` points it at another server (for
example your own, see Warpzone's self-hosting guide).

## Languages

Options, Language switches the menus to any language of your ROUNDS copy.
For Japanese and Chinese the game uses a font installed on your system
(Noto Sans CJK, Droid Sans Fallback, WenQuanYi on Linux; Yu Gothic,
Meiryo, Microsoft YaHei/JhengHei on Windows); without one those texts show
as boxes. On Linux install e.g. `fonts-noto-cjk`.

## Troubleshooting

* **Nothing happens / window stays black.** Run it from a terminal
  (`./play.sh`, or `rounds-port.exe` in a command prompt) to see messages;
  `RUST_LOG=info` adds more. `rounds-port --list-devices` shows the graphics
  adapters, controllers and audio devices the game sees. Update your graphics
  driver; force a backend with `WGPU_BACKEND=vulkan`, `gl` (Linux) or `dx12`
  (Windows).
* **"needs the SDL2 library" (Linux).** Install `libsdl2-2.0-0` (Debian,
  Ubuntu, Mint), `SDL2` (Fedora), `sdl2` (Arch).
* **"ROUNDS is not installed or was not found".** See First start; the folder must contain
  `ROUNDS_Data/globalgamemanagers`. Steam must have finished installing
  ROUNDS.
* **Import was interrupted or looks wrong.** Delete the `import-v*` folder in
  the data folder and start again.
* **No sound.** Check the volumes in Options; on Linux the game uses SDL's
  audio (PulseAudio/PipeWire/ALSA); `rounds-port --list-devices` lists the
  playback devices.
* **A controller is not recognised.** `--list-devices` lists "joysticks with
  no controller mapping"; add a line for it to `gamecontrollerdb.txt` next to
  the program (community database: github.com/mdqinc/SDL_GameControllerDB).
* **Cannot connect online.** Check that the server is reachable (the status
  line on the Online screen shows "Connecting to Warpzone..."). Behind strict
  NATs the relay is used automatically, which adds latency. A firewall prompt
  on Windows should be allowed. `ROUNDS_NET_FORCE_RELAY=1` forces the relay to
  test.
* **Desync or version mismatch.** Everybody in a room must run the same
  rounds-port version; the host rejects other versions with a message.

## Report problems

Open an issue at <https://github.com/jarvis-jhk/rounds-port/issues> with the
version (name of the archive), your system and graphics driver, and the output
of `rounds-port --list-devices`. Do not attach game files or screenshots that
show the original's artwork beyond what is needed to describe the problem.

### Crash reports

If the game crashes (a Rust panic), it sends one short, anonymous report to
the project's public [ntfy](https://ntfy.sh) topic, from which a GitHub
Action files an issue labelled `crash`. The report holds the version, the
operating system and architecture, the panic message and location and a
backtrace of the program; your home folder and user name are removed, and
nothing about your machine, your game files or the match is sent. The same
text is saved as `last-crash.txt` in the data folder. Turn it off with the
environment variable `ROUNDS_PORT_CRASH_REPORTS=0`.
