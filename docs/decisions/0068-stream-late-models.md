# 0068: Models first needed mid-game decode in the background

Date: 2026-11-13. Status: accepted. Focus 3 (performance) and 4 (graphics), decision 0060.

## Context

The first stability run on a hardware GPU (RTX 2080, PROOFS.md) passed every frame budget at
p99, but two render modes had single frames far over the 11.1 ms headset budget after warm-up:
`proof-crystal-ward-render` 42 ms and `proof-signal-yard-render` 32.5 ms. Both games spawn an
entity from a script whose model no earlier frame drew (`crawler.glb`, `drone.glb`). The
renderer loaded models on first draw, inside the frame: reading and decoding one generated
model with three textures takes about 28 ms of CPU (release build). In a headset that is a
visible hitch every time a new kind of thing appears.

## Decision

`Renderer::set_streaming(true)` (and `Offscreen::set_streaming`): after the first prepared
frame, a model file that is not cached yet is sent to one background thread
(`marionette-model-loader`) that reads and decodes it. The frame draws without it; the first
`prepare` after the decode finishes uploads it to the GPU and draws it from then on. Built-in
`primitive:` shapes still load in place (they are cheap). `models_loading()` counts decodes in
flight and `finish_loading()` waits for them.

The first prepared frame always loads everything in place, and streaming is off by default, so
screenshots, reference images and benchmarks stay exact. The window loop and the render
stability modes turn it on, because they are the game loop.

## Alternatives

- A scene-level preload list: exact, but every game and every AI writing one would have to
  remember to list what scripts spawn, and a missed entry hitches again.
- Preloading every model path found in the scene and its scripts: guesses from strings.
- Decoding on the job system: the decode is one long task, not data-parallel work, and must not
  steal job workers from the simulation.

## Consequences

- Measured on the same GPU after the change: worst frames 5 to 7 ms in both modes (PROOFS.md).
- A model still decoding when the stability warm-up sample is taken is counted as memory
  growth once (seen once: 12.6 MiB against a 64 MiB limit).

- A newly spawned model appears one or more frames after its entity (about 30 ms of decode on
  the owner's desktop CPU). Rendering is not part of the state hash, so determinism is
  unchanged.
- The GPU upload (textures and mip levels) still runs in the frame; it is small on a hardware
  GPU. Moving it off the frame is left for when it shows up in a measurement.
- The loader thread ends when its renderer is dropped; requests are one per distinct asset, so
  its queues are bounded by the number of models a scene uses (MEMORY-AND-CRASHES.md).
- Test: `streaming_decodes_late_models_off_the_frame` in `crates/marionette-render/tests/render.rs`.
