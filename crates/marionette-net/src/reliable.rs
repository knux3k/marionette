//! [`Endpoint`]: reliability on top of any [`Datagram`] backend. Unreliable messages pass
//! straight through; reliable ones get a per-peer sequence number, are split into
//! datagram-sized fragments, resent until acknowledged and delivered once, in order.
//!
//! Datagram layout (first byte is the kind):
//! - `0` unreliable: channel u8, payload
//! - `1` reliable fragment: seq u32, channel u8, flags u8 (bit 0: more fragments follow), payload
//! - `2` ack: next expected seq u32, bitmask u32 (bit i: seq next+1+i received)

use crate::transport::{
    Datagram, Delivery, MAX_UNRELIABLE, Micros, NetError, Packet, PeerId, Transport, TransportStats, codes,
};
use crate::wire::{Reader, Writer};
use std::collections::{BTreeMap, VecDeque};

const UNRELIABLE: u8 = 0;
const FRAGMENT: u8 = 1;
const ACK: u8 = 2;
/// Payload bytes per reliable fragment.
const FRAGMENT_PAYLOAD: usize = 1100;
/// Fragments in flight per peer before new ones wait. About 53 KB: below the smallest common
/// OS receive buffer (64 KB on Windows), so a burst is not dropped whole by the receiver.
const WINDOW: usize = 48;
/// Resends per peer per poll, oldest first: a lossy link is not flooded by a timer burst.
const MAX_RESENDS: usize = 16;
const RTO_START: Micros = 200_000;
const RTO_MIN: Micros = 40_000;
const RTO_MAX: Micros = 2_000_000;
/// Reliable bytes buffered per peer before `send` refuses (a peer that never acks).
const MAX_QUEUED: usize = 64 << 20;

struct Unacked {
    datagram: Vec<u8>,
    first_sent: Micros,
    last_sent: Micros,
    sends: u32,
}

#[derive(Default)]
struct Peer {
    // Sending side.
    next_seq: u32,
    unacked: BTreeMap<u32, Unacked>,
    queued: VecDeque<Vec<u8>>,
    queued_bytes: usize,
    srtt: Option<Micros>,
    // Receiving side.
    expected: u32,
    out_of_order: BTreeMap<u32, (u8, u8, Vec<u8>)>,
    partial: Vec<u8>,
    ack_owed: bool,
    last_heard: Option<Micros>,
}

impl Peer {
    fn rto(&self) -> Micros {
        self.srtt
            .map(|r| (r * 2 + 10_000).clamp(RTO_MIN, RTO_MAX))
            .unwrap_or(RTO_START)
    }
}

/// A [`Transport`] made from a datagram backend plus reliability.
pub struct Endpoint<D: Datagram> {
    socket: D,
    peers: BTreeMap<PeerId, Peer>,
    received: VecDeque<Packet>,
    now: Micros,
    stats: TransportStats,
}

impl<D: Datagram> Endpoint<D> {
    pub fn new(socket: D) -> Self {
        Self {
            socket,
            peers: BTreeMap::new(),
            received: VecDeque::new(),
            now: 0,
            stats: TransportStats::default(),
        }
    }

    pub fn socket(&self) -> &D {
        &self.socket
    }

    pub fn socket_mut(&mut self) -> &mut D {
        &mut self.socket
    }

    /// When a datagram last arrived from `peer` (for time-outs).
    pub fn last_heard(&self, peer: PeerId) -> Option<Micros> {
        self.peers.get(&peer).and_then(|p| p.last_heard)
    }

    /// Reliable fragments sent but not acknowledged yet, plus those waiting for the window.
    pub fn pending_reliable(&self, peer: PeerId) -> usize {
        self.peers
            .get(&peer)
            .map(|p| p.unacked.len() + p.queued.len())
            .unwrap_or(0)
    }

    fn raw_send(&mut self, to: PeerId, bytes: &[u8]) {
        self.stats.bytes_sent += bytes.len() as u64;
        self.stats.datagrams_sent += 1;
        // A datagram that cannot leave is the same as a lost one; reliability resends.
        let _ = self.socket.send_to(to, bytes);
    }

    fn pump_window(&mut self, to: PeerId) {
        let now = self.now;
        let mut sends = Vec::new();
        if let Some(p) = self.peers.get_mut(&to) {
            while p.unacked.len() < WINDOW {
                let Some(d) = p.queued.pop_front() else { break };
                p.queued_bytes -= d.len();
                let seq = u32::from_le_bytes(d[1..5].try_into().expect("fragment header"));
                sends.push(d.clone());
                p.unacked.insert(
                    seq,
                    Unacked {
                        datagram: d,
                        first_sent: now,
                        last_sent: now,
                        sends: 1,
                    },
                );
            }
        }
        for d in sends {
            self.raw_send(to, &d);
        }
    }

    fn on_ack(&mut self, from: PeerId, next: u32, mask: u32) {
        let now = self.now;
        let p = self.peers.entry(from).or_default();
        let done: Vec<u32> = p
            .unacked
            .keys()
            .copied()
            .filter(|&s| {
                s.wrapping_sub(next) > u32::MAX / 2 // s < next (wrapping)
                    || {
                        let d = s.wrapping_sub(next);
                        (1..=32).contains(&d) && mask & (1 << (d - 1)) != 0
                    }
            })
            .collect();
        for s in done {
            if let Some(u) = p.unacked.remove(&s) {
                // Karn: only time fragments sent once.
                if u.sends == 1 {
                    let sample = now.saturating_sub(u.first_sent);
                    p.srtt = Some(match p.srtt {
                        Some(r) => (r * 7 + sample) / 8,
                        None => sample,
                    });
                }
            }
        }
        self.pump_window(from);
    }

    fn on_fragment(&mut self, from: PeerId, seq: u32, channel: u8, flags: u8, payload: &[u8]) {
        let p = self.peers.entry(from).or_default();
        p.ack_owed = true;
        let ahead = seq.wrapping_sub(p.expected);
        if ahead > u32::MAX / 2 {
            return; // old duplicate; the ack above tells the sender again
        }
        if ahead as usize > 1024 {
            return; // far outside any window: hostile or broken, ignore
        }
        p.out_of_order
            .entry(seq)
            .or_insert((channel, flags, payload.to_vec()));
        while let Some((channel, flags, data)) = p.out_of_order.remove(&p.expected) {
            p.expected = p.expected.wrapping_add(1);
            p.partial.extend_from_slice(&data);
            if flags & 1 == 0 {
                let data = std::mem::take(&mut p.partial);
                self.received.push_back(Packet {
                    from,
                    channel,
                    delivery: Delivery::Reliable,
                    data,
                });
            }
        }
    }

    fn handle(&mut self, from: PeerId, bytes: &[u8]) {
        self.stats.bytes_received += bytes.len() as u64;
        let now = self.now;
        self.peers.entry(from).or_default().last_heard = Some(now);
        let mut r = Reader::new(bytes);
        match r.u8() {
            Some(UNRELIABLE) => {
                if let Some(channel) = r.u8() {
                    self.received.push_back(Packet {
                        from,
                        channel,
                        delivery: Delivery::Unreliable,
                        data: r.rest().to_vec(),
                    });
                }
            }
            Some(FRAGMENT) => {
                if let (Some(seq), Some(ch), Some(flags)) = (r.u32(), r.u8(), r.u8()) {
                    let payload = r.rest();
                    self.on_fragment(from, seq, ch, flags, payload);
                }
            }
            Some(ACK) => {
                if let (Some(next), Some(mask)) = (r.u32(), r.u32()) {
                    self.on_ack(from, next, mask);
                }
            }
            _ => {}
        }
    }
}

impl<D: Datagram> Transport for Endpoint<D> {
    fn local(&self) -> PeerId {
        self.socket.local()
    }

    fn send(&mut self, to: PeerId, delivery: Delivery, channel: u8, data: &[u8]) -> Result<(), NetError> {
        match delivery {
            Delivery::Unreliable => {
                if data.len() > MAX_UNRELIABLE {
                    return Err(NetError::new(
                        codes::TOO_LARGE,
                        format!(
                            "unreliable message of {} bytes; the limit is {MAX_UNRELIABLE}",
                            data.len()
                        ),
                    ));
                }
                let mut w = Writer::with(UNRELIABLE);
                w.u8(channel).raw(data);
                self.raw_send(to, &w.buf);
                Ok(())
            }
            Delivery::Reliable => {
                let p = self.peers.entry(to).or_default();
                if p.queued_bytes + data.len() > MAX_QUEUED {
                    return Err(NetError::new(
                        codes::TOO_LARGE,
                        format!("more than {MAX_QUEUED} reliable bytes waiting for {to}"),
                    ));
                }
                let chunks: Vec<&[u8]> = if data.is_empty() {
                    vec![&[][..]]
                } else {
                    data.chunks(FRAGMENT_PAYLOAD).collect()
                };
                let last = chunks.len() - 1;
                for (i, c) in chunks.into_iter().enumerate() {
                    let seq = p.next_seq;
                    p.next_seq = p.next_seq.wrapping_add(1);
                    let mut w = Writer::with(FRAGMENT);
                    w.u32(seq).u8(channel).u8(u8::from(i < last)).raw(c);
                    p.queued_bytes += w.buf.len();
                    p.queued.push_back(w.buf);
                }
                self.pump_window(to);
                Ok(())
            }
        }
    }

    fn poll(&mut self, now: Micros) {
        self.now = now;
        while let Some((from, bytes)) = self.socket.recv_from() {
            self.handle(from, &bytes);
        }
        // Resends and acks, peers in id order.
        let ids: Vec<PeerId> = self.peers.keys().copied().collect();
        for id in ids {
            let mut out = Vec::new();
            let p = self.peers.get_mut(&id).expect("listed");
            let rto = p.rto();
            for u in p.unacked.values_mut() {
                if out.len() >= MAX_RESENDS {
                    break;
                }
                let wait = rto.saturating_mul(1 << (u.sends - 1).min(4)).min(RTO_MAX);
                if now.saturating_sub(u.last_sent) >= wait {
                    u.last_sent = now;
                    u.sends += 1;
                    out.push(u.datagram.clone());
                }
            }
            let resends = out.len() as u64;
            if p.ack_owed {
                p.ack_owed = false;
                let mut mask = 0u32;
                for i in 0..32u32 {
                    if p.out_of_order.contains_key(&p.expected.wrapping_add(1 + i)) {
                        mask |= 1 << i;
                    }
                }
                let mut w = Writer::with(ACK);
                w.u32(p.expected).u32(mask);
                out.push(w.buf);
            }
            self.stats.resends += resends;
            for d in out {
                self.raw_send(id, &d);
            }
        }
    }

    fn receive(&mut self) -> Option<Packet> {
        self.received.pop_front()
    }

    fn stats(&self) -> TransportStats {
        self.stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loopback::{LinkConditions, LoopbackNet};

    #[test]
    fn reliable_survives_loss_reorder_and_duplicates() {
        let net = LoopbackNet::new(
            3,
            LinkConditions {
                latency_ms: 40.0,
                jitter_ms: 30.0,
                loss: 0.25,
                duplicate: 0.1,
            },
        );
        let mut a = Endpoint::new(net.endpoint());
        let mut b = Endpoint::new(net.endpoint());
        let big: Vec<u8> = (0..50_000u32).map(|i| (i % 251) as u8).collect();
        let msgs: Vec<Vec<u8>> = (0..40u8).map(|i| vec![i; i as usize]).collect();
        for m in &msgs {
            a.send(b.local(), Delivery::Reliable, 3, m).unwrap();
        }
        a.send(b.local(), Delivery::Reliable, 4, &big).unwrap();
        let mut got = Vec::new();
        for _ in 0..1000 {
            net.advance(10_000);
            let now = net.now();
            a.poll(now);
            b.poll(now);
            while let Some(p) = b.receive() {
                got.push(p);
            }
            if got.len() == msgs.len() + 1 {
                break;
            }
        }
        assert_eq!(got.len(), msgs.len() + 1, "everything arrives once");
        for (p, m) in got.iter().zip(&msgs) {
            assert_eq!(&p.data, m, "in order");
            assert_eq!(p.channel, 3);
        }
        assert_eq!(got.last().unwrap().data, big);
        assert!(a.stats().resends > 0);
        // Settle: every fragment is acknowledged in the end.
        for _ in 0..300 {
            net.advance(10_000);
            let now = net.now();
            a.poll(now);
            b.poll(now);
        }
        assert_eq!(a.pending_reliable(b.local()), 0);
    }

    #[test]
    fn unreliable_size_limit() {
        let net = LoopbackNet::new(1, LinkConditions::default());
        let mut a = Endpoint::new(net.endpoint());
        let to = PeerId(2);
        let _b = net.endpoint();
        let e = a
            .send(to, Delivery::Unreliable, 0, &vec![0; MAX_UNRELIABLE + 1])
            .unwrap_err();
        assert_eq!(e.code, codes::TOO_LARGE);
        a.send(to, Delivery::Unreliable, 0, &vec![0; MAX_UNRELIABLE])
            .unwrap();
    }
}
