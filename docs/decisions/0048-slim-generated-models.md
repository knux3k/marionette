# 0048: Slim generated models to their budgets

Date: 2026-11-12. Status: accepted.

## Context

The first 19 Tripo models came back with 94k to 503k triangles and 5 to 19 MB each, against
recipe budgets of 3k to 20k triangles. Decision 0045 only reported the overrun. Models that
size cost frame time (focus: performance) and would bloat the repository for good.

## Decision

- Model post-processing slims the GLB before normalizing it (`marionette_generate::slim`):
  - Over `max_triangles`: every triangle primitive is simplified by the same ratio with
    meshoptimizer (`meshopt` crate, the reference C++ library; seams and UVs kept, small
    disconnected bits pruned). A primitive the careful pass cannot bring near its share falls
    back to the grid pass. Unused vertices are dropped; indices are reordered for the vertex
    cache.
  - `max_texture_px` (new recipe field): embedded images larger than that are halved until
    they fit and re-encoded, JPEG quality 85 when opaque, PNG otherwise.
  - The binary chunk is rebuilt; the file's own JSON is edited, so materials, extensions and
    extras are kept. Skinned, animated or morphing models, sparse accessors and multi-buffer
    files are left alone with a note.
- Provenance notes record what was done (`decimated from A to B triangles`, each resized image).
- Post-processing options are not part of the cache key, so changing them rebuilds from the
  cache offline at no cost.

## Consequences

- The C++ library is compiled by `cc`; Windows and Linux CI already have compilers.
- Output bytes depend on meshoptimizer's float math; a model is built once on one machine
  and its hash recorded, so cross-platform byte identity is not required.
