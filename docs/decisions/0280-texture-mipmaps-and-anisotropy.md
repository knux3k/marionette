# 0280: Model textures get mipmaps made on the CPU and 16x anisotropic filtering

Date: 2026-11-14. Status: accepted. Focuses 3 (performance) and 4 (graphics), decision 0060.

## Context

Model textures were uploaded with one level and sampled bilinearly: far and grazing surfaces
shimmered (moire), and every far texel fetch touched the full-size image, which costs memory
bandwidth. An independent audit (2026-11-14) listed this first among renderer gaps.

## Decision

1. **Mipmaps for every model texture**, built on the CPU by `marionette_assets::mips::chain`
   while the model decodes (in the background loader when streaming, decision 0068), then
   uploaded with the image. Each level halves the one above with a 2x2 box filter:
   - color (sRGB) averaged in linear light and weighted by alpha, so cut-out edges do not
     darken;
   - normal maps averaged as vectors and renormalized;
   - data (metallic-roughness) averaged as numbers;
   - textures of cut-out materials (decision 0281) keep their alpha coverage at the cutoff
     on every level (alpha scaled by bisection), so foliage and fences do not thin out with
     distance.
   The use comes from the materials that reference the image.
2. **Sampler**: trilinear with `anisotropy_clamp` 16 (`MATERIAL_ANISOTROPY`); wgpu clamps it to
   1 on adapters without anisotropic filtering.
3. **Sprites, UI and fonts keep one level**: they are drawn near pixel size and atlases would
   bleed between cells.

## Alternatives

- GPU mip generation (a blit chain): faster on huge textures, but results differ between GPUs
  and drivers (filter, rounding), it needs a render or compute pass per level per texture, and
  cannot do coverage-preserving alpha or vector normals without a custom shader. CPU levels are
  deterministic and run off the frame.
- Precomputed mips in the asset (KTX2): kept for compressed textures (decision 0273); plain
  PNG and JPEG assets still need levels.

## Consequences

- About one third more texture memory; far texture reads cheaper.
- `examples/reference/showcase.png` was rewritten: its textured gem is now mipmapped (0.7% of
  pixels past the threshold, under the 1% limit but too close for other GPUs). The other
  reference images have no textured models and were kept.
- Test `mipmaps_average_far_textures` (spread about 18 without mipmaps, about 0 with them) and
  unit tests in `crates/marionette-assets/src/mips.rs`.
