---
name: marionette-test-game
description: Test a game with scenario files (scene, seed, ticks, input, assertions, reference image), replay recorded input, and run seeded stability soaks. Use when behaviour was built or changed, a test fails, a state hash differs between runs, or checking a game does not crash, hang or leak.
---

# Test a game

Runs are deterministic: the same scene, seed, ticks and input give the same state hash on
every machine. Tests rely on that, so a hash that moves is a bug to find, not noise.

## Scenario tests

One file per behaviour in `tests/<name>.test.json`:

```json
{
  "scene": "scenes/main.scene.json", "seed": 1, "ticks": 120,
  "input": "tests/jump.inputs.json",
  "assertions": [
    { "count": { "with": ["Camera"], "equals": 1 } },
    { "field": { "entity": 4, "path": "Position.x", "op": "gt", "value": 0 } }
  ],
  "image": { "reference": "tests/main.png", "camera": 1, "size": "320x180" }
}
```

Assertion kinds: `field`, `count`, `state_hash`, `event`. `input` and `image` are optional.
Run all with `marionette op test_run '{}' --json`; the result is data (`passed`, `failed`,
each assertion). Try one without a file by passing it inline as `scenario`.

Prefer `field` and `event` assertions for behaviour: they say what broke. Add a `state_hash`
only to pin a whole run, and expect to update it when behaviour changes on purpose.

## Input for tests

Play a live session (`session_start`, `session_step` with `events`), then
`session_record_save` writes a replayable `*.inputs.json`. From the command line:
`marionette run <scene> --headless --ticks 600 --seed 1 --input <file>` prints the hash.

## When a hash differs between two runs

Look for, in order: wall-clock time, randomness not from `engine.rand()`, state kept in
module-level script variables, order taken from an unordered map, and anything that depends
on thread timing. Halve `ticks` to find the first tick that differs. `session_snapshot` and
`session_restore` let you rewind instead of rerunning.

## Stability soak

`stability/suite.json` lists game modes (`format` `marionette.stability-suite`, `version` 1,
`seed`, and `modes`, each with `name`, `description`, `kind` `sim` or `render`, `scene`,
`random_input`, `fast_ticks`, `long_ticks`, `tick_budget_ms`). Each mode runs with seeded
random input and is checked for panic, hang, replay and snapshot determinism, time budget and
memory growth: `marionette stability --mode <name>`, add `--long` for a soak. Give every playable scene a
mode. A failure writes a crash report with a `reproduce` command and the exact input: run
that command first, it fails the same way.

## Checks

`project_validate` clean, `test_run` all passed, and the stability mode of every scene you
touched passes.
