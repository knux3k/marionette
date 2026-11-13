---
name: marionette-multiplayer
description: Make a Marionette game co-op and ship it on Steam - lockstep sessions with hash checks, lobbies, late join, per-player input, achievements, stats and cloud saves, tested without Steam. Use when a task mentions multiplayer, co-op, netcode, lobby, host or join, desync, Steam, achievements, stats or cloud saves.
---

# Co-op, Steam, achievements, cloud saves

Every peer runs the same deterministic sim; only input travels. The host stamps each tick's
input into a frame, clients run frames in order, state hashes are compared every
`hash_interval` ticks (decisions 0210 to 0213).

## Make the scene network-safe

1. Scripts read player `n`'s actions as `engine.action("p<n>.move_x")` and who is playing from
   the `NetPlayers` resource (`slots`). Never read the plain action names in co-op logic.
2. Prove it, no Steam or second machine needed:
   `marionette op net_simulate '{"scene":"scenes/main.scene.json","players":4,"late_join_at":200,"link":{"latency_ms":80,"loss":0.1}}'`.
   `all_equal` must be true. A non-empty `desyncs` gives the first tick where a peer differed:
   look for clock reads, unseeded randomness, hash map iteration or float maths that differ
   between machines in what ran before that tick (the determinism rules in the engine docs).
3. Add a stability mode for the scene like any other.

## Wire it in the game's Rust code

- Engine: add `marionette_net::NetModule` (`Engine::builder().module(...)`).
- Transport: `Endpoint::new(UdpDatagram::bind(addr))` for LAN, `marionette_net::LoopbackNet` for tests and
  local split screen, or Steam (below). All implement `marionette_net::Transport`.
- Lobbies: the `marionette_net::Lobbies` trait (`marionette_net::LocalLobbies` in process, `marionette_steam::SteamLobbies`). The lobby owner is the
  host; members become clients.
- Each fixed step. Host: `poll`, `set_local`, `next_frame`, `apply_frame` then `Sim::step`,
  `record_hash` when `should_hash`, `provide_snapshot` when `needs_snapshot`, `flush`.
  Client: `poll`, `set_local`, run `frames_due` frames the same way, `flush`; on
  `SessionEvent::Welcomed` with a snapshot, `restore_snapshot` first.
- The local player's own avatar and camera should move outside the sim at once and follow the
  sim's result (client input takes a round trip).

## Achievements, stats, cloud saves

1. Write `*.achievements.json` (format `marionette.achievements`); check it with
   `marionette op achievements_check '{"file":"data/game.achievements.json"}'`. Ids must equal
   the API names set up on the store's site.
2. Use a `marionette_platform::Tracker` over `marionette_platform::OfflinePlatform` (tests, no store) or `Steam::platform()`. Call
   `store` at checkpoints, not every tick. Achievements never go in the scene.
3. Cloud saves: Steam Auto-Cloud on the store's site needs no code; otherwise
   `sync_dir(save_folder, &[".save"], cloud)` at start and after each save. A
   `.cloud-conflict` file means both machines changed a save: ask the player.

## Steam

- Crate `marionette-steam`, added by the game only. It loads `steam_api64.dll` /
  `libsteam_api.so` from next to the executable at run time. That file comes from the game
  developer's own Steamworks SDK download: never commit it to the engine repository.
- Development: `steam_appid.txt` next to the executable (480 is Valve's public test app).
  Check a machine with the crate's `steam_check` example.
- Only a person can accept Valve's agreements, pay for an app id or log in to Steam.
