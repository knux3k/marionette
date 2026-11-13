# Proofs: Input, physics, audio, UI and animation (plan line 6)

Line 6 (input, physics, audio, UI):
- Example scenes: `examples/playground3d.scene.json` and `examples/playground2d.scene.json`,
  each with an action map (`*.actions.json`) and a scripted input file (`*.inputs.json`): a
  character walks, pushes and climbs dynamic boxes, jumps; jump and collision sounds.
- `cargo run -q -p marionette-cli -- run examples/playground3d.scene.json --headless --ticks 1000 --seed 1 --input examples/playground3d.inputs.json`
  prints `541b991eb45fe2a436c2ec8c0bea9b490c3a84356b51c8740771aa1083547c44`; the 2D scene with
  its input prints `1071e11381ec030e587cc2e241fedbafca679becc1ef13a886ea389dc184e8ac`.
- Tests: `crates/marionette-cli/tests/line6_proofs.rs`
  (`physics_scenes_give_the_same_hash_twice_and_match_the_recorded_hash`,
  `replaying_a_recording_gives_the_original_hash`,
  `snapshot_at_500_then_restore_continues_exactly`,
  `sound_log_lists_the_expected_sounds_at_the_expected_ticks`);
  `crates/marionette-physics/tests/build_errors.rs`; unit tests in
  `crates/marionette-input/src/lib.rs` and `crates/marionette-render/src/window.rs`.
- UI scene matches its reference image (line 3 proofs).
- Window: live keyboard and mouse into the input queue, `UiButton` clicks, sound playback,
  `--input`, `--record`, `--mute` (decision 0017). Real screens: [window.md](window.md).
- `enhanced-determinism` cost: within noise (decision 0012, measured with
  `cargo run --release -p marionette-physics --example step_cost -- 1000`).
- Modules: decision 0011; input 0013; physics 0012; audio 0014; UI 0015.

## Animation (decision 0028)

- `cargo run -q -p marionette-cli -- run examples/animation.scene.json --headless --ticks 600 --seed 1`
  prints `f409e570ea3b9d3c1e92a01adc3070dbbd49b39d147b0dd97caa39297dba6d13` (twice in a row).
- Reference image `examples/reference/animation.png` (320x180, 15 ticks, seed 1, MSAA 4).
- Tests: `crates/marionette-cli/tests/animation.rs` (recorded hash twice, snapshot mid-clip
  and restore, bones move and loop exactly, cross-fade and `anim_finished`, unknown clips
  refused, script `playClip` equals `anim_play`, clip listing, reference image, 64 skinned
  characters in 4 draw calls, morph targets change the mesh); `crates/marionette-anim/tests/puppet.rs`;
  unit tests in `crates/marionette-anim/src/lib.rs`, `crates/marionette-anim/src/sample.rs` and `crates/marionette-assets/src/rig.rs`.
- Test character `examples/assets/puppet.glb` (CC0, made by
  `cargo run -p marionette-anim --example make_puppet -- examples/assets/puppet.glb`,
  byte-identical every run).
- Not yet: root motion, animation events, IK, state machines; FBX blend shapes (additive
  layers and bone masks: decision 0074, next section)
  (the FBX converter of the FBX branch writes skins and clips; blend shapes not yet).

## Blend spaces, overlays, 3D sound, music layers (decisions 0074, 0075)

- Scene `examples/blend-and-sound.scene.json`, built by
  `cargo run -q -p marionette-cli -- op batch_run '{"file":"examples/blend-and-sound.ops.json"}'`
  (scripts `examples/scripts/strider.ts`, `orbit.ts`, `conductor.ts`).
- `cargo run -q -p marionette-cli -- run examples/blend-and-sound.scene.json --headless --ticks 600 --seed 1`
  prints `d344027414b14c0f555f9de4b443777a25a1b6f07886cda765a49bfd0938e236`.
- Tests: `crates/marionette-cli/tests/blend_and_sound.rs` (recorded hash twice, synced space
  phase runs at the blended cycle length and parameters glide, a space at an edge poses like
  that clip, a masked overlay changes only masked bones and their children, unknown clips in
  spaces and overlays refused, the orbiting hum pans right, muffles behind and pans left
  through `audio_mix`, music layers fade in by intensity and out through `audio_music`); unit
  tests in `crates/marionette-anim/src/blend.rs` (1D and 2D weights, parameter glide,
  quaternion helpers), `crates/marionette-audio/src/lib.rs` (distance curves, rear muffle and
  head turn, Doppler, stems decode) and `crates/marionette-audio/src/music.rs` (levels, stems
  loop without a click).
- Stability mode `blend-and-sound` (render).
- Drift hash and every earlier pinned hash unchanged (new components only).
- Not yet: heard through a real audio device or headset; HRTF, occlusion, reverb.

## Procedural and streamed audio (decision 0220)

- `cargo test -p marionette-synth`: pitch of sine, saw, square, triangle within 2 Hz; level and
  no DC or clipping; identical bits for block sizes 1, 7, 64, 480, 4096; a note starts on the
  exact frame (1000) even when blocks split elsewhere; late commands run now and count; 300
  notes on 16 voices steal without NaN or going past full scale; release, master glide and
  looping streams make no clicks; a WAV streamed from disk keeps its pitch and start frame, a
  44.1 kHz file resamples to 48 kHz, looping continues past the file's end; custom generators
  mix in and are handed back; bad scores fail with a reason.
- `tests/realtime.rs`: zero allocations in 400 renders with 32 voices, queued commands,
  stealing and a generator.
- `cargo test -p marionette-cli --test synth`: `synth_render` of `examples/synth-demo.score.json`
  (report, envelope, WAV size) and `synth_patches`.
- Cost, `cargo run --release -p marionette-synth --example bench` (cloud container, one core,
  480-frame blocks at 48 kHz): sustained voices as percent of one core: 64 voices 5.6 to 6.5,
  128 voices 11.7 to 13.0, 256 voices 20 to 30, 512 voices 37 to 58.
- `marionette audio-check` without a device exits 2 with a message (checked here).
- Not checked: OGG streaming (WAV only is tested; the
  decoder is shared); the PC figures. Planned PC run (Unity lock free, PC idle 5 minutes):
  `cargo run --release -p marionette-cli -- audio-check --seconds 30 --voices 128`, then 256,
  expecting `pass: true`; record `max_block_us` and the device rate here.
- Real-device runs (2026-11-13, Ryzen 7 2700X, USB speakers, 48 kHz, release build,
  `audio-check --seconds 30`, no Unity, no other bench running). The device calls back with
  96 frames (2 ms), so the deadline is tight. Run 1 (loud test mix, before the counters):
  64 voices clean; 128 voices 3 slow callbacks; 256 voices 12. Run 2 (current check, quiet mix
  that no longer clips; PR 63): 64 voices (79 at peak) 0 slow, worst callback 33% of its
  deadline; 128 voices (155 peak) 0 slow, worst 96%; 256 voices (311 peak) 10 slow, worst 186%,
  4.9 ms; 512 voices (1024 peak, pool full) 6232 slow, 5666 stolen, 5990 dropped commands.
  In every run: 0 underruns, 0 stream underruns, 0 late commands. Reading: 64 voices has wide
  margin (a game uses far fewer); 128 is at the edge of a 2 ms buffer; past that the callback
  misses its deadline, as the release benchmark predicts (256 voices is about 20 to 30 percent
  of a core per 10 ms, which is 2 to 3 ms per 10 ms but spikes past a 2 ms deadline). Not done:
  asking the device for a larger buffer (kira's `StreamConfig` allows it), which would give
  5x margin at 10 ms latency; nobody has listened to the output.
