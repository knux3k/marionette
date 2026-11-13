---
name: marionette-animate-model
description: Play, cross-fade and blend skeletal animation clips of a glTF model in a Marionette scene, from operations or from a script. Use when a character or prop should move (idle, walk, run), when blending two clips, or when checking where a bone is.
---

# Animate a model

Clips come from the entity's `Model` (a glTF file with animations). The `Animator` component
holds the playing layers; it is saved with the scene and part of the state hash, so animation
is deterministic and replayable.

## Steps

1. List what the model has:
   `marionette op anim_clips '{"asset": "assets/hero.glb"}' --json` gives clip names, lengths,
   nodes, skins and morph targets. `#0` names the first clip when names are missing.
2. Play: `anim_play` with `id`, `clip`, and `scene` (written to the file) or `session` (live
   world only). Every other clip fades out over `fade` seconds (default 0.2). `speed` and
   `looping` are optional.
3. Blend: `anim_blend` sets one clip's weight without touching the others, for example walk
   0.7 and run 0.3 for a jog.
4. Stop: `anim_stop` fades out one clip, or all of them.
5. Check a bone: `anim_pose` returns world position and rotation of nodes after `ticks`
   (attach points, hit checks).

## From a script

`self.playClip(clip, { fade, speed, looping })`, `self.blendClip(clip, weight, options)`,
`self.stopClip(clip)`. Drive blends from game state in `update`, for example speed over top
speed as the run weight.

## Checks

- Take `observe_screenshot` at two different `ticks` values: the pose differs.
- A scenario with a state hash holds the animation steady (skill `marionette-test-game`).
- A model with no clips needs rigging outside the engine first; say so instead of faking it
  with transforms.
