//! The transport interface every backend implements (loopback, UDP, Steam, a game's own).

use serde::Serialize;
use std::fmt;

/// Time in microseconds from an arbitrary start. Sessions and transports never read the
/// clock themselves: the caller passes `now`, so tests run on virtual time.
pub type Micros = u64;

/// A peer's address on a transport (a Steam ID, a UDP address packed into 64 bits, a
/// loopback index). Stable for the length of a session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct PeerId(pub u64);

impl fmt::Display for PeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "peer:{}", self.0)
    }
}

/// How a message travels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery {
    /// May be lost, duplicated or reordered; at most [`MAX_UNRELIABLE`] bytes.
    Unreliable,
    /// Arrives once, in send order per peer, any size (split and rebuilt by the transport).
    Reliable,
}

/// Largest unreliable message: fits one datagram on every common path (IPv6 minimum MTU
/// 1280 minus headers, Steam's own limit is larger).
pub const MAX_UNRELIABLE: usize = 1150;

/// A message received from a peer.
#[derive(Clone, Debug, PartialEq)]
pub struct Packet {
    pub from: PeerId,
    pub channel: u8,
    pub delivery: Delivery,
    pub data: Vec<u8>,
}

/// Counters a transport keeps (for reports and budgets).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct TransportStats {
    pub bytes_sent: u64,
    pub datagrams_sent: u64,
    pub bytes_received: u64,
    pub resends: u64,
}

/// Errors with stable codes (`net.*`).
#[derive(Clone, Debug, PartialEq)]
pub struct NetError {
    pub code: &'static str,
    pub message: String,
}

impl NetError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for NetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for NetError {}

/// Stable error codes.
pub mod codes {
    /// An unreliable message larger than [`super::MAX_UNRELIABLE`].
    pub const TOO_LARGE: &str = "net.too_large";
    /// The OS socket failed (bind, send).
    pub const IO: &str = "net.io";
    /// The peer is not known to this transport.
    pub const UNKNOWN_PEER: &str = "net.unknown_peer";
    /// A lobby operation failed (full, closed, not found).
    pub const LOBBY: &str = "net.lobby";
    /// A session message could not be read.
    pub const PROTOCOL: &str = "net.protocol";
}

/// A message transport between peers. Connectionless: a peer is reachable by its id, there is
/// no connect step (Steam's networking messages work the same way). Sessions notice silent
/// peers by time-outs.
///
/// Call order each frame: [`Transport::poll`] (reads the network, resends, sends acks), then
/// drain [`Transport::receive`], then [`Transport::send`] as needed.
pub trait Transport {
    /// This endpoint's own id.
    fn local(&self) -> PeerId;
    fn send(&mut self, to: PeerId, delivery: Delivery, channel: u8, data: &[u8]) -> Result<(), NetError>;
    fn poll(&mut self, now: Micros);
    fn receive(&mut self) -> Option<Packet>;
    fn stats(&self) -> TransportStats {
        TransportStats::default()
    }
}

/// The lowest layer: unreliable datagrams to and from peers. [`crate::Endpoint`] adds
/// reliability on top, so a new datagram backend only needs these three methods.
pub trait Datagram {
    fn local(&self) -> PeerId;
    fn send_to(&mut self, to: PeerId, bytes: &[u8]) -> Result<(), NetError>;
    fn recv_from(&mut self) -> Option<(PeerId, Vec<u8>)>;
}
