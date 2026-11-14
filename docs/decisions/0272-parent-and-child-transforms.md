# 0272: Parent and child transforms (`ChildOf`)

Date: 2026-11-14. Status: accepted.

## Context

Every entity stood alone in world space. A wheel on a car, a lamp on a wall that moves, a
camera on a body or a held object each needed a script that copied a pose every tick, and a
script runs before physics, so the copy trailed by a tick (proof games, gaps G4 and G16). The
porting map listed all of Unity's and Unreal's parenting as a gap.

## Decision

- `Transform` stays what it always was: where the entity is in the world. The renderer,
  physics, audio, navigation, XR and world UI read it unchanged.
- A child has a `ChildOf` component: `parent` (an entity id) and its local `translation`,
  `rotation` and `scale`. The engine writes the child's `Transform` from the parent's
  `Transform` (with `Position` as the translation when the parent has one) and that local
  placement (`marionette_core::hierarchy`).
- When: once when a scene is loaded (also on reload and restore), and twice per tick, after
  `Stage::Logic` (so a collider on a child is in place for the physics step) and after
  `Stage::Physics` (so a child of a body that physics moved is on it in the same tick).
- Order: parents first, then by id. The order is kept between ticks and found again only when
  a link changed; a tick checks each link with two lookups. A world without `ChildOf` pays one
  empty query.
- Math: translation `p.t + p.r * (p.s * c.t)`, rotation `p.r * c.r`, scale `p.s * c.s` per
  axis; only `+ - * /`. A turned child under a parent with unequal scales is not sheared.
- Cycles are a scene error (`scene.parent_cycle`), found by `scene::validate` in one pass and
  by `SceneIndex` on the edit that would close one, so files, edits and sessions all refuse
  them with the same diagnostic. A cycle or a missing parent that appears while running (a
  script wrote it) leaves those entities where they are; the tick goes on.
- Deleting a parent is refused while a child refers to it (the existing rule for references).
- Physics: a collider with no `RigidBody` on a child follows it as a kinematic body. A child
  with a dynamic body or a character controller is refused (`physics.child_body`): two things
  would move it.
- Operation `entity_attach` (`id`, `parent`): adds `ChildOf` with the local placement that
  keeps the entity where it is, also when it already had another parent. Detaching is
  `component_remove` of `ChildOf`: the `Transform` stays as the engine last wrote it.
- Budget in `engine_budgets` (`hierarchy_budgets`): 10,000 children in chains of four under
  2,500 moving parents, 3 ms per tick (measured 1.3 ms, release, 4-core container).

## Why

- Keeping `Transform` as the world placement means no reader changes and no existing scene,
  hash or reference image changes. The other shape (a local `Transform` plus a computed
  global one) would touch every crate that places anything.
- The local placement lives in the link, so removing the link leaves a valid entity.

## Not done yet

- Children are not listed anywhere (no "children of" query); a child names its parent.
- A script cannot detach (scripts cannot remove components).
- Imported Unity prefabs are still flattened to world transforms (decision 0040).
- The editor refuses to nudge a child instead of moving it in its parent's frame.
- A parent's `LookAt` is not part of what a child inherits (the renderer applies `LookAt`).
