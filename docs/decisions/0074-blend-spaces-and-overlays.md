# 0074: Blend spaces and masked or additive overlays

Date: 2026-11-13. Status: accepted.

## Context

Skeletal animation (decision 0028) had cross-fades and a weighted average of clips, nothing
more. Compared with other engines it lacked: clips mixed by a parameter (idle, walk, run by
speed; eight directions by velocity), cycles kept in step while they mix (feet slide
otherwise), and layers that touch only some bones (wave while walking) or add motion on top
(a nod, a breath). Existing scenes and their state hashes must not change.

## Decision

- **New optional components, no new fields** on `Animator` or `AnimLayer`: the state hash is
  the scene text, so a new field would change every animated scene's hash. `BlendSpaces` and
  `AnimOverlays` (`crates/marionette-anim/src/blend.rs`) sit next to `Animator`.
- **A blend space is played by name.** An `Animator` (or overlay) layer whose `clip` matches a
  space name plays the space; spaces are looked up before model clips. Cross-fades between a
  space and a clip or another space need nothing new.
- **Weights by gradient band interpolation** (cartesian), the freeform method other engines
  use for 2D blend trees. With all points on a line it is linear interpolation between
  neighbours; outside the points the edge holds. Only `+ - * /`, so deterministic.
- **Sync by phase.** A synced space (default) stores its phase (0..1) in the layer's `time`
  and advances it at `1 / sum(weight * clip duration)`, so every clip is at the same point of
  its cycle. Unsynced spaces keep seconds and wrap each clip by its own length.
- **Parameters glide on the tick** toward a target at `rate` units per second, saved in the
  component like layer weights (snapshots and replays repeat exactly).
- **Overlays** apply in list order on top of the base pose, each with a strength that fades
  like a layer weight, a mask of bone names (each with its children; empty = whole model) and
  override or additive mode. An override overlay only touches masked bones that its clips
  animate, so a mask never snaps a bone to its rest pose. Additive motion is relative to the
  clip's first frame (local delta: `inverse(first) * now`, scaled by strength with nlerp).
- **One implementation for every entry point**: operations `anim_space`, `anim_overlay`
  (and `anim_play` accepting space names), script `setBlend`, `playOverlay`, `stopOverlay`
  all call the same `marionette-anim` functions. Unknown clips (also inside a space) are
  `anim.unknown_clip`; scripts steering a missing space throw `anim.unknown_space`.

## Alternatives

- Delaunay triangles for 2D spaces: exact barycentric weights, but needs a triangulation
  that changes discontinuously when points move and fails on collinear points. Gradient band
  needs neither and costs O(n^2) per space per tick for n points (n is small).
- A full state machine graph (states, transitions, conditions): larger, and scripts already
  decide when to play what. Left for later if games need it.
- Inverse distance weighting: never reaches exactly one clip between points and blurs.

## Consequences

- Focus 1 (modularity): new components in the existing anim crate; games that do not use
  them pay one optional component lookup per animated entity per tick.
- Focus 3 (performance): a space samples each clip it mixes (usually two non-zero), an
  overlay samples its clips twice when additive (reference frame). Not measured on a hardware
  GPU yet (UNVERIFIED in STATE.md).
- Root motion, IK, state machines and per-bone mask weights stay open.
