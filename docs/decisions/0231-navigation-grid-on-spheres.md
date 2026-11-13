# 0231: Navigation on round, changing worlds uses a cube-sphere cell grid

Date: 2026-11-13. Status: accepted.

## Context

Games need a replacement for a baked navigation mesh that works on spheres, on terrain made
at run time, and while the world changes (buildings placed, obstacles moving), with thousands
of agents and deterministic results. Navigation meshes (Recast style) are built in a flat
up-axis space, take long to rebuild and are hard to make bit-exact.

## Decision

- **New crate `marionette-nav`** (cargo feature `nav`, on by default) with a `NavModule`.
- **A cube-sphere grid per `NavSurface`**: six faces of `resolution²` cells, in the surface's
  local space (a moving or turning world keeps its grid). Cells are placed with a tangent warp
  from a rational approximation (no trigonometry, the same on every platform), so cell edges
  vary by at most 1.8× instead of about 3×. Each cell has 8 neighbours, crossing face seams,
  made symmetric; edge costs are integer millimetres times the mean walk weight.
- **Ground from the physics ray cast** at load (`probe` above and below the radius; slope
  checked as grade without trigonometry; `sea_level`), or from a game's own height function
  (`bake_with`), re-probed in an area after terrain edits (`rebake`).
- **`NavObstacle` stamps a count per cell**: moving, adding or removing one changes only its
  cells, marks the surface changed, and only agents whose remaining route is no longer in
  sight plan again.
- **Snapshots** save routes, the search queue and each grid's baked weights and heights;
  obstacles are stamped again after a restore without counting as a change.
- Operations `nav_path` and `nav_agents`; skill `marionette-round-worlds`.

## Alternatives

- A navigation mesh per cube face: faster long searches, but seams, rebuild cost on change,
  and float-heavy triangulation that is hard to keep bit-exact.
- A flat grid per region: does not wrap around a sphere.
- Hierarchical search (clusters and portals): planned when worlds outgrow one grid; not needed
  for 100k-cell surfaces yet.

## Consequences

- Resolution trades memory and search cost for detail: 128 gives about 98k cells (around
  2 MB with neighbour tables).
- Flat worlds are not covered by this crate yet.
