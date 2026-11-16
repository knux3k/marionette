//! Hosted sessions: one host owns the game state and guests connect to it (star topology).
//! For games where the host runs the world and streams it, and guests send their own pose and
//! requests (the alternative to [`crate::lockstep`], decision 0214). Nothing here knows the
//! game: it does the handshake, keep-alives, time-outs, leaving and who is connected, and
//! hands every other message to the game as a [`HostedEvent::Message`].
//!
//! The control messages travel on [`CONTROL`] (channel 255); games use the other channels.
//! - `1` hello (guest to host, reliable): protocol text, player name
//! - `2` welcome (host to guest, reliable): host name
//! - `3` refuse (host to guest, reliable): reason
//! - `4` ping (either way, unreliable): keeps a quiet link alive
//! - `5` bye (either way, reliable): leaving on purpose
//!
//! Like the rest of the crate, nothing reads the clock: callers pass `now`.
//!
//! Ownership: [`HostedHost`] and [`HostedGuest`] own their tables and event queues; the
//! transport is borrowed (`&mut dyn Transport`) for each call, so one transport can be swapped
//! for another (UDP, Steam, loopback) without the session changing.

use crate::transport::{Delivery, Micros, NetError, Packet, PeerId, Transport, codes};
use crate::wire::{Reader, Writer};
use std::collections::{BTreeMap, VecDeque};

/// The channel the session's own messages use.
pub const CONTROL: u8 = 255;
/// Longest protocol text or player name accepted (bytes); longer ones are cut.
pub const MAX_NAME: usize = 64;

const HELLO: u8 = 1;
const WELCOME: u8 = 2;
const REFUSE: u8 = 3;
const PING: u8 = 4;
const BYE: u8 = 5;

/// Settings shared by host and guest.
#[derive(Clone, Debug, PartialEq)]
pub struct HostedConfig {
    /// Must be equal on host and guest (a game name and wire version, e.g. `"mygame/3"`);
    /// a guest with another protocol is refused.
    pub protocol: String,
    /// This player's name, shown to the others.
    pub name: String,
    /// Guests the host takes at once (more are refused).
    pub max_guests: usize,
    /// A ping goes out when nothing else was sent to a peer for this long.
    pub keepalive: Micros,
    /// A peer heard from for this long is gone (and a guest gives up connecting).
    pub timeout: Micros,
}

impl Default for HostedConfig {
    fn default() -> Self {
        Self {
            protocol: String::new(),
            name: "Player".into(),
            max_guests: 7,
            keepalive: 1_000_000,
            timeout: 10_000_000,
        }
    }
}

/// Why a peer is no longer in the session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeaveReason {
    /// Said bye (left on purpose).
    Left,
    /// Not heard from within the time-out.
    TimedOut,
    /// Removed by the host.
    Kicked,
}

/// What happened since the last call, in order.
#[derive(Clone, Debug, PartialEq)]
pub enum HostedEvent {
    /// Host: a guest connected. Guest: the host welcomed us (`peer` is the host).
    Joined { peer: PeerId, name: String },
    /// Host: a guest left. Guest: the host went away or closed the session.
    Left { peer: PeerId, reason: LeaveReason },
    /// Guest: the host refused us (wrong protocol, full).
    Refused { reason: String },
    /// A game message from a connected peer (never on [`CONTROL`]).
    Message(Packet),
}

fn short(text: &str) -> &str {
    if text.len() <= MAX_NAME {
        return text;
    }
    let mut end = MAX_NAME;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn text(r: &mut Reader) -> Option<String> {
    let b = r.bytes()?;
    let s = std::str::from_utf8(b).ok()?;
    Some(short(s).to_string())
}

fn control(kind: u8) -> Writer {
    Writer::with(kind)
}

struct Guest {
    name: String,
    last_heard: Micros,
    last_sent: Micros,
}

/// The host side of a hosted session.
pub struct HostedHost {
    cfg: HostedConfig,
    guests: BTreeMap<PeerId, Guest>,
    events: VecDeque<HostedEvent>,
    now: Micros,
}

impl HostedHost {
    pub fn new(cfg: HostedConfig) -> Self {
        Self {
            cfg,
            guests: BTreeMap::new(),
            events: VecDeque::new(),
            now: 0,
        }
    }

    pub fn config(&self) -> &HostedConfig {
        &self.cfg
    }

    /// Connected guests and their names, in id order.
    pub fn guests(&self) -> impl Iterator<Item = (PeerId, &str)> {
        self.guests.iter().map(|(p, g)| (*p, g.name.as_str()))
    }

    pub fn guest_count(&self) -> usize {
        self.guests.len()
    }

    pub fn is_guest(&self, peer: PeerId) -> bool {
        self.guests.contains_key(&peer)
    }

    /// Reads the transport, answers hellos, times out silent guests and pings quiet ones.
    /// Call once a frame after [`Transport::poll`], then drain [`HostedHost::event`].
    pub fn poll(&mut self, t: &mut dyn Transport, now: Micros) {
        self.now = now;
        while let Some(p) = t.receive() {
            if p.channel == CONTROL {
                self.on_control(t, p);
                continue;
            }
            if let Some(g) = self.guests.get_mut(&p.from) {
                g.last_heard = now;
                self.events.push_back(HostedEvent::Message(p));
            }
            // Anyone else has not said hello: ignored.
        }
        let gone: Vec<PeerId> = self
            .guests
            .iter()
            .filter(|(_, g)| now.saturating_sub(g.last_heard) > self.cfg.timeout)
            .map(|(p, _)| *p)
            .collect();
        for peer in gone {
            self.drop_guest(t, peer, LeaveReason::TimedOut);
        }
        let quiet: Vec<PeerId> = self
            .guests
            .iter()
            .filter(|(_, g)| now.saturating_sub(g.last_sent) >= self.cfg.keepalive)
            .map(|(p, _)| *p)
            .collect();
        for peer in quiet {
            let _ = t.send(peer, Delivery::Unreliable, CONTROL, &control(PING).buf);
            if let Some(g) = self.guests.get_mut(&peer) {
                g.last_sent = now;
            }
        }
    }

    fn on_control(&mut self, t: &mut dyn Transport, p: Packet) {
        let mut r = Reader::new(&p.data);
        match r.u8() {
            Some(HELLO) => {
                let (Some(protocol), Some(name)) = (text(&mut r), text(&mut r)) else {
                    return;
                };
                if let Some(g) = self.guests.get_mut(&p.from) {
                    // A repeated hello (the guest restarted its side): welcome again.
                    g.last_heard = self.now;
                    self.welcome(t, p.from);
                    return;
                }
                if protocol != self.cfg.protocol {
                    self.refuse(t, p.from, "another game version");
                    return;
                }
                if self.guests.len() >= self.cfg.max_guests {
                    self.refuse(t, p.from, "the game is full");
                    return;
                }
                self.guests.insert(
                    p.from,
                    Guest {
                        name: name.clone(),
                        last_heard: self.now,
                        last_sent: self.now,
                    },
                );
                self.welcome(t, p.from);
                self.events.push_back(HostedEvent::Joined { peer: p.from, name });
            }
            Some(PING) => {
                if let Some(g) = self.guests.get_mut(&p.from) {
                    g.last_heard = self.now;
                }
            }
            Some(BYE) if self.guests.contains_key(&p.from) => {
                self.drop_guest(t, p.from, LeaveReason::Left);
            }
            _ => {}
        }
    }

    fn welcome(&mut self, t: &mut dyn Transport, to: PeerId) {
        let mut w = control(WELCOME);
        w.bytes(short(&self.cfg.name).as_bytes());
        let _ = t.send(to, Delivery::Reliable, CONTROL, &w.buf);
    }

    fn refuse(&mut self, t: &mut dyn Transport, to: PeerId, reason: &str) {
        let mut w = control(REFUSE);
        w.bytes(reason.as_bytes());
        let _ = t.send(to, Delivery::Reliable, CONTROL, &w.buf);
    }

    fn drop_guest(&mut self, t: &mut dyn Transport, peer: PeerId, reason: LeaveReason) {
        if self.guests.remove(&peer).is_some() {
            t.forget(peer);
            self.events.push_back(HostedEvent::Left { peer, reason });
        }
    }

    /// Sends a game message to one connected guest.
    pub fn send(
        &mut self,
        t: &mut dyn Transport,
        to: PeerId,
        delivery: Delivery,
        channel: u8,
        data: &[u8],
    ) -> Result<(), NetError> {
        if channel == CONTROL {
            return Err(NetError::new(codes::PROTOCOL, "channel 255 is the session's own"));
        }
        let Some(g) = self.guests.get_mut(&to) else {
            return Err(NetError::new(
                codes::UNKNOWN_PEER,
                format!("{to} is not in the session"),
            ));
        };
        g.last_sent = self.now;
        t.send(to, delivery, channel, data)
    }

    /// Sends a game message to every connected guest but `except`. Returns the first error
    /// (the others are still sent).
    pub fn broadcast(
        &mut self,
        t: &mut dyn Transport,
        delivery: Delivery,
        channel: u8,
        data: &[u8],
        except: Option<PeerId>,
    ) -> Result<(), NetError> {
        let to: Vec<PeerId> = self
            .guests
            .keys()
            .copied()
            .filter(|p| Some(*p) != except)
            .collect();
        let mut first = Ok(());
        for peer in to {
            if let Err(e) = self.send(t, peer, delivery, channel, data)
                && first.is_ok()
            {
                first = Err(e);
            }
        }
        first
    }

    /// Removes a guest (it is told, then forgotten).
    pub fn kick(&mut self, t: &mut dyn Transport, peer: PeerId) {
        if self.guests.contains_key(&peer) {
            let _ = t.send(peer, Delivery::Reliable, CONTROL, &control(BYE).buf);
            self.drop_guest(t, peer, LeaveReason::Kicked);
        }
    }

    /// Ends the session: every guest is told. Keep polling the transport briefly so the byes
    /// are delivered.
    pub fn close(&mut self, t: &mut dyn Transport) {
        let all: Vec<PeerId> = self.guests.keys().copied().collect();
        for peer in all {
            let _ = t.send(peer, Delivery::Reliable, CONTROL, &control(BYE).buf);
        }
        self.guests.clear();
    }

    pub fn event(&mut self) -> Option<HostedEvent> {
        self.events.pop_front()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GuestState {
    New,
    Connecting { since: Micros },
    Connected,
    Closed,
}

/// The guest side of a hosted session.
pub struct HostedGuest {
    cfg: HostedConfig,
    host: PeerId,
    state: GuestState,
    host_name: String,
    last_heard: Micros,
    last_sent: Micros,
    now: Micros,
    events: VecDeque<HostedEvent>,
}

impl HostedGuest {
    /// A guest that says hello to `host` on its first [`HostedGuest::poll`].
    pub fn new(cfg: HostedConfig, host: PeerId) -> Self {
        Self {
            cfg,
            host,
            state: GuestState::New,
            host_name: String::new(),
            last_heard: 0,
            last_sent: 0,
            now: 0,
            events: VecDeque::new(),
        }
    }

    pub fn host(&self) -> PeerId {
        self.host
    }

    pub fn host_name(&self) -> &str {
        &self.host_name
    }

    pub fn is_connected(&self) -> bool {
        self.state == GuestState::Connected
    }

    /// True once refused, timed out, left, or the host closed the session.
    pub fn is_closed(&self) -> bool {
        self.state == GuestState::Closed
    }

    pub fn poll(&mut self, t: &mut dyn Transport, now: Micros) {
        self.now = now;
        if self.state == GuestState::New {
            let mut w = control(HELLO);
            w.bytes(short(&self.cfg.protocol).as_bytes())
                .bytes(short(&self.cfg.name).as_bytes());
            let _ = t.send(self.host, Delivery::Reliable, CONTROL, &w.buf);
            self.state = GuestState::Connecting { since: now };
            self.last_sent = now;
            self.last_heard = now;
        }
        while let Some(p) = t.receive() {
            if p.from != self.host || self.state == GuestState::Closed {
                continue;
            }
            self.last_heard = now;
            if p.channel != CONTROL {
                if self.state == GuestState::Connected {
                    self.events.push_back(HostedEvent::Message(p));
                }
                continue;
            }
            let mut r = Reader::new(&p.data);
            match r.u8() {
                Some(WELCOME) => {
                    self.host_name = text(&mut r).unwrap_or_default();
                    if self.state != GuestState::Connected {
                        self.state = GuestState::Connected;
                        self.events.push_back(HostedEvent::Joined {
                            peer: self.host,
                            name: self.host_name.clone(),
                        });
                    }
                }
                Some(REFUSE) => {
                    let reason = text(&mut r).unwrap_or_default();
                    self.close_with(t, None);
                    self.events.push_back(HostedEvent::Refused { reason });
                }
                Some(BYE) => {
                    self.close_with(t, Some(LeaveReason::Left));
                }
                _ => {}
            }
        }
        match self.state {
            GuestState::Connecting { since } if now.saturating_sub(since) > self.cfg.timeout => {
                self.close_with(t, None);
                self.events.push_back(HostedEvent::Refused {
                    reason: "no answer from the host".into(),
                });
            }
            GuestState::Connected if now.saturating_sub(self.last_heard) > self.cfg.timeout => {
                self.close_with(t, Some(LeaveReason::TimedOut));
            }
            GuestState::Connected if now.saturating_sub(self.last_sent) >= self.cfg.keepalive => {
                let _ = t.send(self.host, Delivery::Unreliable, CONTROL, &control(PING).buf);
                self.last_sent = now;
            }
            _ => {}
        }
    }

    fn close_with(&mut self, t: &mut dyn Transport, left: Option<LeaveReason>) {
        let was = self.state;
        self.state = GuestState::Closed;
        t.forget(self.host);
        if let (Some(reason), GuestState::Connected) = (left, was) {
            self.events.push_back(HostedEvent::Left {
                peer: self.host,
                reason,
            });
        }
    }

    /// Sends a game message to the host (only once connected).
    pub fn send(
        &mut self,
        t: &mut dyn Transport,
        delivery: Delivery,
        channel: u8,
        data: &[u8],
    ) -> Result<(), NetError> {
        if channel == CONTROL {
            return Err(NetError::new(codes::PROTOCOL, "channel 255 is the session's own"));
        }
        if self.state != GuestState::Connected {
            return Err(NetError::new(codes::UNKNOWN_PEER, "not connected to the host"));
        }
        self.last_sent = self.now;
        t.send(self.host, delivery, channel, data)
    }

    /// Leaves on purpose: the host is told. No event follows for our own leaving.
    pub fn leave(&mut self, t: &mut dyn Transport) {
        if matches!(self.state, GuestState::Connected | GuestState::Connecting { .. }) {
            let _ = t.send(self.host, Delivery::Reliable, CONTROL, &control(BYE).buf);
        }
        self.state = GuestState::Closed;
    }

    pub fn event(&mut self) -> Option<HostedEvent> {
        self.events.pop_front()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Endpoint, LinkConditions, LoopbackNet, LoopbackSocket};

    fn cfg(name: &str) -> HostedConfig {
        HostedConfig {
            protocol: "test/1".into(),
            name: name.into(),
            max_guests: 2,
            ..HostedConfig::default()
        }
    }

    struct Rig {
        net: LoopbackNet,
        ht: Endpoint<LoopbackSocket>,
        host: HostedHost,
    }

    impl Rig {
        fn new(loss: f64) -> Self {
            let net = LoopbackNet::new(
                9,
                LinkConditions {
                    latency_ms: 20.0,
                    jitter_ms: 5.0,
                    loss,
                    duplicate: 0.0,
                },
            );
            let ht = Endpoint::new(net.endpoint());
            Self {
                net,
                ht,
                host: HostedHost::new(cfg("Host")),
            }
        }

        fn guest(&self, c: HostedConfig) -> (Endpoint<LoopbackSocket>, HostedGuest) {
            let t = Endpoint::new(self.net.endpoint());
            let g = HostedGuest::new(c, self.ht.local());
            (t, g)
        }

        fn step(&mut self, guests: &mut [(&mut Endpoint<LoopbackSocket>, &mut HostedGuest)]) {
            self.net.advance(10_000);
            let now = self.net.now();
            self.ht.poll(now);
            self.host.poll(&mut self.ht, now);
            for (t, g) in guests.iter_mut() {
                t.poll(now);
                g.poll(*t, now);
            }
        }
    }

    #[test]
    fn join_message_and_leave() {
        let mut rig = Rig::new(0.1);
        let (mut gt, mut g) = rig.guest(cfg("Sam"));
        for _ in 0..100 {
            rig.step(&mut [(&mut gt, &mut g)]);
        }
        assert!(g.is_connected());
        assert_eq!(g.host_name(), "Host");
        assert_eq!(
            rig.host.event(),
            Some(HostedEvent::Joined {
                peer: gt.local(),
                name: "Sam".into()
            })
        );
        g.send(&mut gt, Delivery::Reliable, 3, b"hi").unwrap();
        rig.host
            .broadcast(&mut rig.ht, Delivery::Reliable, 4, b"world", None)
            .unwrap();
        let mut host_got = Vec::new();
        let mut guest_got = Vec::new();
        for _ in 0..100 {
            rig.step(&mut [(&mut gt, &mut g)]);
            while let Some(e) = rig.host.event() {
                host_got.push(e);
            }
            while let Some(e) = g.event() {
                guest_got.push(e);
            }
        }
        assert!(matches!(&host_got[..], [HostedEvent::Message(p)] if p.data == b"hi" && p.channel == 3));
        assert!(
            guest_got
                .iter()
                .any(|e| matches!(e, HostedEvent::Message(p) if p.data == b"world"))
        );
        g.leave(&mut gt);
        for _ in 0..100 {
            rig.step(&mut [(&mut gt, &mut g)]);
        }
        assert_eq!(
            rig.host.event(),
            Some(HostedEvent::Left {
                peer: gt.local(),
                reason: LeaveReason::Left
            })
        );
        assert_eq!(rig.host.guest_count(), 0);
    }

    #[test]
    fn wrong_protocol_and_full_are_refused() {
        let mut rig = Rig::new(0.0);
        let mut other = cfg("Old");
        other.protocol = "test/0".into();
        let (mut t1, mut g1) = rig.guest(other);
        let (mut t2, mut g2) = rig.guest(cfg("A"));
        let (mut t3, mut g3) = rig.guest(cfg("B"));
        let (mut t4, mut g4) = rig.guest(cfg("C"));
        for _ in 0..60 {
            rig.step(&mut [
                (&mut t1, &mut g1),
                (&mut t2, &mut g2),
                (&mut t3, &mut g3),
                (&mut t4, &mut g4),
            ]);
        }
        assert!(
            matches!(g1.event(), Some(HostedEvent::Refused { reason }) if reason == "another game version")
        );
        // Which two get in depends on arrival order; the third is told the game is full.
        let mut refused = Vec::new();
        for g in [&mut g2, &mut g3, &mut g4] {
            while let Some(e) = g.event() {
                if let HostedEvent::Refused { reason } = e {
                    refused.push(reason);
                }
            }
        }
        assert_eq!(refused, vec!["the game is full".to_string()]);
        assert_eq!([&g2, &g3, &g4].iter().filter(|g| g.is_connected()).count(), 2);
        assert_eq!(rig.host.guest_count(), 2);
    }

    #[test]
    fn silence_times_out_both_ways() {
        let mut rig = Rig::new(0.0);
        let (mut gt, mut g) = rig.guest(cfg("Sam"));
        for _ in 0..50 {
            rig.step(&mut [(&mut gt, &mut g)]);
        }
        assert!(g.is_connected());
        let _ = rig.host.event();
        let _ = g.event();
        // Keep-alives hold a quiet link open.
        for _ in 0..1500 {
            rig.step(&mut [(&mut gt, &mut g)]);
        }
        assert!(g.is_connected() && rig.host.guest_count() == 1);
        // The guest's machine goes away: the host notices, and the guest does too.
        rig.net.set_down(gt.local(), true);
        for _ in 0..1100 {
            rig.step(&mut [(&mut gt, &mut g)]);
        }
        assert_eq!(
            rig.host.event(),
            Some(HostedEvent::Left {
                peer: gt.local(),
                reason: LeaveReason::TimedOut
            })
        );
        assert!(g.is_closed());
        assert_eq!(
            g.event(),
            Some(HostedEvent::Left {
                peer: rig.ht.local(),
                reason: LeaveReason::TimedOut
            })
        );
    }

    #[test]
    fn host_close_tells_the_guest() {
        let mut rig = Rig::new(0.0);
        let (mut gt, mut g) = rig.guest(cfg("Sam"));
        for _ in 0..50 {
            rig.step(&mut [(&mut gt, &mut g)]);
        }
        let _ = g.event();
        rig.host.close(&mut rig.ht);
        for _ in 0..50 {
            rig.step(&mut [(&mut gt, &mut g)]);
        }
        assert_eq!(
            g.event(),
            Some(HostedEvent::Left {
                peer: rig.ht.local(),
                reason: LeaveReason::Left
            })
        );
    }

    #[test]
    fn unknown_peers_and_bad_bytes_are_ignored() {
        let mut rig = Rig::new(0.0);
        let mut stranger = Endpoint::new(rig.net.endpoint());
        stranger
            .send(rig.ht.local(), Delivery::Reliable, 1, b"not hello")
            .unwrap();
        stranger
            .send(rig.ht.local(), Delivery::Reliable, CONTROL, &[HELLO, 200, 0])
            .unwrap();
        for _ in 0..30 {
            rig.net.advance(10_000);
            let now = rig.net.now();
            stranger.poll(now);
            rig.ht.poll(now);
            rig.host.poll(&mut rig.ht, now);
        }
        assert_eq!(rig.host.event(), None);
        assert_eq!(rig.host.guest_count(), 0);
    }
}
