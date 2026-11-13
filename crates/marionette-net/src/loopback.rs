//! In-process datagram network with simulated latency, jitter, loss and duplication, on
//! virtual time. Deterministic: the same seed and the same calls give the same deliveries.
//! Used by tests, `net_simulate` and local split-screen style play.

use crate::transport::{Datagram, Micros, NetError, PeerId, codes};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

/// How a simulated link behaves (each direction separately, same settings).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct LinkConditions {
    /// One-way delay in milliseconds.
    pub latency_ms: f64,
    /// Extra random delay, uniform in 0..jitter_ms (reorders packets).
    pub jitter_ms: f64,
    /// Chance that a datagram is lost, 0..1.
    pub loss: f64,
    /// Chance that a datagram arrives twice, 0..1.
    pub duplicate: f64,
}

impl Default for LinkConditions {
    fn default() -> Self {
        Self {
            latency_ms: 0.0,
            jitter_ms: 0.0,
            loss: 0.0,
            duplicate: 0.0,
        }
    }
}

/// SplitMix64: small, seeded, the same everywhere.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Mix(pub u64);

impl Mix {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Uniform in 0..1.
    pub fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

struct InFlight {
    from: PeerId,
    to: PeerId,
    bytes: Vec<u8>,
}

struct Hub {
    now: Micros,
    rng: Mix,
    conditions: LinkConditions,
    /// Per-link overrides, (from, to).
    links: BTreeMap<(PeerId, PeerId), LinkConditions>,
    /// Ordered by delivery time, then send order: deterministic.
    flight: BTreeMap<(Micros, u64), InFlight>,
    order: u64,
    inbox: BTreeMap<PeerId, Vec<(PeerId, Vec<u8>)>>,
    down: BTreeSet<PeerId>,
    next_id: u64,
}

/// The shared network. Clone it to hand to each endpoint's creator.
#[derive(Clone)]
pub struct LoopbackNet(Arc<Mutex<Hub>>);

impl LoopbackNet {
    pub fn new(seed: u64, conditions: LinkConditions) -> Self {
        Self(Arc::new(Mutex::new(Hub {
            now: 0,
            rng: Mix(seed),
            conditions,
            links: BTreeMap::new(),
            flight: BTreeMap::new(),
            order: 0,
            inbox: BTreeMap::new(),
            down: BTreeSet::new(),
            next_id: 1,
        })))
    }

    fn hub(&self) -> std::sync::MutexGuard<'_, Hub> {
        // A panic in another test thread must not poison every later call.
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A new endpoint with the next free id (1, 2, 3...).
    pub fn endpoint(&self) -> LoopbackSocket {
        let mut h = self.hub();
        let id = PeerId(h.next_id);
        h.next_id += 1;
        h.inbox.insert(id, Vec::new());
        LoopbackSocket {
            net: self.clone(),
            id,
        }
    }

    /// Current virtual time.
    pub fn now(&self) -> Micros {
        self.hub().now
    }

    /// Moves virtual time forward and delivers everything due.
    pub fn advance(&self, by: Micros) {
        let mut h = self.hub();
        h.now += by;
        let now = h.now;
        let due: Vec<(Micros, u64)> = h.flight.range(..(now + 1, 0)).map(|(k, _)| *k).collect();
        for k in due {
            let p = h.flight.remove(&k).expect("key listed");
            if h.down.contains(&p.to) {
                continue;
            }
            if let Some(inbox) = h.inbox.get_mut(&p.to) {
                inbox.push((p.from, p.bytes));
            }
        }
    }

    /// Different conditions for one direction of one link.
    pub fn set_link(&self, from: PeerId, to: PeerId, c: LinkConditions) {
        self.hub().links.insert((from, to), c);
    }

    /// Cuts a peer off (as if its game closed): nothing reaches it or leaves it.
    pub fn set_down(&self, peer: PeerId, down: bool) {
        let mut h = self.hub();
        if down {
            h.down.insert(peer);
            h.inbox.entry(peer).or_default().clear();
        } else {
            h.down.remove(&peer);
        }
    }
}

/// One endpoint on a [`LoopbackNet`].
pub struct LoopbackSocket {
    net: LoopbackNet,
    id: PeerId,
}

impl Datagram for LoopbackSocket {
    fn local(&self) -> PeerId {
        self.id
    }

    fn send_to(&mut self, to: PeerId, bytes: &[u8]) -> Result<(), NetError> {
        let mut h = self.net.hub();
        if !h.inbox.contains_key(&to) {
            return Err(NetError::new(
                codes::UNKNOWN_PEER,
                format!("no loopback endpoint {to}"),
            ));
        }
        if h.down.contains(&self.id) {
            return Ok(());
        }
        let c = h.links.get(&(self.id, to)).copied().unwrap_or(h.conditions);
        let copies = if h.rng.unit() < c.loss {
            0
        } else if h.rng.unit() < c.duplicate {
            2
        } else {
            1
        };
        for _ in 0..copies {
            let jitter = c.jitter_ms * h.rng.unit();
            let delay = ((c.latency_ms + jitter) * 1000.0).max(0.0) as Micros;
            let at = h.now + delay;
            let order = h.order;
            h.order += 1;
            h.flight.insert(
                (at, order),
                InFlight {
                    from: self.id,
                    to,
                    bytes: bytes.to_vec(),
                },
            );
        }
        Ok(())
    }

    fn recv_from(&mut self) -> Option<(PeerId, Vec<u8>)> {
        let mut h = self.net.hub();
        let inbox = h.inbox.get_mut(&self.id)?;
        if inbox.is_empty() {
            None
        } else {
            Some(inbox.remove(0))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latency_loss_and_determinism() {
        let run = |seed| {
            let net = LoopbackNet::new(
                seed,
                LinkConditions {
                    latency_ms: 50.0,
                    jitter_ms: 20.0,
                    loss: 0.2,
                    duplicate: 0.1,
                },
            );
            let mut a = net.endpoint();
            let mut b = net.endpoint();
            for i in 0..200u8 {
                a.send_to(b.local(), &[i]).unwrap();
            }
            net.advance(49_000);
            assert!(b.recv_from().is_none(), "nothing before the latency");
            net.advance(30_000);
            let mut got = Vec::new();
            while let Some((from, d)) = b.recv_from() {
                assert_eq!(from, a.local());
                got.push(d[0]);
            }
            got
        };
        let x = run(7);
        assert_eq!(x, run(7));
        assert_ne!(x, run(8));
        assert!(
            x.len() > 120 && x.len() < 220,
            "about 80% arrive, some twice: {}",
            x.len()
        );
        assert!(x.windows(2).any(|w| w[0] > w[1]), "jitter reorders");
    }
}
