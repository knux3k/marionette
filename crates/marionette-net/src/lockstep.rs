//! Host-stamped lockstep (decision 0210).
//!
//! Every peer runs the same deterministic simulation. Only inputs travel. The host owns the
//! order: each tick it builds a [`Frame`] (its own input changes, the clients' input changes
//! that have arrived, roster changes) and sends it to every client; clients run frames in
//! order and never guess. So the host never waits for a slow client, a client's input
//! arrives one round trip later, and joins, leaves and resyncs are just frame contents.
//!
//! State hashes are compared every `hash_interval` ticks: clients send theirs to the host,
//! the host reports a [`SessionEvent::Desync`] with the slot and tick, and can resend a
//! snapshot ([`Host::resync`]).
//!
//! Inputs are action changes (`action index`, exact `f32` bits). Frames and inputs travel
//! unreliably and redundantly (everything not yet acknowledged, in every packet), so one
//! lost packet costs nothing; welcomes, snapshots and hashes travel reliably.
//!
//! Wire messages (first byte is the kind):
//! - channel 0, unreliable: `FRAMES` host to client, `INPUTS` client to host;
//! - channel 1, reliable: `HELLO`, `WELCOME`, `HASH`, `BYE`.

use crate::transport::{Delivery, MAX_UNRELIABLE, Micros, PeerId, Transport};
use crate::wire::{Reader, Writer};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

const FRAMES: u8 = 1;
const INPUTS: u8 = 2;
const HELLO: u8 = 3;
const WELCOME: u8 = 4;
const HASH: u8 = 5;
const BYE: u8 = 6;

const CH_FAST: u8 = 0;
const CH_CONTROL: u8 = 1;
/// Frames kept for clients that have not acknowledged them; a client further behind is
/// dropped (about 18 minutes at 60 ticks per second).
const MAX_HISTORY: u64 = 65_536;

/// Session settings. Host and clients must use the same `actions` list (same order).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct LockstepConfig {
    /// Action names that travel, in a fixed order (index = position).
    pub actions: Vec<String>,
    /// Compare state hashes every this many ticks (0 = never).
    pub hash_interval: u64,
    /// A peer silent this long (microseconds) has left.
    pub timeout_us: Micros,
    /// Players including the host.
    pub max_players: u8,
    /// Let peers that were not in the starting list join a running session.
    pub late_join: bool,
}

impl Default for LockstepConfig {
    fn default() -> Self {
        Self {
            actions: Vec::new(),
            hash_interval: 60,
            timeout_us: 10_000_000,
            max_players: 8,
            late_join: true,
        }
    }
}

/// Who joined or left at a frame. Slot 0 is the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum RosterEvent {
    Join { slot: u8, peer: PeerId },
    Leave { slot: u8 },
}

/// Everything that happens at one tick, identical on every peer.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Frame {
    pub tick: u64,
    pub roster: Vec<RosterEvent>,
    /// (slot, action index, value), in stamping order.
    pub inputs: Vec<(u8, u16, f32)>,
}

/// Something the game should know about.
#[derive(Clone, Debug, PartialEq)]
pub enum SessionEvent {
    /// Client: the host accepted us. With a snapshot, restore it and run frames from
    /// `start_tick`; without, the session starts from the shared scene at `start_tick`.
    Welcomed {
        slot: u8,
        start_tick: u64,
        snapshot: Option<Vec<u8>>,
    },
    /// Host: a slot's state hash differed from the host's at `tick`.
    Desync { slot: u8, tick: u64 },
    /// Host: a peer stopped answering or said goodbye (its `Leave` is in the next frame).
    PeerLost { slot: u8, peer: PeerId },
    /// Client: the host stopped answering.
    HostLost,
}

fn encode_frame(w: &mut Writer, f: &Frame) {
    w.u8(f.roster.len() as u8);
    for r in &f.roster {
        match *r {
            RosterEvent::Join { slot, peer } => {
                w.u8(0).u8(slot).u64(peer.0);
            }
            RosterEvent::Leave { slot } => {
                w.u8(1).u8(slot);
            }
        }
    }
    w.u16(f.inputs.len() as u16);
    for &(slot, a, v) in &f.inputs {
        w.u8(slot).u16(a).f32(v);
    }
}

fn decode_frame(r: &mut Reader, tick: u64) -> Option<Frame> {
    let mut f = Frame {
        tick,
        ..Frame::default()
    };
    for _ in 0..r.u8()? {
        f.roster.push(match r.u8()? {
            0 => RosterEvent::Join {
                slot: r.u8()?,
                peer: PeerId(r.u64()?),
            },
            1 => RosterEvent::Leave { slot: r.u8()? },
            _ => return None,
        });
    }
    for _ in 0..r.u16()? {
        f.inputs.push((r.u8()?, r.u16()?, r.f32()?));
    }
    Some(f)
}

/// Action changes since the last call, as (index, value), indexes ascending.
fn diff(
    cfg: &LockstepConfig,
    last: &mut BTreeMap<u16, f32>,
    values: &BTreeMap<String, f32>,
) -> Vec<(u16, f32)> {
    let mut out = Vec::new();
    for (i, name) in cfg.actions.iter().enumerate() {
        let v = values.get(name).copied().unwrap_or(0.0);
        let old = last.get(&(i as u16)).copied().unwrap_or(0.0);
        if v.to_bits() != old.to_bits() {
            last.insert(i as u16, v);
            out.push((i as u16, v));
        }
    }
    out
}

/// Moves whole batches into the frame until one would change an action already changed in
/// this frame; that batch waits for the next frame, so a press and release in quick
/// succession are never merged away.
fn stamp(batches: &mut VecDeque<Vec<(u16, f32)>>, slot: u8, out: &mut Vec<(u8, u16, f32)>) {
    let mut used = BTreeSet::new();
    while let Some(b) = batches.front() {
        if b.iter().any(|(a, _)| used.contains(a)) {
            break;
        }
        for &(a, v) in batches.pop_front().expect("front exists").iter() {
            used.insert(a);
            out.push((slot, a, v));
        }
    }
}

struct Remote {
    slot: u8,
    /// Lowest tick the client has not confirmed.
    needed: u64,
    /// Next input batch number expected.
    next_seq: u32,
    early: BTreeMap<u32, Vec<(u16, f32)>>,
    ready: VecDeque<Vec<(u16, f32)>>,
    last_heard: Option<Micros>,
}

/// The host side. Slot 0 is the host's own player.
pub struct Host {
    cfg: LockstepConfig,
    local: PeerId,
    next_tick: u64,
    local_last: BTreeMap<u16, f32>,
    local_batches: VecDeque<Vec<(u16, f32)>>,
    remotes: BTreeMap<PeerId, Remote>,
    slots: BTreeMap<u8, PeerId>,
    frames: BTreeMap<u64, Vec<u8>>,
    joins: Vec<PeerId>,
    leaves: Vec<u8>,
    awaiting_snapshot: Vec<PeerId>,
    own_hashes: BTreeMap<u64, Vec<u8>>,
    their_hashes: BTreeMap<(u64, u8), Vec<u8>>,
    events: VecDeque<SessionEvent>,
    outbox: Vec<(PeerId, Vec<u8>)>,
    now: Micros,
    pending_first: Option<Vec<RosterEvent>>,
}

impl Host {
    /// A session starting at `start_tick` with `clients` (for example the lobby members),
    /// who all load the same scene and seed. They get slots 1, 2, ... in the order given.
    pub fn new(cfg: LockstepConfig, local: PeerId, start_tick: u64, clients: &[PeerId]) -> Self {
        let mut h = Self {
            cfg,
            local,
            next_tick: start_tick,
            local_last: BTreeMap::new(),
            local_batches: VecDeque::new(),
            remotes: BTreeMap::new(),
            slots: BTreeMap::from([(0, local)]),
            frames: BTreeMap::new(),
            joins: Vec::new(),
            leaves: Vec::new(),
            awaiting_snapshot: Vec::new(),
            own_hashes: BTreeMap::new(),
            their_hashes: BTreeMap::new(),
            events: VecDeque::new(),
            outbox: Vec::new(),
            now: 0,
            pending_first: None,
        };
        let mut first = Vec::new();
        first.push(RosterEvent::Join { slot: 0, peer: local });
        for &c in clients {
            if c == local || h.remotes.contains_key(&c) {
                continue;
            }
            let Some(slot) = h.free_slot() else { break };
            h.add_remote(c, slot, start_tick);
            first.push(RosterEvent::Join { slot, peer: c });
            let w = h.welcome(slot, start_tick, None);
            h.outbox.push((c, w));
        }
        h.pending_first = Some(first);
        h
    }

    fn free_slot(&self) -> Option<u8> {
        (0..self.cfg.max_players).find(|s| !self.slots.contains_key(s))
    }

    fn add_remote(&mut self, peer: PeerId, slot: u8, needed: u64) {
        self.slots.insert(slot, peer);
        self.remotes.insert(
            peer,
            Remote {
                slot,
                needed,
                next_seq: 0,
                early: BTreeMap::new(),
                ready: VecDeque::new(),
                last_heard: None,
            },
        );
    }

    fn welcome(&self, slot: u8, start_tick: u64, snapshot: Option<&[u8]>) -> Vec<u8> {
        let mut w = Writer::with(WELCOME);
        w.u8(slot).u64(start_tick);
        match snapshot {
            Some(s) => {
                w.u8(1).bytes(s);
            }
            None => {
                w.u8(0);
            }
        }
        w.buf
    }

    /// The tick the next [`Host::next_frame`] will be for.
    pub fn next_tick(&self) -> u64 {
        self.next_tick
    }

    /// Occupied slots and their peers (slot 0 is the host).
    pub fn slots(&self) -> &BTreeMap<u8, PeerId> {
        &self.slots
    }

    /// The host player's current action values; changes go into the next frame.
    pub fn set_local(&mut self, values: &BTreeMap<String, f32>) {
        let d = diff(&self.cfg, &mut self.local_last, values);
        if !d.is_empty() {
            self.local_batches.push_back(d);
        }
    }

    /// Reads the network: client inputs, hellos, hashes, goodbyes; notices silent clients.
    pub fn poll(&mut self, t: &mut dyn Transport, now: Micros) {
        self.now = now;
        t.poll(now);
        while let Some(p) = t.receive() {
            self.handle(p.from, p.channel, &p.data);
        }
        let timeout = self.cfg.timeout_us;
        let silent: Vec<PeerId> = self
            .remotes
            .iter()
            .filter(|(_, r)| match r.last_heard {
                Some(h) => now.saturating_sub(h) > timeout,
                None => false,
            })
            .map(|(p, _)| *p)
            .collect();
        for p in silent {
            self.drop_peer(p);
        }
        // Clients from the starting list that never speak are timed from the first poll.
        for r in self.remotes.values_mut() {
            r.last_heard.get_or_insert(now);
        }
    }

    fn drop_peer(&mut self, peer: PeerId) {
        if let Some(r) = self.remotes.remove(&peer) {
            self.slots.remove(&r.slot);
            self.leaves.push(r.slot);
            self.awaiting_snapshot.retain(|p| *p != peer);
            self.events
                .push_back(SessionEvent::PeerLost { slot: r.slot, peer });
        }
    }

    fn handle(&mut self, from: PeerId, channel: u8, data: &[u8]) {
        let now = self.now;
        let mut r = Reader::new(data);
        let kind = r.u8();
        if let Some(rem) = self.remotes.get_mut(&from) {
            rem.last_heard = Some(now);
        }
        match (channel, kind) {
            (CH_FAST, Some(INPUTS)) => {
                let Some(rem) = self.remotes.get_mut(&from) else {
                    return;
                };
                let (Some(needed), Some(first), Some(count)) = (r.u64(), r.u32(), r.u16()) else {
                    return;
                };
                rem.needed = rem.needed.max(needed);
                for i in 0..count as u32 {
                    let Some(n) = r.u16() else { return };
                    let mut batch = Vec::with_capacity(n as usize);
                    for _ in 0..n {
                        let (Some(a), Some(v)) = (r.u16(), r.f32()) else {
                            return;
                        };
                        batch.push((a, v));
                    }
                    let seq = first.wrapping_add(i);
                    if seq.wrapping_sub(rem.next_seq) < u32::MAX / 2 {
                        rem.early.entry(seq).or_insert(batch);
                    }
                }
                while let Some(b) = rem.early.remove(&rem.next_seq) {
                    rem.ready.push_back(b);
                    rem.next_seq = rem.next_seq.wrapping_add(1);
                }
            }
            (CH_CONTROL, Some(HELLO)) => {
                if !self.remotes.contains_key(&from)
                    && from != self.local
                    && self.cfg.late_join
                    && !self.joins.contains(&from)
                {
                    self.joins.push(from);
                }
            }
            (CH_CONTROL, Some(HASH)) => {
                let Some(slot) = self.remotes.get(&from).map(|r| r.slot) else {
                    return;
                };
                let (Some(tick), Some(h)) = (r.u64(), r.bytes()) else {
                    return;
                };
                self.their_hashes.insert((tick, slot), h.to_vec());
                self.compare_hashes();
            }
            (CH_CONTROL, Some(BYE)) => self.drop_peer(from),
            _ => {}
        }
    }

    fn compare_hashes(&mut self) {
        let keys: Vec<(u64, u8)> = self.their_hashes.keys().copied().collect();
        for (tick, slot) in keys {
            if let Some(own) = self.own_hashes.get(&tick) {
                let theirs = self.their_hashes.remove(&(tick, slot)).expect("listed");
                if &theirs != own {
                    self.events.push_back(SessionEvent::Desync { slot, tick });
                }
            } else if self.own_hashes.keys().next().is_some_and(|&first| tick < first) {
                self.their_hashes.remove(&(tick, slot)); // too old to check
            }
        }
    }

    /// Whether to hash after running the frame for `tick - 1` (call with the sim's tick).
    pub fn should_hash(&self, tick: u64) -> bool {
        self.cfg.hash_interval > 0 && tick.is_multiple_of(self.cfg.hash_interval)
    }

    /// The host's own state hash at `tick` (the sim's tick after stepping).
    pub fn record_hash(&mut self, tick: u64, hash: &[u8]) {
        self.own_hashes.insert(tick, hash.to_vec());
        while self.own_hashes.len() > 64 {
            self.own_hashes.pop_first();
        }
        self.compare_hashes();
    }

    /// Builds the frame for [`Host::next_tick`]. Run it on the host's sim like any other
    /// frame, then call [`Host::flush`]. If [`Host::needs_snapshot`] is true afterwards,
    /// pass a snapshot of the sim (now at the next tick) to [`Host::provide_snapshot`].
    pub fn next_frame(&mut self) -> Frame {
        let tick = self.next_tick;
        let mut f = Frame {
            tick,
            ..Frame::default()
        };
        if let Some(first) = self.pending_first.take() {
            f.roster.extend(first);
        }
        for slot in std::mem::take(&mut self.leaves) {
            f.roster.push(RosterEvent::Leave { slot });
        }
        for peer in std::mem::take(&mut self.joins) {
            if let Some(slot) = self.free_slot() {
                self.add_remote(peer, slot, tick + 1);
                self.remotes.get_mut(&peer).expect("added").last_heard = Some(self.now);
                f.roster.push(RosterEvent::Join { slot, peer });
                self.awaiting_snapshot.push(peer);
            }
        }
        stamp(&mut self.local_batches, 0, &mut f.inputs);
        for r in self.remotes.values_mut() {
            stamp(&mut r.ready, r.slot, &mut f.inputs);
        }
        let mut w = Writer::new();
        encode_frame(&mut w, &f);
        self.frames.insert(tick, w.buf);
        self.next_tick += 1;
        let oldest = self
            .remotes
            .values()
            .map(|r| r.needed)
            .min()
            .unwrap_or(self.next_tick);
        let behind: Vec<PeerId> = self
            .remotes
            .iter()
            .filter(|(_, r)| self.next_tick - r.needed.min(self.next_tick) > MAX_HISTORY)
            .map(|(p, _)| *p)
            .collect();
        for p in behind {
            self.drop_peer(p);
        }
        self.frames = self.frames.split_off(&oldest.min(self.next_tick));
        f
    }

    pub fn needs_snapshot(&self) -> bool {
        !self.awaiting_snapshot.is_empty()
    }

    /// Sends `snapshot` (the sim's state at [`Host::next_tick`]) to every peer that joined
    /// in the last frame.
    pub fn provide_snapshot(&mut self, snapshot: &[u8]) {
        for peer in std::mem::take(&mut self.awaiting_snapshot) {
            if let Some(slot) = self.remotes.get(&peer).map(|r| r.slot) {
                let w = self.welcome(slot, self.next_tick, Some(snapshot));
                self.outbox.push((peer, w));
            }
        }
    }

    /// Sends a client the current state again (after a desync). `snapshot` is the sim's
    /// state at [`Host::next_tick`].
    pub fn resync(&mut self, slot: u8, snapshot: &[u8]) {
        let Some(&peer) = self.slots.get(&slot) else {
            return;
        };
        if let Some(r) = self.remotes.get_mut(&peer) {
            r.needed = self.next_tick;
            let w = self.welcome(slot, self.next_tick, Some(snapshot));
            self.outbox.push((peer, w));
        }
    }

    /// Sends every client the frames it has not confirmed, plus queued control messages.
    pub fn flush(&mut self, t: &mut dyn Transport) {
        for (peer, msg) in std::mem::take(&mut self.outbox) {
            let _ = t.send(peer, Delivery::Reliable, CH_CONTROL, &msg);
        }
        for (peer, r) in &self.remotes {
            let mut w = Writer::with(FRAMES);
            w.u32(r.next_seq).u64(r.needed);
            let count_at = w.len();
            w.u16(0);
            let mut count = 0u16;
            for (_, f) in self.frames.range(r.needed..) {
                if w.len() + f.len() > MAX_UNRELIABLE || count == u16::MAX {
                    break;
                }
                w.raw(f);
                count += 1;
            }
            w.buf[count_at..count_at + 2].copy_from_slice(&count.to_le_bytes());
            let _ = t.send(*peer, Delivery::Unreliable, CH_FAST, &w.buf);
        }
    }

    /// Takes the next event, if any.
    pub fn event(&mut self) -> Option<SessionEvent> {
        self.events.pop_front()
    }
}

/// The client side.
pub struct Client {
    cfg: LockstepConfig,
    host: PeerId,
    slot: Option<u8>,
    start: Option<u64>,
    next_tick: u64,
    frames: BTreeMap<u64, Frame>,
    last: BTreeMap<u16, f32>,
    next_seq: u32,
    unacked: BTreeMap<u32, Vec<(u16, f32)>>,
    events: VecDeque<SessionEvent>,
    outbox: Vec<Vec<u8>>,
    last_heard: Option<Micros>,
    host_lost: bool,
}

impl Client {
    /// A client of `host`. It says hello at once; a host that does not know it yet treats
    /// that as a request to join.
    pub fn new(cfg: LockstepConfig, host: PeerId) -> Self {
        Self {
            cfg,
            host,
            slot: None,
            start: None,
            next_tick: 0,
            frames: BTreeMap::new(),
            last: BTreeMap::new(),
            next_seq: 0,
            unacked: BTreeMap::new(),
            events: VecDeque::new(),
            outbox: vec![vec![HELLO]],
            last_heard: None,
            host_lost: false,
        }
    }

    pub fn slot(&self) -> Option<u8> {
        self.slot
    }

    /// The tick the next frame will be for.
    pub fn next_tick(&self) -> u64 {
        self.next_tick
    }

    /// Frames ready to run, in a row.
    pub fn buffered(&self) -> usize {
        if self.start.is_none() {
            return 0;
        }
        let mut n = 0;
        while self.frames.contains_key(&(self.next_tick + n as u64)) {
            n += 1;
        }
        n
    }

    /// How many frames to run this fixed step: none when empty, one normally, more to catch
    /// up when the buffer grew (after a stall or a join).
    pub fn frames_due(&self) -> usize {
        match self.buffered() {
            0 => 0,
            n if n > 4 => n - 2,
            _ => 1,
        }
    }

    /// This player's current action values. Ignored until the host has welcomed us.
    pub fn set_local(&mut self, values: &BTreeMap<String, f32>) {
        if self.slot.is_none() {
            return;
        }
        let d = diff(&self.cfg, &mut self.last, values);
        if !d.is_empty() {
            self.unacked.insert(self.next_seq, d);
            self.next_seq = self.next_seq.wrapping_add(1);
        }
    }

    pub fn poll(&mut self, t: &mut dyn Transport, now: Micros) {
        t.poll(now);
        while let Some(p) = t.receive() {
            if p.from != self.host {
                continue;
            }
            self.last_heard = Some(now);
            self.handle(p.channel, &p.data);
        }
        let heard = *self.last_heard.get_or_insert(now);
        if !self.host_lost && now.saturating_sub(heard) > self.cfg.timeout_us {
            self.host_lost = true;
            self.events.push_back(SessionEvent::HostLost);
        }
    }

    fn handle(&mut self, channel: u8, data: &[u8]) {
        let mut r = Reader::new(data);
        match (channel, r.u8()) {
            (CH_FAST, Some(FRAMES)) => {
                let (Some(acked), Some(first), Some(count)) = (r.u32(), r.u64(), r.u16()) else {
                    return;
                };
                self.unacked.retain(|&s, _| s.wrapping_sub(acked) < u32::MAX / 2);
                for i in 0..count as u64 {
                    let Some(f) = decode_frame(&mut r, first + i) else {
                        return;
                    };
                    if f.tick >= self.next_tick {
                        self.frames.entry(f.tick).or_insert(f);
                    }
                }
            }
            (CH_CONTROL, Some(WELCOME)) => {
                let (Some(slot), Some(start), Some(has)) = (r.u8(), r.u64(), r.u8()) else {
                    return;
                };
                let snapshot = if has == 1 {
                    match r.bytes() {
                        Some(b) => Some(b.to_vec()),
                        None => return,
                    }
                } else {
                    None
                };
                self.slot = Some(slot);
                self.start = Some(start);
                self.next_tick = start;
                self.frames = self.frames.split_off(&start);
                self.events.push_back(SessionEvent::Welcomed {
                    slot,
                    start_tick: start,
                    snapshot,
                });
            }
            _ => {}
        }
    }

    /// The next frame to run, in order.
    pub fn next_frame(&mut self) -> Option<Frame> {
        self.start?;
        let f = self.frames.remove(&self.next_tick)?;
        self.next_tick += 1;
        Some(f)
    }

    pub fn should_hash(&self, tick: u64) -> bool {
        self.cfg.hash_interval > 0 && tick.is_multiple_of(self.cfg.hash_interval)
    }

    /// Sends this client's state hash at `tick` to the host for comparison.
    pub fn record_hash(&mut self, tick: u64, hash: &[u8]) {
        let mut w = Writer::with(HASH);
        w.u64(tick).bytes(hash);
        self.outbox.push(w.buf);
    }

    /// Says goodbye (the host frees the slot at once instead of after the time-out).
    pub fn leave(&mut self) {
        self.outbox.push(vec![BYE]);
    }

    /// Sends unacknowledged input batches and what we have received.
    pub fn flush(&mut self, t: &mut dyn Transport) {
        for msg in std::mem::take(&mut self.outbox) {
            let _ = t.send(self.host, Delivery::Reliable, CH_CONTROL, &msg);
        }
        let needed = self.next_tick + self.buffered() as u64;
        let mut w = Writer::with(INPUTS);
        w.u64(needed);
        let first = self.unacked.keys().next().copied().unwrap_or(self.next_seq);
        w.u32(first);
        let count_at = w.len();
        w.u16(0);
        let mut count = 0u16;
        for b in self.unacked.values() {
            if w.len() + 2 + b.len() * 6 > MAX_UNRELIABLE {
                break;
            }
            w.u16(b.len() as u16);
            for &(a, v) in b {
                w.u16(a).f32(v);
            }
            count += 1;
        }
        w.buf[count_at..count_at + 2].copy_from_slice(&count.to_le_bytes());
        let _ = t.send(self.host, Delivery::Unreliable, CH_FAST, &w.buf);
    }

    pub fn event(&mut self) -> Option<SessionEvent> {
        self.events.pop_front()
    }
}
