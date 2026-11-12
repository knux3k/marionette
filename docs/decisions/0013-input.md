# 0013: Input as actions, recorded at action level

Date: 2026-11-11. Status: accepted.

## Decision

- Action map file (`*.actions.json`, format `marionette.input-map`): named actions, each a list
  of bindings `{input, scale}`; value = clamp(sum of device value times scale, -1, 1). Input
  names: `key:<KeyCode>`, `mouse:<Button>`, `pad:<Button>`, `axis:<Axis>`. Scenes opt in with
  the `InputConfig` resource.
- Events file (`*.inputs.json`, format `marionette.input-events`): `{tick, input | action,
  value}`. The same format serves scripts and recordings.
- Live devices, scripts and recordings all enter one queue keyed by tick; `Stage::Input` applies
  the tick's events and computes `ActionState` (`values`, `previous`), a scene resource that is
  part of the state hash.
- A recording stores every change of an action's value at its tick. Raw device values and
  directly set actions live in `DeviceState`, which is saved in snapshots but not hashed, so a
  replay that sets actions directly hashes the same as the original run that pressed keys.

## Why

Agents think in actions, not keys. Recording at action level makes replay independent of
bindings and devices. One format for scripts and recordings means a recording can be edited
and replayed like any script.

## Not done yet

Live gamepads: the map format supports them, but no gamepad backend is wired.
