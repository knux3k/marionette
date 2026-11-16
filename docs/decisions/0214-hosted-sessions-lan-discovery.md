# 0214: Hosted sessions (host owns the world, guests connect), LAN discovery, a received-message cap and `Transport::forget`

Date: 2026-11-16. Status: accepted.

## Context

Lockstep (0210) fits games whose whole state is deterministic and cheap to step on every
peer. Many co-op games instead let one host run the world and stream what changed to the
others (physics-driven items, large factories, worlds that load per player). Those games had
to write their own connection layer on top of `Transport`: handshake, keep-alives, time-outs,
who is connected. Two more gaps showed up: the reliable `Endpoint` rebuilt a peer's message
with no size limit (a broken or hostile peer could make it buffer without bound), and nothing
dropped a peer's resend queue once it left, so a long-running host kept resending to players
who were gone. Players also expect to see games on their own network without typing addresses.

## Decision

- `marionette_net::hosted`: `HostedHost` and `HostedGuest` over any `Transport`, borrowed per
  call. Control messages on channel 255: hello (protocol text and name), welcome, refuse
  ("another game version", "the game is full"), ping, bye. Events: `Joined`, `Left` with a
  `LeaveReason` (`Left`, `TimedOut`, `Kicked`), `Refused`, `Message`. Messages from peers that
  never said hello are ignored. `HostedConfig`: protocol, name, max guests, keep-alive (1 s)
  and time-out (10 s). The game owns everything else (what to stream, who is "ready").
- `Endpoint::set_max_message` (default 64 MiB, `DEFAULT_MAX_MESSAGE`): a longer reliable
  message is dropped while it arrives; its fragments are still acknowledged so the sender moves
  on; counted in `oversize_dropped`.
- `Transport::forget(peer)` (default does nothing; `Endpoint` drops the peer's queues). Hosted
  sessions call it when a peer leaves or times out.
- `marionette_net::lan`: `LanBeacon` broadcasts `TAG|name|port` once a second; `LanBrowser`
  lists games with the same tag heard in the last 4 s. Without a broadcast route nothing is
  found and joining by address still works.
- `Lobbies::open_invite_dialog` (default: refused) and `LobbyEvent::InviteAccepted` for
  platforms whose overlay can invite friends and start a join from outside the game.

## Alternatives

- **Only lockstep**: forces every game into a fully deterministic, fully shared sim; a game
  whose items are moved by a physics engine on the host cannot be ported that way. Rejected.
- **Connections in the transport (connect/accept)**: Steam's networking messages are
  connectionless and so is UDP; keeping the transport connectionless and putting the session
  on top keeps every backend at three methods. Kept as is.
- **Per-channel size limits in the endpoint**: the game knows which messages may be large;
  one ceiling in the endpoint plus the game's own per-kind limits after receipt is simpler.

## Consequences

- A host-owned game uses `HostedHost`/`HostedGuest` plus its own message kinds; lockstep games
  are unchanged.
- Hosts no longer leak resend queues for players who left.
- LAN discovery needs UDP broadcast; only one browser per machine can hold the port.
