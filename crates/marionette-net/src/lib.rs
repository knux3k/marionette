//! Marionette co-op networking (decisions 0210 to 0213).
//!
//! Layers, each replaceable:
//! - [`Transport`]: messages between peers, reliable or not. Backends: [`Endpoint`] over any
//!   [`Datagram`] source ([`LoopbackNet`] with simulated latency and loss, [`UdpDatagram`]),
//!   or a backend with its own reliability (the optional `marionette-steam` crate).
//! - [`lockstep`]: host-stamped lockstep sessions with state hash checks, late join by
//!   snapshot and resync.
//! - [`sim`]: frames onto a [`marionette_core::Sim`] (per-player actions `p<slot>.<action>`,
//!   the `NetPlayers` resource, snapshots as bytes).
//! - [`Lobbies`]: gathering players; [`LocalLobbies`] in process.
//! - [`simulate()`]: a whole session in one process for tests and the `net_simulate`
//!   operation.
//!
//! Nothing here reads the clock: callers pass `now` in microseconds.

pub mod lobby;
pub mod lockstep;
pub mod loopback;
pub mod reliable;
pub mod sim;
pub mod simulate;
pub mod transport;
pub mod udp;
pub mod wire;

pub use lobby::{Lobbies, LobbyEvent, LobbyId, LobbyInfo, LocalLobbies, Visibility};
pub use lockstep::{Client, Frame, Host, LockstepConfig, RosterEvent, SessionEvent};
pub use loopback::{LinkConditions, LoopbackNet, LoopbackSocket};
pub use reliable::Endpoint;
pub use sim::{NetModule, NetPlayers, apply_frame, encode_snapshot, player_action, restore_snapshot};
pub use simulate::{PeerReport, SimulateOptions, SimulateReport, simulate};
pub use transport::{
    Datagram, Delivery, MAX_UNRELIABLE, Micros, NetError, Packet, PeerId, Transport, TransportStats, codes,
};
pub use udp::UdpDatagram;
