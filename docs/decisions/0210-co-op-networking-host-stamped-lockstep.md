# 0210: Co-op networking is host-stamped lockstep over a pluggable transport (`marionette-net`)

Date: 2026-11-13. Status: accepted.

## Context

Co-op games need several machines to see the same world. The engine's simulation is
deterministic (fixed step, seeded RNG, ordered systems, SHA-256 state hash), and the games it
targets can hold very large states (a factory game: tens of thousands of machines and belt
items per site) that do not fit a state-streaming budget. Players join from lobbies, sometimes
mid-game, over home connections with loss and jitter. Everything must be testable in CI
without Steam or a second machine.

## Decision

- New crate `marionette-net`, layered so each layer is replaceable:
  - `Transport`: connectionless messages to a `PeerId`, `Reliable` (once, in order, any size)
    or `Unreliable` (at most 1150 bytes), on numbered channels. `poll(now)` takes the time from
    the caller; nothing reads the clock, so tests run on virtual time.
  - `Datagram` + `Endpoint`: a datagram source (three methods) gets reliability from
    `Endpoint`: per-peer sequence numbers, fragments of 1100 bytes, cumulative plus 32-bit
    selective acks, resend timers from a smoothed round trip (Karn's rule), a 256-fragment
    window, a 64 MiB queue cap. Sources: `LoopbackNet` (in process, seeded latency, jitter,
    loss, duplication) and `UdpDatagram` (std UDP, peer id = packed IPv4 address).
  - `lockstep`: `Host` and `Client`. Only inputs travel. The host builds one `Frame` per tick
    (its own action changes, the clients' changes that have arrived, roster changes) and sends
    every client all frames it has not acknowledged, unreliably, in each packet; clients send
    their unacknowledged input batches the same way. Welcomes, snapshots and hashes travel
    reliably.
  - `sim`: frames onto a `Sim` (decision 0211). `simulate`: a whole session in one process.
  - `Lobbies` trait plus `LocalLobbies` (in process).
- Hash checks: every `hash_interval` ticks (default 60) clients send their state hash for that
  tick; the host reports `SessionEvent::Desync { slot, tick }` and can `resync` the client with
  a snapshot.
- Late join: a peer that says hello gets a `Join` in the next frame and, after the host has run
  it, a snapshot of the host's sim at the following tick. Leaves: a goodbye or 10 s of silence
  (configurable) puts a `Leave` in the next frame; the leaving player's actions return to 0.
- Agent operation `net_simulate` (feature `netplay` of `marionette-agent`, on in the
  `marionette` tool, off for games that do not ask for it).

## Alternatives

- **Peer-to-peer lockstep (every peer waits for every peer's input)**: one-way latency instead
  of a round trip, but the whole session stalls on the slowest peer and joins, leaves and
  missing inputs need agreement between all peers. Rejected: co-op favours a host that never
  waits.
- **Rollback (predict remote input, rewind on correction)**: best feel for fighting games, but
  needs a full save and re-simulation of several ticks per correction. For large states
  (`save_state` builds scene text, tens of milliseconds at factory scale, decision 0073) that
  is far over budget. Can be added later as another session type on the same transports;
  local presentation (the player's own camera and hands) is predicted outside the sim instead.
- **State replication (the host streams entity state)**: independent of determinism, but the
  bandwidth grows with the world; a factory site would need megabytes per second. Rejected as
  the default; a game can still send its own unreliable presence messages (avatar poses) on
  another channel.
- **An existing networking crate (renet, laminar, GGRS)**: each brings its own clock, threads or
  session model, and none fits a pluggable Steam transport plus virtual-time tests without
  wrappers. The reliability layer is about 300 lines.

## Consequences

- Bandwidth is independent of world size: about 50 bytes per tick per client in the tests
  (3 KB/s), 375 bytes per tick for the host with five players in the co-op example.
- A client's own input reaches its screen one round trip later (host input: next tick). Games
  hide this for the local avatar by moving it outside the sim and reconciling.
- Determinism is now a network requirement: a nondeterministic system shows up as a desync
  with the tick it happened, which `net_simulate` finds in CI.
- Focus 1: three crates, all optional (`netplay` feature, Steam a separate crate). Focus 2:
  one operation proves a scene is network-safe. Focus 3: no cost when unused; host work per
  tick is one frame encode plus one packet per client.
