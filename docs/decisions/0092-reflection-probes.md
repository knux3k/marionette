# 0092: Reflection probes

Date: 2026-11-13. Status: accepted. Focus 4 (graphics), focus 3 (performance).

## Context

Shiny surfaces reflected only the environment (decision 0066): a mirror floor in a room showed
the sky, not the room. Ray-traced reflections (decision 0093) need ray query hardware; every
other GPU, and headsets on a frame budget, need something that costs almost nothing per frame.

## Decision

Component `ReflectionProbe { extent, resolution }` on an entity with a transform.

- Capture: when a probe appears, moves or changes, `Renderer::prepare` draws the frame six
  times from its position (90-degree faces, linear light into the floating-point target, no
  bloom, sprites, UI or probe reflections), reads the faces back, resamples them into an
  equirectangular image and prefilters it exactly like the environment (`probes.rs`, reusing
  `environment.rs`). A still probe costs nothing per frame.
- Up to four probes (lowest entity ids), each one layer of a 256x128, six-mip half-float
  array (350 KB per probe; a 1x1 stand-in while a scene has none).
- Shading: for the first captured probe whose box (plus a small margin) holds the surface, the
  mirror direction is corrected for the box (it is followed to the box's wall and the probe is
  looked up toward that point), blurred by roughness, and replaces the environment or ambient
  reflection, fading out just outside the box. Diffuse light keeps the environment's.
- Probes and ray-traced reflections combine: the traced reflection goes on top of the probe's.

## Alternatives

- Screen-space reflections: no capture, but they show only what is on screen, differ between
  the two eyes of a headset, and cost a full-screen pass per eye every frame.
- Cube textures: simpler lookups, but a second prefilter path; the equirectangular path is
  already written and tested for the environment.
- Capturing every N frames: catches moving objects, but each capture stalls for a readback.
  Moving objects are what ray-traced reflections are for.

## Consequences

- Box correction is exact for surfaces on or near the box's walls; objects in the middle of a
  large box reflect at the wrong place (visible in the example's fallback picture). Fit the box
  to the room.
- A capture blocks the frame that makes it (six passes plus a readback); measured in
  PROOFS.md.
