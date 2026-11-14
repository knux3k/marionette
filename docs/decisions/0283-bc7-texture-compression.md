# 0283: Opt-in BC7 compression of model textures with a bounded disk cache

Date: 2026-11-14. Status: accepted. Focuses 1 (modularity), 3 (performance) and 4 (graphics),
decision 0060.

## Context

Model textures were uploaded as RGBA8 (4 bytes per texel, 5.6 MB for a 2K texture with mips).
Other engines ship block-compressed textures by default (audit 2026-11-14). Compression cuts
memory and sampling bandwidth by 4x but loses a little detail, and encoding is slow.

## Decision

1. **Resource `TextureCompression { normal_maps }`** turns it on per scene; off by default,
   so existing pictures and reference images do not change. `normal_maps: false` keeps normal
   maps exact for smooth mirror-like surfaces.
2. **BC7 only**, encoded with Intel's ISPC texture compressor (`intel_tex_2`, MIT or Apache,
   prebuilt kernels for Windows and Linux on x86-64 and ARM64), `basic` quality, opaque or
   alpha settings per texture. Every mip level is encoded from the CPU-made chain (decision
   0280), so mip filtering is unchanged. Rows of blocks are split across the CPU's threads.
3. **Where it applies**: model textures whose size is a multiple of 4, on devices with
   `TEXTURE_COMPRESSION_BC` (requested where the adapter has it, desktop and OpenXR devices).
   Elsewhere textures stay RGBA8. Sprites, UI and fonts are never compressed.
4. **Disk cache**: one file per texture keyed by SHA-256 of the texels and settings, in
   `MARIONETTE_TEXTURE_CACHE` (`off` disables) or the user cache folder
   (`%LOCALAPPDATA%\marionette\textures`, `$XDG_CACHE_HOME` or `~/.cache`). Written to a
   temporary name then renamed; a file that does not parse is re-encoded. Bounded at 2 GB:
   past it the least recently used files go until it is under 1.5 GB.
5. Encoding runs in the model decoder (the background loader when streaming). Turning the
   resource on or off reloads loaded models once.
6. Cargo feature `compress` (default on in `marionette-render`); without it the resource is
   accepted and nothing is compressed.

## Results (cloud container, lavapipe)

- Material ball scene: texture bytes 283,984 to 71,104 (25.0%); with exact normal maps
  136,612. Picture against uncompressed: mean difference 0.05 of 255, one pixel in 57,600 over
  the reference threshold (limit 1%).
- Encoding a 2K opaque texture with its mips: 0.85 s on 4 threads (release).

## Rejected

- ASTC for GPUs without BC: `intel_tex_2` 0.5 leaves its ASTC encoder unimplemented. Those
  GPUs keep RGBA8 until another encoder is added.
- BC5 for normal maps: needs a shader change to rebuild z; BC7 is within tolerance.
- Compressing by default: changes every textured picture; left to the scene.
- `texture-compressor` crate: needs a newer Rust than the workspace pins.

## Limits

- On Linux the binary links the C++ runtime (`libstdc++`), because the crate bundles a C++
  object.
- First load of a scene with many 2K textures takes seconds per texture core; later loads
  read the cache.
