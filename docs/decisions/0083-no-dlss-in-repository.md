# 0083: No DLSS

Date: 2026-11-13. Status: accepted. Focuses 1 (modularity) and 3 (performance).

## Context

The owner asked whether the engine has DLSS. Read on 2026-11-13: the NVIDIA RTX SDKs License
(version of 2023-01-23, which covers DLSS), the NVIDIA DLSS repository README, and the README of
`dlss_wgpu` (a crate that connects DLSS to wgpu).

## Findings

- The SDK may be distributed only "as incorporated in object code format into a software
  application" (section 1.c), not as a stand-alone product (4.b), and "You may not use the SDK
  in any manner that would cause it to become subject to an open source software license"
  (4.e). It cannot be committed to an MIT/Apache repository.
- Shipping a game with it adds duties: NVIDIA attribution and marks on splash screen, about box
  and credits, notifying NVIDIA before commercial release, and use only on NVIDIA GPUs.
- `dlss_wgpu` does not bundle the SDK: each developer downloads it and points a variable at
  it; Vulkan backend only.
- DLSS Super Resolution is temporal: it needs motion vectors and a jittered projection, which
  this renderer does not produce yet (the input list is from general knowledge of DLSS, not
  from the pages read).

## Decision

No DLSS code, files or add-on point for it. After these terms were summarised, the owner chose
not to use DLSS at all (2026-11-13). Vendor-neutral options only: render scale with an
upscaler (decision 0081), foveation (decision 0082), and later temporal upscaling written in
the engine if motion vectors and temporal anti-aliasing land.
