# 0284: A generated realistic render benchmark, buildable in Unity from the same files

Date: 2026-11-14. Status: accepted. Focuses 3 (performance) and 4 (graphics), decision 0060.

## Context

The engine's render benchmarks were small examples and a factory of cubes (audit 2026-11-14):
nothing near a shipped game's load of triangles, materials and textures, and nothing that
could be built the same way in Unity to compare the two engines.

## Decision

1. **Operation `perf_realistic_scene`** writes, from fixed seeds and with no downloads:
   - 33 texture sets (32 models and the ground), each a 2048x2048 color, normal (+Y up) and
     metal-roughness image (99 textures; also a metal-smoothness image per set for engines that
     use that packing). Patterns: tiles, planks, rock, riveted metal panels, all tileable.
   - 32 models of 8 parts with one material each (257 materials with the ground): 24 static
     clusters of about 32,000 triangles and 8 moving ones of about 1,800.
   - 384 static instances and 3,200 movers orbiting 32 anchors with the built-in `Follow` and
     `Velocity` systems (about 17.9 million triangles in all), a sun with 4 shadow cascades,
     hemisphere ambient, ACES tone mapping and `TextureCompression` (decision 0283).
   - `manifest.json` and plain mesh files (`MBM1`) describing the same scene for other engines.
   - `scale` multiplies the instance counts; `texture_size` makes small sets for tests.
2. **Unity side** in `tools/unity-bench/`: an editor builder (batch mode) that imports the same
   textures as BC7, builds meshes and URP Lit materials, places the same instances and camera,
   moves the movers with the same formula in `FixedUpdate`, and builds a player; a timer that
   writes frame, main-thread, render-thread and GPU times as JSON.
3. Timing on the Marionette side is `marionette bench` (mono 1920x1080 and `--headset`); the
   suite (decision 0100) is unchanged, so its reports stay comparable.

## Limits

- Lighting and tone mapping are close, not identical (Unity's URP has its own ambient and tone
  curve); the comparison is cost at the same work.
- Marionette renders offscreen without presenting; the Unity player presents to a window with
  vertical sync off.
- Static batching in Unity is a separate run (`-benchStatic`); Marionette has no equivalent.
- The Unity scripts were written without a Unity install in the cloud; the first PC run is
  their first compile.
