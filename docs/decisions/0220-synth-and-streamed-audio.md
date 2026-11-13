# 0220: A real-time synth, streamed music and offline audio checks

Date: 2026-11-13. Status: accepted.

## Context

Audio so far (0075) played recorded or built-in sounds through kira. Games that make all their
sound in code need voices computed in real time, long music read from disk instead of held in
memory, many simultaneous voices, notes that start on an exact sample, and a way to verify
audio when nobody can listen. Nothing had been heard through a real device either.

## Decision

- **A new crate, `marionette-synth`**, with no sound-card dependency. A game or an agent can
  take it without `marionette-audio`. `marionette-audio` only hosts it (`Player::start_synth`
  runs a `Synth` as one kira sound, so it shares the device and mixer with 0075's playback).
- **Two halves joined by a lock-free queue** (`rtrb`): `SynthHandle` for the game, `Synth` for
  the audio thread. `Synth::render` allocates nothing, takes no lock, does no I/O. All memory
  (voices, pending table, queue, generator slots) is reserved in `create`; overflow drops or
  steals and is counted in `Stats`. `tests/realtime.rs` counts allocations during render and
  requires zero.
- **Sample-accurate scheduling**: a command carries the synth-timeline frame it runs at. The
  audio thread splits its block at each command. Late commands run at once and count.
- **Voices are plain data**: `Patch` is `Copy` (two oscillators with band-limited saw, square
  and triangle, noise, biquad filter with envelope and key tracking, amp and filter ADSR,
  vibrato, pan), so a note-on carries its whole sound through the queue. Stealing takes a
  releasing, quiet, old voice first. Control values refresh every 32 samples counted per
  voice, so output does not depend on how the host splits blocks (tested for block sizes 1 to
  4096: identical bits).
- **Game sample code**: the `Generator` trait in 16 slots. Replaced or finished generators
  go back to the game thread to be dropped there, never on the audio thread.
- **Streaming**: a reader thread decodes WAV or OGG (symphonia) in chunks, resamples to the
  synth's rate, and fills a 65,536-frame ring. The audio thread pops from it; an empty ring
  plays silence and counts an underrun. Looping reopens the file. Memory is the ring, whatever
  the file's length. `blocking` mode (offline only) waits for data so renders are repeatable.
- **Offline render and numbers**: `Score` (JSON) renders through the same code to a buffer;
  `analyze` reports peak, rms, DC, clipping, largest sample step (clicks), dominant pitch,
  first sound frame; `wav_stereo_bytes` writes a file. Operations `synth_render` and
  `synth_patches`. Hashes of samples are not pinned across platforms (libm differs).
- **Real-device check**: `marionette audio-check` plays a stress run (default 64 notes per
  second, a looping streamed bed) and reports callbacks slower than their audio, stream
  underruns, dropped commands. Exit 1 on a glitch, 2 without a device.

## Alternatives

- Use kira's own streaming sound: no shared graph with the synth, and cannot be rendered
  offline for numeric checks.
- Add `cpal` directly and own the device: a second audio backend beside kira's for no gain.
- A mutex-protected parameter struct instead of a queue: blocks the audio thread.

## Focus check

Modularity: own crate, hosted by an adapter, game code plugs in as `Generator`. Tokens: one
op returns a few numbers instead of audio; one skill. Performance: about 0.1 percent of a core
per sustained voice in release on the cloud container (PROOFS: play.md). Graphics: none.
