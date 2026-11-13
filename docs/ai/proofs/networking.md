# Proofs: co-op networking, Steam, achievements, cloud saves

Decisions 0210 to 0213. All run in CI with no Steam and no second machine.

- Reliability: `crates/marionette-net/src/reliable.rs` tests (`reliable_survives_loss_reorder_and_duplicates`:
  40 messages plus 50 KB over 25% loss, 30 ms jitter, 10% duplicates arrive once and in order;
  `unreliable_size_limit`). Loopback determinism and loss: `loopback.rs` test.
- Real sockets: `crates/marionette-net/tests/udp.rs` (`reliable_over_udp` 200 KB on 127.0.0.1;
  `lockstep_over_udp`: the client runs exactly the host's 120 frames).
- Sessions on the loopback network, `crates/marionette-net/tests/session.rs`:
  `four_players_bad_network_stay_in_step` (80 ms, 40 ms jitter, 10% loss, 5% duplicates, 600
  ticks: every peer ends in the host's state, 57 hash checks, lag under 40 ticks, the whole run
  repeats byte for byte), `late_join_and_drop`, `a_desync_is_reported` (first check, right slot).
- Through the operation table, `crates/marionette-cli/tests/netplay_e2e.rs` on
  `examples/coop.scene.json`: four players plus a late joiner at 80 ms and 10% loss end equal;
  frames move only the named players (`frames_drive_the_named_players_only`);
  `achievements_check` on `examples/coop.achievements.json`.
- Traffic (`net_simulate`, default link 40 ms, 2% loss, drift scene, 4 players): about 25 bytes
  per tick per client up, 53 down; host 165 bytes per tick in total.
- Platform: `crates/marionette-platform/tests/platform.rs` (definitions checks, stat-driven
  unlocks with clamping and persistence, two-machine save sync with conflict kept, cloud off).
- Steam backend: `crates/marionette-steam` tests (missing library and a non-Steam library give
  `steam.library`; callback struct size per platform). Not run against a Steam client.
- Stability mode `coop-netplay` (`stability/suite.json`).
