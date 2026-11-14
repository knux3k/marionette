# 0091: HDR environment files (.hdr, .exr)

Date: 2026-11-13. Status: accepted. Focus 4 (graphics), focus 1 (modularity).

## Context

`Environment` read PNG only (decision 0066), so the sky and sun clipped at 1.0 times
`intensity`: real sky images lost the sun's brightness, reflections of the sun were gray, and
bloom could not pick them up.

## Decision

`Environment.image` may name a Radiance `.hdr` or OpenEXR `.exr` file (by extension).

- `.hdr`: decoded in `marionette-assets` (`hdr.rs`, flat and run-length scanlines, `-Y h +X w`),
  no new dependency.
- `.exr`: through the `exr` crate behind cargo feature `exr` of `marionette-assets` (on by
  default). A build without it reports a clear `render.asset` error.
- Pixels stay linear and unclamped up to 60,000 (half-float range). Resampling to the 256x128
  base averages every source texel a base texel covers (box filter), so a small bright sun keeps
  its energy; small sources are sampled bilinearly as before. Prefiltering is the same as for
  PNG.
- `examples/assets/sky-sun.hdr` (made by a script, a sky with a sun disc) and
  `examples/assets/flat-2.hdr` (test file) are in the repository.

## Alternatives

- The `image` crate with its `hdr` and `exr` features: one more large dependency in the
  renderer for two formats; the Radiance format is 150 lines.
- Clamping HDR values to some lower limit: avoids fireflies in rough reflections, but throws
  away exactly the brightness the format exists for. The prefilter blur spreads it instead.

## Consequences

- An HDR image with a sun also lights diffuse surfaces through its spherical harmonics; a scene
  that also has a `DirectionalLight` for the sun counts it twice unless the image's sun is
  small or `intensity` is lowered. Documented on `Environment`.
