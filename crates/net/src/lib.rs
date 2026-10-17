//! # rounds-net
//!
//! Online play for rounds-port: GGRS rollback sessions whose packets travel
//! over Warpzone's encrypted peer channels, and a small host-authoritative
//! lobby/match protocol on the reliable channel. See `docs/net.md`.
//!
//! | Module | What |
//! |---|---|
//! | [`session`] | [`NetSession`]: rooms, lobby, match start/end, `advance()` |
//! | `matchrun` | GGRS sessions of a match, prediction limit, freeze/resume on drops |
//! | [`input`] | [`PlayerInput`] placeholder and the `Pod` → GGRS input adapter |
//! | [`wire`] | channels, message types, framing |
//! | [`lobby`] | host-side slot/colour/team assignment |
//! | [`transport`] | GGRS socket over Warpzone, host forwarding, link simulation |
//! | [`backend`] | Warpzone client or in-process [`MemHub`] rooms |

pub mod backend;
pub mod input;
pub mod lobby;
mod matchrun;
pub mod session;
pub mod toy;
pub mod transport;
pub mod wire;

pub use backend::{MemEndpoint, MemHub};
pub use input::{NetInput, PlayerInput};
pub use session::{
    GAME_ID, InputStatus, LaunchAction, MachineStats, NetConfig, NetError, NetEvent, NetSession,
    NetStats, NetworkStats, Request, StateCell,
};
pub use transport::{LinkSim, TransportStats};
pub use warpzone_core;
pub use wire::{
    DoneReport, GgrsParams, LobbyState, MAX_SLOTS, MachineId, MachineView, MatchSettings,
    MatchStart, Phase, PlayerSlot, SlotRequest, TeamMode,
};
