# 0285: Single-pass stereo with multiview

Date: 2026-11-14. Status: accepted. Focuses 3 (performance) and 4 (graphics), decision 0060.

## Context

Two-eye renders recorded every draw twice, one main pass per eye. Other engines draw both
eyes in one pass (audit 2026-11-14), which halves the CPU's draw recording for headsets.

## Decision

1. **`Renderer::render_stereo(frame, [View; 2], both, eyes, encoder)`** draws both eyes into a
   two-layer target with one multiview render pass (mask `0b11`) when the device and frame
   allow it, and otherwise one pass per eye into the single-layer views of the same layers, so
   callers always get both layers. `Renderer::single_pass_stereo()` says whether the device can.
2. **Shaders are not written twice.** The multiview form is made from each normal WGSL source
   (`renderer/multiview.rs`): the view uniform becomes `array<ViewU, 2>` and every vertex and
   fragment entry point gains `@builtin(view_index)` and picks its eye first, so the bodies stay
   the same. Stereo pipeline variants (lit, skinned, sky, particles) are built on first use.
3. **What the single pass covers**: opaque, cut-out and see-through models, skinned models, sky,
   environment, shadows (drawn once, shared by both eyes) and particles. Frames with capture,
   bloom, render scale, ray tracing, sprites, screen or world UI, world quads or game shaders
   fall back to a pass per eye. Shadow maps were already drawn once per frame.
4. **Device needs**: `MULTIVIEW` with at least two views; with MSAA also `MULTISAMPLE_ARRAY`
   (Vulkan and Metal; not DX12, so DX12 with MSAA falls back). Both are requested where the
   adapter has them. The view bind group layout has no minimum size, so the same layout binds
   one view or both.
5. **Where it is used**: `Offscreen::set_single_pass_stereo` and the two-eye benchmarks
   (`marionette bench --headset`, perf suite headset modes), on by default;
   `MARIONETTE_SINGLE_PASS_STEREO=0` times a pass per eye. The OpenXR runtime still renders
   one swapchain per eye; drawing into a two-layer swapchain is the next step.

## Results (cloud container, lavapipe)

- Material ball scene, both eyes: draw calls 13 to 8, one pass; right eye identical to a pass
  per eye (largest difference 0).
- Skinned animation scene: draw calls 8 to 4, identical picture.
- Bloom scene falls back to a pass per eye with an identical picture.
- Tests: `crates/marionette-render/tests/stereo.rs`, unit test of the shader conversion.
