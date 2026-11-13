---
name: marionette-make-vr
description: VR headsets through OpenXR, hands, grab and throw, VR movement. Use when a game needs VR.
---

# Make a game playable in VR

VR is optional data on top of a normal scene. Tracked poses arrive as input actions, so a
headset run records and replays like any other run, and tests need no headset.

## Scene

1. A rig entity: `XrRig` (`height_offset` 0 when standing) and a `Transform` placing the play
   space in the world. Add `XrLocomotion` for movement: action names for `move_x`, `move_y`,
   `turn`, `teleport` (empty string turns a part off), `turn_mode` "Snap" (most comfortable),
   "Smooth" or "Off", `vignette` 0..1.
2. The head: `XrTracked` with `source` "Head" and `rig` set to the rig's id, plus `Camera`
   (near about 0.05) and `AudioListener`.
3. Hands: `XrTracked` with sources "LeftGrip" and "RightGrip", each with `XrHand` (`grab_action`,
   `grab_radius` about 0.15, `throw_scale` 1) and a `Model`.
4. Pick-ups: `Grabbable`. With a dynamic `RigidBody`, `Velocity` and `AngularVelocity` they
   keep colliding while held and fly when released.
5. Action map: bind actions to `xr:left/...` and `xr:right/...` (`trigger`, `squeeze`,
   `thumbstick_x`, `thumbstick_y`, `thumbstick_click`, `primary`, `secondary`, `menu`,
   `pinch` for hand tracking). Keep keyboard bindings too, for desktop runs.

Use `project_schema` for exact fields of `XrRig`, `XrTracked`, `XrHand`, `Grabbable` and
`XrLocomotion`.

## Test without a headset

- Script poses as directly set actions `xr.<source>.px` .. `xr.<source>.qw` and
  `xr.<source>.tracked` (sources `head`, `left`, `right`, `left_aim`, `right_aim`; metres in
  play space, -Z forward) plus `xr:` inputs, in an inputs file passed to `scene_run`; check
  `Transform` of the held object and the rig, and that two runs give the same `state_hash`.
- Register a stability mode with `views` 2 for the VR scene.

## On a headset

1. `marionette xr --probe`: runtime, headset name, eye size, hand tracking. Error
   `xr.no_runtime` means no OpenXR runtime is active; `xr.no_headset` means it is asleep or
   unplugged.
2. `marionette xr <scene> --record vr.inputs.json`: plays until the player quits from the
   runtime's menu. Replay the recording headless to reproduce a bug exactly.
3. Only a person wearing the headset can judge comfort, bindings and hand tracking; ask them,
   with a short list of what to try.

## Shipping

Put the OpenXR loader (`openxr_loader.dll` on Windows) next to the game's executable.
