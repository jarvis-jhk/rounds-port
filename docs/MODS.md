# Built-in mods

The ROADMAP item "Popular mods, built in" brings the behaviour of the 20 most
popular ROUNDS mods into rounds-port as native, optional features. This file
is the reviewable list of which mods that is, and how each will be done.

## Rules

- **Clean room.** Every mod is reimplemented in our Rust code from its
  documented and observable behaviour (README, card texts, playing it). We do
  not copy a mod's code, and we ship a mod's art or data only where its
  licence allows it and we credit it; otherwise we draw our own (card art is
  procedural, like the icon). Nothing from the original game is shipped.
- **Off by default, one toggle each.** Every mod is a separate switch in the
  lobby settings, stored per match. With all of them off the simulation is
  bit-identical to vanilla rounds-port (the determinism, card and stress
  tests stay green unchanged).
- **Online.** The host's set of enabled mods is part of the match setup that
  every rollback peer receives, so all peers simulate the same rules.
- **Tests.** Each mod gets a test that its toggle changes behaviour and that
  "off" equals vanilla.
- **Credit.** Each mod's author is named here and in the in-game toggle's
  description.

## How the list was chosen

Source: Thunderstore's ROUNDS community (thunderstore.io/c/rounds), total
downloads over all versions, snapshot of 2026-10-19 (1176 packages).

Not counted as features, as the request says: mod managers (r2modman, Gale),
the BepInEx pack, libraries and frameworks (UnboundLib, ModdingUtils,
MMHook, RarityLib, CardThemeLib, RarityBundle, ClassesManagerReborn,
MapsExtended core, WillsWackyManagers, ItemShops, ModsPlus, NullManager,
PlayerTimeScale, SimulationChamber, PoppycarsStatAdditions), Unity-only tools
(UnityExplorer, Performance_Improvements) and deprecated packages. What a
framework enables is counted where it shows up: curses and rerolls
(WillsWackyManagers) with the card packs that use them, custom maps
(MapsExtended) with the map editor and map packs.

The 15 single-bug patches (Grow, TemporaryStats, PlayerJump, LegRaycasters,
Respawn, GunUnblockable, ZeroGBullet, BlockForce, DemonicPact,
ResetAttackCooldown, Gravity, Regeneration, Teleport, StopShootingYoureDead,
AttackLevel) are counted once, as the bundle that ships them together
(AllThePatch): they only make sense as one "vanilla fixes" switch.

## The 20

Downloads in millions. Licence as stated by the mod's source repository
("none" = no licence given, so all rights reserved: behaviour only, own art).

| # | Mod | Author | Licence | Downloads | Kind | What it does |
|---|---|---|---|---|---|---|
| 1 | [CR](https://thunderstore.io/c/rounds/p/XAngelMoonX/CR/) | XAngelMoonX | none (no repo) | 2.21 | cards | 100 new cards, including a Zodiac class |
| 2 | [RoundsWithFriends](https://thunderstore.io/c/rounds/p/olavim/RoundsWithFriends/) | olavim | GPL-3.0 | 2.15 | mode | up to 16 players, deathmatch and team deathmatch, continue after a win |
| 3 | [AllThePatch](https://thunderstore.io/c/rounds/p/RoundsModding/AllThePatch/) | Rounds Modding community (Pykess, willuwontu, Root, Senyksia, TeamDK, ...) | per patch | 1.25 (bundle; single patches 1.2–2.0 each) | fixes | the 15 fundamental bug fixes listed above |
| 4 | [ChaosPoppycarsCards](https://thunderstore.io/c/rounds/p/poppycars/ChaosPoppycarsCards/) | poppycars | MIT | 1.19 | cards | 177 cards (256 with hidden ones), some curses |
| 5 | [CardBarPatch](https://thunderstore.io/c/rounds/p/BossSloth/CardBarPatch/) | BossSloth | GPL-3.0 | 1.18 | QoL | configurable card bar: size, rows, opacity, up to ~95 cards |
| 6 | [Pick_N_Cards](https://thunderstore.io/c/rounds/p/Pykess/Pick_N_Cards/) | Pykess | GPL-3.0 | 1.16 | rules | pick N cards per pick phase, draw N cards per hand, draw delay |
| 7 | [WillsWackyCards](https://thunderstore.io/c/rounds/p/willuwontu/WillsWackyCards/) | willuwontu | GPL-3.0 | 1.14 | cards | cards plus the curse and reroll mechanics (WillsWackyManagers) |
| 8 | [SetRounds](https://thunderstore.io/c/rounds/p/Ascyst/SetRounds/) | Ascyst | none | 0.99 | rules | rounds to win and points per round |
| 9 | [KeysCards](https://thunderstore.io/c/rounds/p/Keys/KeysCards/) | Keys | none | 0.96 | cards | 48 cards (+22 hidden), Treasure and Hex mechanics |
| 10 | [CardsPlus](https://thunderstore.io/c/rounds/p/willis81808/CardsPlus/) | willis81808 | MIT | 0.96 | cards | 20 cards (Hare, Turtle, Phantom, Quick Reflexes, ...) |
| 11 | [Root_Cards](https://thunderstore.io/c/rounds/p/Root/Root_Cards/) | Lilith and Tess (Root) | Unlicense | 0.92 | cards | card pack incl. the NULL system and classes |
| 12 | [MapsExtendedEditor](https://thunderstore.io/c/rounds/p/olavim/MapsExtendedEditor/) (+ [ExtraMapObjects](https://thunderstore.io/c/rounds/p/BossSloth/ExtraMapObjects/), BossSloth) | olavim | MIT | 0.89 | maps | in-game map editor; custom maps saved and shared as files |
| 13 | [ZenithMaps](https://thunderstore.io/c/rounds/p/Zenith/ZenithMaps/) | Zenith | none | 0.83 | maps | 73 custom maps (MapsExtended format) |
| 14 | [PCE](https://thunderstore.io/c/rounds/p/Pykess/PCE/) | Pykess | GPL-3.0 | 0.82 | cards | Pykess's Card Expansion |
| 15 | [LobbyImprovements](https://thunderstore.io/c/rounds/p/RoundsModdingCommunity/LobbyImprovements/) | willuwontu, Pykess | none | 0.74 | online | lobby codes, host-only invites, streamer mode, kick |
| 16 | [BetterChat](https://thunderstore.io/c/rounds/p/BossSloth/BetterChat/) | BossSloth | none | 0.74 | online | chat window with history, team and dead chat |
| 17 | [Deck_Customization](https://thunderstore.io/c/rounds/p/Pykess/Deck_Customization/) | Pykess | GPL-3.0 | 0.74 | rules | adjustable rarity and theme weights, rarity as text |
| 18 | [TabInfo](https://thunderstore.io/c/rounds/p/willuwontu/TabInfo/) | willuwontu | none | 0.72 | QoL | Tab overlay with every player's stats and cards |
| 19 | [Infoholic](https://thunderstore.io/c/rounds/p/Penial/Infoholic/) | Penial | none | 0.65 | QoL | always-on HUD with your own stats |
| 20 | [RSCards](https://thunderstore.io/c/rounds/p/RS_Mind/RSCards/) | RS_Mind | MIT | 0.65 | cards | cards built from vanilla stats (Hitscan, Mortar, ...) |

Next in line, if one of the 20 turns out to be already covered or
impossible: Map_Embiggener (Pykess, GPL-3.0, 0.60: bigger maps, chaos
out-of-bounds), GearUpCards (GearUP, 0.59), BudsCards (0.58), Arcana
(willis81808, MIT, 0.57), RSClasses (RS_Mind, MIT, 0.54).

## Already in rounds-port

Part of what some of these mods add is base behaviour here, so their toggle
only covers the rest:

- **RoundsWithFriends**: rounds-port already has up to 12 players (the
  project's limit; RWF's 16 is not planned), free-for-all and teams of any
  split. Left: "continue" after a win instead of rematch.
- **SetRounds**: points per round and rounds to win are lobby settings
  already (`MatchSetup`). Nothing left; listed for completeness.
- **LobbyImprovements**: Warpzone lobbies have invite links and the host can
  kick (`rounds-net`). Left: host-only invites, streamer mode.

## Plan, in slices

1. Survey (this file). Done 2026-10-19.
2. Framework: a `Mods` set in `MatchSetup` (sent to every peer), lobby
   toggles, settings persistence, and the "off == vanilla" test harness.
   Done 2026-10-19 with Pick_N_Cards as the first mod: `crates/sim/src/mods.rs`,
   the lobby's MODS page, `--pick-n P,H`, `crates/sim/tests/mods.rs`
   (vanilla checksums recorded before the change).
3. Rule mods: Deck_Customization, RoundsWithFriends'
   continue, AllThePatch (each fix a documented deviation from the
   original).
4. Client-side QoL: CardBarPatch, TabInfo, Infoholic, BetterChat,
   LobbyImprovements leftovers (no sim impact; online they stay per-player).
5. Card packs, one per slice: CardsPlus, RSCards, WillsWackyCards (curses,
   rerolls), PCE, Root_Cards, KeysCards, CR, ChaosPoppycarsCards. Card art is
   our own unless the licence allows reuse.
6. Maps: a reader for the MapsExtended map format that imports custom maps
   from the player's own downloads (like the game data, nothing shipped),
   then ZenithMaps through it, then the editor.

Each slice is recorded in `docs/STATUS.md` and NOTES.md when it lands.
