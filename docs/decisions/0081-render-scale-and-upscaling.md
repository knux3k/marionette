# 0081: Render scale with an edge-preserving upscaler (`RenderScale`), opt-in

Date: 2026-11-13. Status: accepted. Focuses 3 (performance) and 4 (graphics), decision 0060.

## Context

GPU cost grows with pixels: a headset eye is about 2160x2160, twice a 1080p screen, and drawn
twice. The common way to buy back frame time is to draw fewer pixels and resize (DLSS, FSR,
XeSS); the opposite (supersampling) buys the cleanest edges when there is time to spare. The
engine had neither. DLSS cannot ship in this repository (decision 0083).

## Decision

Resource `RenderScale { scale, sharpness }`, `scale` clamped to 0.5..2.0. Absent, or scale 1
without `Foveation`, nothing changes (same image, same passes; tested).

1. The world (models, sky, sprites, bloom) is drawn into an internal image of output size
   times `scale` (same color format and MSAA as the output, `crates/marionette-render/src/scale.rs`).
   Images are kept per size and shared by views of that size, like bloom's.
2. An output pass resolves it (`shaders/scale.wgsl`), then draws UI at full resolution:
   - below 1: Catmull-Rom (16 taps), clamped to the 2x2 nearest texels so edges get no
     ringing halos, then `sharpness` pushes the result away from plain bilinear (an unsharp
     mask inside the same clamp);
   - above 1: a 2x2 grid of bilinear taps over each output pixel's footprint (a box filter of
     up to 4x4 texels);
   - at 1 (foveation only): a straight copy.
3. Culling and level of detail use the same views, so they are unaffected.

## Alternatives

- Port AMD FSR 1 (EASU + RCAS, MIT license): better edge reconstruction than Catmull-Rom at
  0.5 to 0.67. A WGSL port written from the published source is the planned upgrade; this
  first version is written from first principles so no third-party shader code needs a
  license notice yet.
- Temporal upscaling (FSR 2/3, DLSS, TAA-based): the best quality per pixel, but needs motion
  vectors and a jittered projection the renderer does not have, and ghosts on fast motion in
  a headset. Later, together with temporal anti-aliasing.
- A separate sharpening pass (RCAS-style): one more full-screen pass; the clamped unsharp mask
  inside the resolve costs nothing extra.

## Consequences

- Below 1 the image is softer than native; it is a choice a game makes (opt-in), never a
  default. Above 1 costs GPU time roughly in proportion to `scale` squared.
- Memory: one internal color image (plus its MSAA copy and depth) per output size.
- One extra pass per view, about 0.2 to 0.35 ms at headset size on a desktop GPU, so scale
  pays only in scenes whose shading costs more than that (0.5 saved 26% on a bloom scene;
  0.75 was slower on a light one). Numbers: `docs/ai/proofs/render.md`.
