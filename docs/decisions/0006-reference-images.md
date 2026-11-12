# 0006: Reference images and their tolerance

Date: 2026-11-11. Status: accepted.

## Decision

`examples/reference/{showcase,sprites}.png` are 320x180, MSAA 4, 60 ticks, seed 1. The test
`matches_reference_images` passes when at most 1% of pixels differ by more than 24/255 in any
channel and the mean absolute difference is at most 2/255. Regenerate with
`MARIONETTE_UPDATE_REFERENCES=1 cargo test -p marionette-render --test render`, and only for an
intended visual change after looking at the new images.

## Why this tolerance

GPUs and drivers may legitimately differ in edge coverage (MSAA sample positions and resolve),
texture filtering precision and shading precision. That shows up as small differences on many
pixels and larger ones on a few edge pixels. Real regressions (wrong material, missing object,
wrong camera) change large areas by large amounts. Measured against the committed references
(made on a desktop GPU of the RTX 2080 class with Vulkan):

| Adapter | showcase max / mean | sprites max / mean |
| --- | --- | --- |
| Desktop GPU (RTX 2080 class), Vulkan | 0 / 0 | 0 / 0 |
| Desktop GPU (RTX 2080 class), DX12 | 0 / 0 | 0 / 0 |
| Desktop GPU (RTX 2080 class), GL | 0 / 0 | 0 / 0 |
| Microsoft Basic Render Driver (WARP, CPU) | 4 / 0.013 | 3 / 0.194 |

The thresholds leave a wide margin over WARP's differences, and a test checks that a different
scene fails the comparison. Same-machine determinism is checked separately and exactly
(identical PNG bytes).

## Not yet measured

AMD, Intel and Apple GPUs, and Mesa lavapipe (what Linux CI uses). If one exceeds the tolerance,
look at the difference before widening it.
