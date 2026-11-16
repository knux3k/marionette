//! Finding games on the local network: the host's [`LanBeacon`] says "here I am" once a second
//! as a UDP broadcast, and a player's [`LanBrowser`] lists the games it heard recently. Joining
//! is then by the address found (or typed). Text format: `TAG|name|port` in UTF-8, at most
//! [`MAX_BEACON`] bytes; a different tag is another game and is ignored.
//!
//! Without a broadcast route (some networks block it) nothing is found; joining by address
//! still works. Like the rest of the crate, callers pass `now` in microseconds.

use crate::transport::{Micros, NetError, codes};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};

/// Largest beacon accepted or sent (bytes).
pub const MAX_BEACON: usize = 512;
/// How often a beacon goes out.
pub const BEACON_EVERY: Micros = 1_000_000;
/// A game not heard from for this long is dropped from the list.
pub const FOUND_FOR: Micros = 4_000_000;

/// The host side: broadcasts `tag|name|port` every [`BEACON_EVERY`].
pub struct LanBeacon {
    socket: UdpSocket,
    to: SocketAddr,
    payload: Vec<u8>,
    next: Micros,
}

impl LanBeacon {
    /// A beacon broadcast to `discovery_port` on this network.
    pub fn broadcast(tag: &str, name: &str, game_port: u16, discovery_port: u16) -> Result<Self, NetError> {
        Self::to(
            tag,
            name,
            game_port,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::BROADCAST), discovery_port),
        )
    }

    /// A beacon sent to one address (tests on 127.0.0.1, or a known subnet broadcast).
    pub fn to(tag: &str, name: &str, game_port: u16, to: SocketAddr) -> Result<Self, NetError> {
        let io = |e: std::io::Error| NetError::new(codes::IO, format!("lan beacon: {e}"));
        let socket = UdpSocket::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0)).map_err(io)?;
        socket.set_broadcast(true).map_err(io)?;
        socket.set_nonblocking(true).map_err(io)?;
        let clean = |s: &str| s.replace('|', "/");
        let mut payload = format!("{}|{}|{game_port}", clean(tag), clean(name)).into_bytes();
        payload.truncate(MAX_BEACON);
        Ok(Self {
            socket,
            to,
            payload,
            next: 0,
        })
    }

    /// Sends when due. A failed send (no broadcast route) is ignored: joining by address works.
    pub fn poll(&mut self, now: Micros) {
        if now < self.next {
            return;
        }
        self.next = now + BEACON_EVERY;
        let _ = self.socket.send_to(&self.payload, self.to);
    }
}

/// A game heard on the network.
#[derive(Clone, Debug, PartialEq)]
pub struct LanGame {
    pub name: String,
    /// The sender's address with the game's port.
    pub address: SocketAddr,
    pub last_seen: Micros,
}

/// The player side: listens for beacons with one tag.
pub struct LanBrowser {
    socket: UdpSocket,
    tag: String,
    found: Vec<LanGame>,
    buf: [u8; MAX_BEACON],
}

impl LanBrowser {
    /// Listens on `discovery_port` on every interface. Only one browser per machine can hold
    /// the port; a second gets an error (and the player can still join by address).
    pub fn listen(tag: &str, discovery_port: u16) -> Result<Self, NetError> {
        Self::bind(
            tag,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), discovery_port),
        )
    }

    pub fn bind(tag: &str, addr: SocketAddr) -> Result<Self, NetError> {
        let io = |e: std::io::Error| NetError::new(codes::IO, format!("lan browser on {addr}: {e}"));
        let socket = UdpSocket::bind(addr).map_err(io)?;
        socket.set_nonblocking(true).map_err(io)?;
        Ok(Self {
            socket,
            tag: tag.replace('|', "/"),
            found: Vec::new(),
            buf: [0; MAX_BEACON],
        })
    }

    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.socket.local_addr().ok()
    }

    /// Reads waiting beacons and drops games not heard for [`FOUND_FOR`].
    pub fn poll(&mut self, now: Micros) {
        for _ in 0..64 {
            let (n, from) = match self.socket.recv_from(&mut self.buf) {
                Ok(got) => got,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => continue,
            };
            let Ok(text) = std::str::from_utf8(&self.buf[..n]) else {
                continue;
            };
            let mut parts = text.split('|');
            let (Some(tag), Some(name), Some(port)) = (parts.next(), parts.next(), parts.next()) else {
                continue;
            };
            let Ok(port) = port.parse::<u16>() else { continue };
            if tag != self.tag {
                continue;
            }
            let address = SocketAddr::new(from.ip(), port);
            match self.found.iter_mut().find(|g| g.address.ip() == from.ip()) {
                Some(g) => {
                    g.last_seen = now;
                    g.address = address;
                }
                None => self.found.push(LanGame {
                    name: name.to_string(),
                    address,
                    last_seen: now,
                }),
            }
        }
        self.found
            .retain(|g| now.saturating_sub(g.last_seen) <= FOUND_FOR);
    }

    /// Games heard recently, in the order first heard.
    pub fn found(&self) -> &[LanGame] {
        &self.found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_beacon_is_found_and_forgotten() {
        let mut browser = LanBrowser::bind("GAME", "127.0.0.1:0".parse().unwrap()).unwrap();
        let to = browser.local_addr().unwrap();
        let mut beacon = LanBeacon::to("GAME", "Sam|x", 7777, to).unwrap();
        let mut other = LanBeacon::to("OTHER", "Kim", 7000, to).unwrap();
        let mut now = 0;
        for _ in 0..200 {
            beacon.poll(now);
            other.poll(now);
            std::thread::sleep(std::time::Duration::from_millis(2));
            browser.poll(now);
            if !browser.found().is_empty() {
                break;
            }
            now += 10_000;
        }
        let found = browser.found();
        assert_eq!(found.len(), 1, "only our game's tag: {found:?}");
        assert_eq!(found[0].name, "Sam/x");
        assert_eq!(found[0].address.port(), 7777);
        browser.poll(now + FOUND_FOR + 1);
        assert!(browser.found().is_empty());
    }
}
