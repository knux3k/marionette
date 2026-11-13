# 0066: Environment lighting, reflections and sky background

Date: 2026-11-13. Status: accepted. Focus 4 (graphics, decision 0060).

## Context

Ambient light was a flat color or a two-color hemisphere (decision 0063). Shiny and metal
surfaces reflected nothing but a tint, so metal looked gray and plastic looked flat. The
background was the camera's clear color.

## Decision

Resource `Environment { image, intensity, background }`. Absent, nothing changes.

- Source: an equirectangular PNG (2:1, relative to the scene file), or, with `image` empty, a
  sky made on the CPU from `SkyLight`'s sky and ground colors (or a default dusk-blue sky): sky
  overhead, brighter toward the horizon, ground below with a soft edge.
- Built once per source change on the CPU (`crates/marionette-render/src/environment.rs`):
  resampled to 256x128, then five more mip levels down to 8x4, level `k` blurred for roughness
  `k / 5` (separable Gaussian on the sphere, horizontal width divided by the cosine of the
  latitude, variances adding level by level). Stored as one `Rgba16Float` texture (350 KB).
  Diffuse light: nine spherical harmonics coefficients of the radiance convolved with the
  cosine lobe.
- Shading: replaces the flat or sky ambient for models. Diffuse = irradiance x base color x
  (1 - metallic); specular = the mip matching the roughness, sampled along the reflection,
  times an analytic split-sum BRDF fit (Karis, mobile); both times `intensity`.
- `background: true` draws the environment behind everything: a full-screen triangle at the
  far plane after the models, so only uncovered pixels run it. With `Fog`, the sky is fogged as
  if it were 3 / density meters away at the horizon and farther upward, so the ground meets
  the sky without an edge.
- A missing or unreadable image is a `render.asset` error with component `Environment`.

## Alternatives

- Prefiltering on the GPU with compute shaders: faster for large HDR images, but adds pipelines
  and a compute path for something that runs once per scene. The CPU build takes milliseconds
  at these sizes.
- Cube maps instead of equirectangular images: less distortion at the poles, but six faces to
  load and blur; equirectangular is what free sky images come as.
- Reading HDR (`.hdr`, `.exr`) images: needs a decoder dependency; PNG times `intensity`
  covers the proof scenes. HDR files are a follow-up if a game needs real sun intensities.
- A BRDF lookup texture: exact split-sum values, one more texture; the analytic fit is within
  a few percent.

## Consequences

- Shaders: one `atan2`/`acos` lookup and one filtered sample per pixel for reflections, nine
  SH terms for diffuse. Render only; nothing in the simulation or state hash.
- No reflections of other objects (no screen-space or probe reflections); a mirror shows the
  sky and ground only.
- Fixed tuning note from decision 0063 ("fog does not hide the far ground edge"): the lighting
  example's ground was 40 m wide, so its edge stood 20 m away where fog was half thick. The
  ground is now 300 m wide (beyond the camera's far plane), and with the sky background on,
  fog also blends the horizon.
