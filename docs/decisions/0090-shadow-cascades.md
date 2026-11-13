# 0090: Sun shadow cascades

Date: 2026-11-13. Status: accepted. Focus 4 (graphics), focus 3 (performance).

## Context

The sun's shadow was one map around a point ahead of the camera (decision 0063). A map small
enough to be sharp up close stopped a few meters away; one large enough to reach the horizon
was blurry everywhere. Open levels (racing, top-down, a factory floor seen from a headset)
need both.

## Decision

Resource `ShadowCascades { count, far }`, used with `Shadows`. Absent, nothing changes.

- The view from the camera's near plane to `far` (capped at the camera's far plane) is cut into
  `count` slices (2 to 4), splits 0.75 logarithmic plus 0.25 even.
- Each slice gets an orthographic map around the bounding sphere of its eight corners. The
  sphere depends only on the slice's shape, so turning the camera does not resize the map; its
  center snaps to whole texels, so moving does not make edges crawl. Room behind each slice
  reaches back `far` meters for casters between the slice and the sun.
- All cascades are layers of one depth texture array (`Shadows.resolution` each); the sun's
  map became a one-layer array, so the binding is the same with or without cascades.
- The shader takes the first cascade whose map holds the point with the 3x3 filter inside it;
  normal offset per cascade from its own texel size.
- Orthographic cameras keep one map. Stereo: cascades follow the first view, as before.

## Alternatives

- Cascades fitted to the slice's box in light space: tighter, but the size changes as the
  camera turns, so shadows shimmer. Spheres waste some texels and never shimmer.
- Blending between cascades: hides the seam, costs a second lookup in the band; the seam is a
  resolution step only, left for when a scene shows it.
- One separate texture per cascade: more bindings for no gain.

## Consequences

- Draw calls rise by the mesh group count per extra cascade (tested).
- Frame cost on the desktop GPU: in PROOFS.md ("Remaining lighting").
