# 0082: Foveated rendering by skipping pixel quads outside an ellipse (`Foveation`), opt-in

Date: 2026-11-13. Status: accepted. Focuses 3 (performance) and 4 (graphics), decision 0060.

## Context

A headset lens is sharp in the middle and blurry at the edges, so full shading at the edges is
wasted. Hardware variable-rate shading would be the cheapest way to shade less there, but
wgpu 30 does not expose it (its `Features` list has multiview, ray queries and mesh shaders,
no shading-rate feature, checked in the wgpu-types 30.0.1 source).

## Decision

Resource `Foveation { inner_radius }` (share of the half width and height; the ellipse touches
the image edges at 1.0). It uses the internal image of decision 0081 (at the output size when
`RenderScale` is absent).

1. Before anything is drawn in a view, a full-screen mask pass writes depth 0 (the near plane)
   into every other 2x2 pixel quad outside the ellipse (a checkerboard of quads, because GPUs
   shade in 2x2 quads: skipping single pixels would save nothing). Models, sky and sprites then
   fail the depth test there and are not shaded. This is "radial density masking", known from
   VR rendering talks.
2. The resolve pass fills each skipped pixel from the nearest pixels of the four neighbouring
   quads, weighted by inverse distance (`fetch` in `shaders/scale.wgsl`).
3. Works in the plain and the bloom path (the mask has a variant for the HDR target).

## Alternatives

- Lower-resolution periphery images composited around a full-resolution center: saves more at
  the edges but draws the geometry twice per eye.
- Variable-rate shading or `VK_EXT_fragment_density_map`: best, but not in wgpu; revisit when
  wgpu exposes it.
- Eye-tracked foveation: needs a tracked gaze from the headset runtime; the ellipse can follow
  a gaze point later without changing the technique.

## Consequences

- About half the pixels outside the ellipse are not shaded: with 0.6, about 36% of a square
  eye image. The saving is in fragment shading only; geometry cost is unchanged.
- High-contrast edges in the periphery look slightly stair-stepped up close on a flat screen
  (visible in a screenshot, intended to fall in the lens blur); the middle is unchanged
  (tested pixel for pixel without bloom, within 2 of 255 with bloom, whose glow blurs a little
  of the filled edge in).
- Meant for stereo headset views; a game turns it on for its headset mode.
- On its own it adds a copy pass, and on small example scenes it was slightly slower on a
  desktop GPU; it pays only where edge shading is costly. With `RenderScale` below 1 the
  periphery is upscaled bilinearly (a 16-tap filter that filled holes per tap made the pair
  1.6x to 2.8x slower than plain).
