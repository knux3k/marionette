---
name: marionette-tune-render-cost
description: Lower frame cost or raise image quality with level of detail, render scale, foveation, anti-aliasing and culling. Use when GPU time or triangles dominate a frame, a headset misses 90 Hz, far objects cost too much, or edges look jagged.
---

# Tune render cost

Measure before and after every change with a release build on the real target GPU
(skill `marionette-find-slowdown`): `marionette bench <scene> --headset --frames 600 --json`.
Keep a change only if the number improved and the picture is still acceptable to the owner.
Every control below except culling is off by default and set per scene with `resource_set`.

## What each control does

| Control | Pixels | Use when |
| --- | --- | --- |
| Frustum culling | identical | always on; compare with `bench --no-cull` |
| `LevelOfDetail` | far parts slightly simpler | many detailed models (64 triangles or more each) far from the camera |
| `RenderScale` below 1 | softer | GPU wait dominates and shading per pixel is heavy |
| `RenderScale` above 1 | cleaner edges | spare GPU time and shimmering edges |
| `Foveation` | edges of the view, headsets only | heavy shading in a headset |
| `--msaa 4` | anti-aliased edges | default; keep it |

## Steps

1. Read `render` in the bench output: `instances`, `culled`, `triangles`, and the phase
   medians. If `record` or `extract` dominate, the cost is on the CPU: these controls will
   not help (reduce entities or use `JobDraw`).
2. Many triangles far away: add level of detail.
   `marionette op resource_set '{"scene": "scenes/main.scene.json", "resource": "LevelOfDetail", "value": {"medium_below": 0.2, "low_below": 0.06, "hide_below": 0.0}}'`
   Sizes are a part's diameter as a share of screen height. A small `hide_below` (0.004)
   drops tiny far parts.
3. GPU-bound with heavy shading: try `RenderScale` with `{"scale": 0.75, "sharpness": 0.3}`.
   It adds one full-screen pass, so it only pays when the world pass is expensive; in light
   scenes it can be slower. Measure.
4. Headset mode with heavy shading: `Foveation` with `{"inner_radius": 0.7}`. Never on a flat
   screen (edges look stair-stepped up close).
5. Re-run the bench with the same seed and frames; record both numbers.

## Traps

- Turning `LevelOfDetail` on in a running scene reloads models once (a hitch).
- Skinned and morphed models are not culled and keep full detail.
- UI is never scaled or foveated.
- A software GPU (lavapipe, WARP) says nothing about these controls' real cost.
