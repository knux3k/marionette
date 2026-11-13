# 0070: Scenes and scripts run job TypeScript over job arrays saved with the scene

Date: 2026-11-13. Status: accepted.

## Context

Job TypeScript (decision 0030) compiled and ran, but only from Rust and the `jobs_run`
operation. A game made of scene data and scripts could not use it, so its hot loops
(particles, crowds, belt slots, machine timers: the factory port target) were either one
script call per entity (about 50x slower) or a Rust module. FOCUSES.md listed it as the open
modularity gap. Questions: where the data lives between ticks, how a scene schedules a job
without code, how scripts call one without the script crate depending on wasmtime, and how
the result is seen.

## Decision

- **Job arrays are core scene data**: component `JobData { arrays: [{ name, values: {F32 |
  F64 | I32: [...]} }] }` (`crates/marionette-core/src/jobdata.rs`). Saved, restored and
  hashed like any component; a run takes the vectors out of the component and puts them back
  (no copy). A game's own Rust jobs can use the same arrays.
- **Scenes schedule steps**: component `JobSchedule { steps: [{ path, function, args, chunk
  }] }`, run by the new `JobsModule` (`crates/marionette-jobs-ts/src/scene.rs`, module name
  `jobs`, behind the existing `jobs-ts` feature) in `Stage::Logic`: entities in scene-id
  order, steps in list order, after scripts and before physics. `args` are short strings, one
  per parameter: `name`, `in:name`, `all:name`, `dt`, `tick`, `len`, `chunk_start`, `rand`,
  numbers, `true`/`false`. Every scheduled file is compiled and every step checked against its
  function's signature and the entity's arrays when the scene loads; problems fail the load as
  `jobts.*` diagnostics with entity, component and field. A step that fails in a tick records
  a `job_error` event and the tick goes on (as script errors do).
- **Scripts call jobs**: `engine.jobs.run/sum/get/set/read/whole/write/chunkStart`. Arguments
  are job array names (in place), the script's own typed arrays (copied in and back), numbers
  and booleans. The script crate reaches the runner through a core resource `JobRunner` (a
  function pointer, the same pattern as `RayCaster`), so `marionette-script` does not depend
  on wasmtime; without the `jobs-ts` feature `engine.jobs.run` throws `jobts.unavailable` and
  `get`/`set` still work.
- **Job arrays are drawn**: component `JobDraw { asset, material, x, y, z, scale }` makes one
  instanced bulk draw (decision 0025) from three arrays, placed by the entity's `Transform`.
  The core adds its bulk draw hook to every world.
- **Hot reload**: operation `jobs_reload` recompiles changed job files of a live session; a file
  that fails keeps its last good version.
- **Example**: `examples/swarm.scene.json` (recipe `examples/swarm.ops.json`): 4,096 particles,
  seeded by a script through a job, stepped by a schedule, summed by a script, drawn with
  shadows. Stability modes `job-swarm` and `job-swarm-render`.

## Alternatives

- **Gather component fields into arrays per tick** (a job reads `Position.x` of every entity
  with a tag): keeps one entity per item, which is what makes 100,000 items slow (entity
  storage, reflection, snapshots); gathering and scattering each tick costs more than the job.
  Rejected for now; `JobData` plus `JobDraw` covers the bulk case without entities.
- **Arrays in a module resource or outside the scene** (not saved): breaks snapshots, replays
  and the state hash. Rejected.
- **Only a script API**: every game would need a script just to call `step` each tick, and a
  script turn per tick per entity. The schedule needs no code and is checked at load.
- **`marionette-script` depending on `marionette-jobs-ts`**: would pull wasmtime into every
  scripted build and undo the `jobs-ts` feature (decision 0062). Rejected for the runner
  resource.
- **A JSON mini-language with objects per argument**: more tokens per scene for the same
  information; strings read naturally (`"in:vy"`) and are validated at load.

## Consequences

- Focus 1 (modularity): closed gap; data in core, compute optional, scripts and Rust modules
  share one data format. Focus 2 (tokens): a schedule step is one line of JSON; errors carry
  the fix. Focus 3 (performance): 131,072 swarm particles cost 2.5 ms per tick on one thread
  and 0.8 ms on four (release, 4-core x86-64 container, `perf_sim`); 4,096 cost 0.09 ms.
  Focus 4 (graphics): `JobDraw` draws any number of copies in one draw call per pass.
- Every element of a job array is written into snapshots and the state hash text; very large
  arrays make `state_hash` and `session_snapshot` slower (not the tick).
- Scheduled steps always run with safety checks on (decision 0030 `Safety::Checked`); an
  unchecked option per step is a follow-up.
- The swarm draws cubes: 4,096 UV spheres (1,024 triangles each) made Mesa lavapipe grow
  resident memory by about 60 MiB in the render stability mode; on a hardware GPU spheres are
  fine. A low-poly sphere primitive is a renderer follow-up.
- Existing state hashes are unchanged (no scene had the new components; drift hash checked).
