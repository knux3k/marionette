# Proofs: headsets through OpenXR (decision 0190)

- `cargo test -p marionette-cli --test xr`: `grab_and_throw_a_block`, `throws_replay_bit_for_bit`,
  `snap_turn_turns_once_around_the_head`, `teleport_lands_where_the_hand_points`,
  `stick_movement_moves_the_rig_and_asks_for_a_vignette` (scripted poses, no runtime);
  `example_runs_on_an_openxr_runtime` with `MARIONETTE_XR_RUNTIME=1` (CI job `xr`: Monado
  21.0 simulated headset, Xvfb, lavapipe). Unit tests in `crates/marionette-xr`.
- Hand tracking without a hand-tracking runtime (Monado's simulated headset has none):
  `a_pinching_hand_grabs_and_throws_without_a_controller` (pinch ramps in and out, a light
  pinch does not grab) and `marionette-openxr` unit tests that feed made-up joints to
  `hand_from_joints` (pinch from thumb-to-index distance, palm as the hand pose, a partly lost
  hand reads as none). Real finger joints from a headset are still untested.
- Stability mode `xr-play` (scripted throw plus random controller input, two eyes).
- Simulated headset run, container, lavapipe, debug build:
  `marionette xr examples/xr.scene.json --frames 120 --mute`: session idle, ready,
  synchronized, visible, focused, then stopping, idle, exiting; 119 of 121 frames rendered at
  896x1007 per eye, 4x MSAA. Both eyes seen in Monado's compositor window.
- Release build, same container: `--frames 300`: 299 rendered; CPU per rendered frame (acquire
  and wait for the eye images, extract, record, submit) p50 0.56 ms, p99 3.8 ms. The frame
  interval (50 ms) is Monado compositing in software, not the engine; GPU time on a desktop GPU
  and a worn headset is UNVERIFIED (STATE.md).
- PC with SteamVR (2026-11-14), headset not linked: release build ran; the probe first failed
  for want of a loader beside the exe (now also searched in SteamVR's default folder and
  `MARIONETTE_OPENXR_LOADER`), then SteamVR refused the app (its log: wireless headset not
  connected), now reported as `xr.runtime_refused`. No frames rendered; 90 Hz still unmeasured.
- PC with SteamVR's null headset (2026-11-14): probe and a 900-frame run ok, loader found in
  SteamVR's folder, all controller profiles accepted. The null headset is a desktop window, so
  the compositor ran at the monitor's 60 Hz (interval p50 16.7 ms, p99 17.1 ms) and never
  reported it worn (session stayed synchronized; 1 frame drawn, 3.3 ms CPU at 2016x2240 per
  eye). 90 Hz needs a real or spoofed worn headset. SteamVR could not save bindings while the
  app name held a `/`; names now pass through `safe_app_name`.
