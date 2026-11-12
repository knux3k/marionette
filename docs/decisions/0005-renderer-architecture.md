# 0005: Renderer architecture (wgpu forward PBR + batched sprites)

Date: 2026-11-11. Status: accepted.

## Decision

- wgpu 30 (Vulkan, DX12, Metal, GL), glam 0.34 for render-side math, gltf 1.4 for glTF 2.0 /
  GLB, png 0.18 for PNG, winit 0.30 for the window.
- Offscreen first: `Offscreen` renders into a texture and reads it back; the window
  (`window.rs`) is a thin layer over the same `Renderer`.
- One forward pass per view: PBR models (metallic-roughness, GGX, Lambert, up to 4 directional
  lights, an ambient term, normal maps through a screen-space tangent frame so meshes need no
  tangents), then sprites (one vertex buffer per frame, batches of consecutive sprites sharing an
  image, sorted by layer then entity id, alpha blended, depth-tested but not depth-written).
- Bind groups: 0 = view (view-projection, eye), 1 = lights, 2 = material textures, 3 = per-draw
  data (model and normal matrices plus material factors) in one dynamic-offset uniform buffer.
- A `View` is a value passed to `Renderer::render`, never renderer state, so a later OpenXR
  module renders two eyes with two views (or multiview) without a rewrite.
- `extract()` takes `&World` and copies what to draw, so rendering cannot change simulation
  state (tested: the state hash is identical before and after a screenshot).
- Output is linear, clamped to 1, stored in an sRGB target. No tone mapping and no environment
  lighting yet (metals look dull without reflections).
- MSAA: 1 or 4 samples (the counts every WebGPU device supports), default 4.
- Sprite sampler is linear with clamp; atlas regions are inset by half a texel so neighbouring
  cells never bleed in.

## Why material factors are per draw

The first version had a small uniform buffer per material. On the WARP software rasterizer
(DX12) every mesh then showed the last material's factors, while the per-draw dynamic-offset
data was correct. Moving the factors into the per-draw uniform fixed it, rendered identically on
a desktop GPU of the RTX 2080 class (diff 0), and needs fewer buffers. The root cause (driver or wgpu) is not verified.
WARP also crashed with an access violation when several test threads used it at once; CI runs
the GPU tests with one thread.

## Consequences

- Asset failures are diagnostics (`render.asset`, with the entity), a missing camera is
  `render.no_camera`, no GPU is `render.no_adapter`.
- Shadows, environment maps, tone mapping, mipmaps and skinning are later work.
