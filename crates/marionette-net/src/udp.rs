//! Plain UDP datagrams (LAN play, dedicated hosts, CI tests on 127.0.0.1). Wrap in
//! [`crate::Endpoint`] for reliable messages. Peer ids are the address packed into 64 bits
//! (IPv4: address << 16 | port), so both sides agree without a handshake.

use crate::transport::{Datagram, NetError, PeerId, codes};
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};

pub struct UdpDatagram {
    socket: UdpSocket,
    local: PeerId,
    /// IPv6 peers get ids from this table (IPv4 ids are the address itself).
    v6: BTreeMap<PeerId, SocketAddr>,
    buf: Vec<u8>,
}

/// The id of an address. IPv6 addresses get a hash-based id kept in a table.
pub fn peer_of(addr: SocketAddr) -> PeerId {
    match addr.ip() {
        IpAddr::V4(ip) => PeerId(((u32::from(ip) as u64) << 16) | addr.port() as u64),
        IpAddr::V6(ip) => {
            // FNV-1a over the address and port; top bit set so it never looks like IPv4.
            let mut h: u64 = 0xcbf2_9ce4_8422_2325;
            for b in ip.octets().iter().chain(addr.port().to_le_bytes().iter()) {
                h = (h ^ *b as u64).wrapping_mul(0x100_0000_01b3);
            }
            PeerId(h | 1 << 63)
        }
    }
}

/// The IPv4 address of an id made by [`peer_of`].
pub fn addr_of(peer: PeerId) -> Option<SocketAddr> {
    if peer.0 >> 48 != 0 {
        return None;
    }
    let ip = Ipv4Addr::from((peer.0 >> 16) as u32);
    Some(SocketAddr::new(IpAddr::V4(ip), (peer.0 & 0xffff) as u16))
}

impl UdpDatagram {
    /// Binds `addr` (port 0 picks a free port) in non-blocking mode.
    pub fn bind(addr: SocketAddr) -> Result<Self, NetError> {
        let io = |e: std::io::Error| NetError::new(codes::IO, format!("udp {addr}: {e}"));
        let socket = UdpSocket::bind(addr).map_err(io)?;
        socket.set_nonblocking(true).map_err(io)?;
        let local = peer_of(socket.local_addr().map_err(io)?);
        Ok(Self {
            socket,
            local,
            v6: BTreeMap::new(),
            buf: vec![0; 65536],
        })
    }

    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.socket.local_addr().ok()
    }

    /// Makes an IPv6 peer reachable by id (IPv4 peers need nothing).
    pub fn add_peer(&mut self, addr: SocketAddr) -> PeerId {
        let id = peer_of(addr);
        if addr.is_ipv6() {
            self.v6.insert(id, addr);
        }
        id
    }
}

impl Datagram for UdpDatagram {
    fn local(&self) -> PeerId {
        self.local
    }

    fn send_to(&mut self, to: PeerId, bytes: &[u8]) -> Result<(), NetError> {
        let addr = addr_of(to)
            .or_else(|| self.v6.get(&to).copied())
            .ok_or_else(|| NetError::new(codes::UNKNOWN_PEER, format!("no address for {to}")))?;
        match self.socket.send_to(bytes, addr) {
            Ok(_) => Ok(()),
            // A full send buffer is a lost datagram, not an error.
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(()),
            Err(e) => Err(NetError::new(codes::IO, format!("udp send to {addr}: {e}"))),
        }
    }

    fn recv_from(&mut self) -> Option<(PeerId, Vec<u8>)> {
        for _ in 0..64 {
            match self.socket.recv_from(&mut self.buf) {
                Ok((n, addr)) => {
                    let id = peer_of(addr);
                    if addr.is_ipv6() {
                        self.v6.insert(id, addr);
                    }
                    return Some((id, self.buf[..n].to_vec()));
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return None,
                // Windows reports an earlier send's ICMP "port unreachable" here; skip it.
                Err(_) => continue,
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip() {
        let a: SocketAddr = "192.168.1.20:27015".parse().unwrap();
        assert_eq!(addr_of(peer_of(a)), Some(a));
        let b: SocketAddr = "[::1]:9000".parse().unwrap();
        assert_eq!(addr_of(peer_of(b)), None);
    }
}
