# 0212: Steam is an optional backend that loads the game's own steam_api library at run time

Date: 2026-11-13. Status: accepted.

## Context

Games built on the engine ship on Steam and want Steam's relayed peer-to-peer networking,
lobbies, achievements, stats and cloud files. The Steamworks SDK is distributed by Valve under
the Steamworks SDK Access Agreement, which a developer accepts when downloading it; the SDK
itself (headers, docs, tools) is not licensed for redistribution, while the redistributable
runtime (`steam_api64.dll`, `libsteam_api.so`, `libsteam_api.dylib`) may ship with the
developer's game. The engine repository is public under MIT OR Apache-2.0 and must build and
test in CI with no Steam.

## Decision

- Crate `marionette-steam`, not used by any other engine crate and not in the `marionette`
  tool: a game adds it to its own binary when it wants Steam.
- No SDK file is in the repository or downloaded by the build. The crate declares the few
  flat-API functions it calls (about 50, plain C ABI: `SteamAPI_*`) and loads them with
  `libloading` from the library the game ships next to its executable (or a given path).
  Interface accessors are tried newest first (for example `SteamAPI_SteamUser_v023`, then
  `v022`); `SteamAPI_InitFlat` is used when present, else `SteamAPI_Init`.
- Callbacks use manual dispatch (`SteamAPI_ManualDispatch_*`), polled from the transport and
  lobby `poll` calls: no callback threads, no C++ objects.
- It implements the engine's interfaces: `Transport` over `ISteamNetworkingMessages` (Valve's
  relays, NAT traversal; session requests accepted; reliable messages over 512 KiB split),
  `Lobbies` over `ISteamMatchmaking`, `Achievements` over `ISteamUserStats`, `CloudFiles` over
  `ISteamRemoteStorage`.
- A missing library or a Steam client that is not running is an error with a stable code
  (`steam.library`, `steam.init`); the game falls back to the offline platform and UDP.
- `examples/steam_check.rs` checks a machine: library found, API started, app id, cloud on.

## Alternatives

- **The `steamworks` crate (steamworks-sys)**: its crate package contains the SDK headers and
  redistributable binaries and links them at build time; depending on it would pull SDK files
  into every build of the engine and tie the engine to one SDK version. Rejected for the
  engine; a game may still use it in its own code.
- **Build-time bindings against an SDK path (`STEAM_SDK_LOCATION`)**: no SDK in the repository,
  but every build of a game would need the SDK, and CI could not compile the backend. Rejected.
- **Steam's C++ interface**: needs a C++ compiler and the headers. Rejected.

## Consequences

- CI compiles and unit-tests the backend (missing library, wrong library, struct layouts), but
  it has never talked to a real Steam client (UNVERIFIED in STATE.md). First real check: the
  `steam_check` example and a two-machine lobby on the owner's PC with Valve's public test app
  id 480 (Spacewar), which needs a Steam login, so the owner runs or allows it.
- A shipping game needs its own Steam app id and Steamworks partner account (the developer
  accepts Valve's agreements and pays the app fee); the engine never does either.
- Struct offsets that differ by platform packing (callback structs are packed to 4 bytes except
  on Windows) are written out in `ffi.rs` with the reason.
