# Porting Unity and Unreal code to Marionette

Recipe for an agent asked to bring a Unity (C#) or Unreal (C++ or Blueprint) game, or part of
one, into Marionette. The design is decision 0020; the concept tables are
[reference/porting-map.md](reference/porting-map.md) (generated from
`crates/marionette-port/data/`). Scripts are TypeScript (decision 0022): an ES module named by a
`Script { path }` component, exporting hooks that take `self: Entity`.

## The model in one paragraph

A Unity GameObject or an Unreal Actor is an **entity** (stable id and optional name in the scene
file). Engine components (Rigidbody, BoxCollider, Camera, UStaticMeshComponent, ...) become
**data components** in the scene file, set through operations. A MonoBehaviour, an Actor
subclass or an ActorComponent becomes **one TypeScript script** on the entity: its fields become
`self.state` entries (saved and part of the state hash), its lifecycle methods become exported
hooks. There is no frame-rate `Update`: everything runs in **fixed ticks** (`engine.dt`, default
1/60 s), in a fixed order, and must be **deterministic** (same seed and input give the same
state hash on every machine).

## Steps

1. **Scan.** `marionette op port_scan '{"source": "<Assets or Source folder>"}'` (read-only; MCP
   tool `port_scan`). Read `summary` first: script count, uses by status, `actions`, `hazards`,
   `top_gaps`. Each class lists its role, fields (with TypeScript type, kind, default, whether the
   source editor showed it), methods (the hook each goes into, coroutines, delegate bindings) and
   the components its entity needs. `api` lists every engine call with its Marionette equivalent,
   count, argument names and locations, hazards and gaps first.
2. **Scaffold.** `marionette op port_scaffold '{"source": "...", "out": "scripts/ported",
   "scene": "scenes/main.scene.json"}'` writes one `.ts` skeleton per gameplay class and
   `PORT-NOTES.md`. Each skeleton compiles as is (`tsc --strict`) and holds the original code as
   comments in the hook it belongs to; every line that uses an engine API is followed by a `->`
   line with the equivalent and its status. It never overwrites unless `overwrite: true`; use
   `dry_run: true` to see the file list.
3. **Build the scene** with operations (`entity_create`, `component_set`; never hand-edit scene
   JSON): one entity per GameObject/Actor, the components from the class report, and
   `"Script": {"path": "<script_path from the scaffold result>"}`. Copy positions, rotations and
   sizes from the source scene through `port_convert` (axes and units differ, see below).
4. **Define input actions** in the scene's action map (`*.actions.json`) for every name in
   `summary.actions`; scripts read actions, never raw keys.
5. **Write the code.** Replace the comments in each skeleton with TypeScript, following the
   `->` lines and the rules below. Fix every `hazard`. Work around each `gap` as its note says.
6. **Prove it.** Run the scene headless twice with scripted input and the same seed; the state
   hashes must match (`scene_run` or `marionette run ... --headless --ticks N --seed S --input
   ...`). Write a scenario test (`test_run`) for the behaviour you ported.
7. **Record** what could not be ported (gaps left, behaviour changed) at the end of
   `PORT-NOTES.md`. If a gap blocks several classes, it is engine work: add it to
   `docs/ai/STATE.md` under next steps.

## Statuses

| Status | Meaning | What to do |
| --- | --- | --- |
| `ready` | Exists now as data: a component, an action map entry, a file, an operation. | Port as data. |
| `script` | A direct call or hook in the script API with the same meaning. | Translate the call. |
| `rewrite` | Possible, but the code takes another shape. | Follow the note (timers, coroutines, tags, input bindings). |
| `gap` | No equivalent yet. | Use the note's workaround; record what is left. |
| `hazard` | Breaks determinism. | Must be rewritten; the note says with what. |
| `unknown` | An engine call not in the tables. | Look it up; if it recurs, add an entry to the table. |

## Lifecycle

| Unity | Unreal | Marionette |
| --- | --- | --- |
| `Awake`, `OnEnable`, `Start` | constructor defaults, `BeginPlay` | `export function start(self: Entity)`: field defaults first, then Awake code, then Start code |
| `FixedUpdate`, `Update`, `LateUpdate` | `Tick(DeltaTime)` | `export function update(self: Entity)`, once per fixed tick; FixedUpdate code first |
| `OnCollisionEnter` (2D too) | `NotifyHit`, `OnComponentHit.AddDynamic` | `export function onCollisionEnter(self: Entity, other: Entity)` |
| `OnCollisionExit` | - | `export function onCollisionExit(self: Entity, other: Entity)` |
| `OnTriggerEnter` (2D too) | `NotifyActorBeginOverlap`, `OnComponentBeginOverlap.AddDynamic` | `export function onTriggerEnter(self: Entity, other: Entity)` |
| `OnTriggerExit` | `NotifyActorEndOverlap`, `OnComponentEndOverlap.AddDynamic` | `export function onTriggerExit(self: Entity, other: Entity)` |
| `OnCollisionStay`, `OnTriggerStay` | - | a flag in `self.state` set on enter, cleared on exit, used in `update` |
| `OnDestroy` | `EndPlay`, `Destroyed` | `export function onDestroy(self: Entity)`, called when `engine.destroy` removes the entity |
| `OnDisable` | - | gap |

Triggers: give the trigger a `Collider` with `sensor: true`; its overlaps arrive as trigger
hooks. The other side is an `Entity`, not a Collision or hit result.

## Rules that matter most

- **Time.** `Time.deltaTime`, `DeltaTime`, `GetDeltaSeconds()` -> `engine.dt`. `Time.time`,
  `GetTimeSeconds()` -> `engine.tick * engine.dt`. Wall-clock time (`realtimeSinceStartup`,
  `DateTime.Now`, `FPlatformTime::Seconds`) is a hazard.
- **Input.** `Input.GetAxis("Horizontal")`, `BindAxis("MoveForward", ...)` -> an action in the
  action map, read with `engine.action(name)`; buttons with `engine.pressed(name)` and
  `engine.justPressed(name)`. Bound Unreal handlers move into `update`. Character movement
  (`CharacterController.Move`, `AddMovementInput`, `Jump`) is the `CharacterController`
  component driven by its actions: drop the calls, bind the axes.
- **Components are data.** `GetComponent<Rigidbody>()`, `FindComponentByClass<T>()` ->
  `self.get("RigidBody")` returns a copy: change it, then `self.set("RigidBody", value)`.
  `transform.position` / `GetActorLocation()` -> `self.get("Transform").translation`.
- **References.** Fields holding a GameObject or Actor pointer become entity ids in
  `self.state`; look them up with `engine.entity(id)` or `engine.find(name)`. Tags become a
  marker component or a name check.
- **Timers** (`Invoke`, `InvokeRepeating`, `SetTimer`, `SetLifeSpan`): `engine.after(seconds,
  fn, arg?)`, `engine.every(seconds, fn, arg?, firstSeconds?)`, `engine.cancel(handle | fn)`,
  `engine.pending(...)`. `fn` must be an exported function of the same script; it is called as
  `fn(self, arg)` before `update`. Timers are data (component `ScriptTimers`), so they survive
  snapshots (decision 0021).
- **Coroutines** (`StartCoroutine` + `yield return new WaitForSeconds(t)`, Blueprint `Delay`):
  split the coroutine at each wait into exported step functions; each step ends with
  `engine.after(t, nextStep, arg)`. Locals shared by steps go in `self.state` or `arg`;
  `StopCoroutine` is `engine.cancel(nextStep)`. Generators are not supported (a suspended
  generator cannot be saved in a snapshot).
- **Randomness.** `Random.Range`, `FMath::RandRange` -> `engine.rand()` (seeded). Unity's int
  `Range` excludes max; Unreal's includes it. `System.Random`, `FRandomStream`, GUIDs are hazards.
- **Math.** `+ - * /` and `Math.sqrt` are deterministic. `sin`, `cos`, `atan2`, `pow`, `exp`
  differ between machines: precompute constants while porting (`port_convert` turns Euler angles
  into quaternions) or use quaternion math with sqrt only. `Quaternion.Euler`, `FRotator` built
  at run time are hazards.
- **Order.** Never let `Dictionary`/`TMap` iteration order change game state. No threads, tasks
  or async in simulation code.
- **Physics from scripts.** Moving a `Kinematic` body by setting its `Transform` works. Pushing a
  dynamic body (`AddForce`, `AddImpulse`, setting velocity) is a gap: use a kinematic body or the
  `CharacterController`.
- **Spawning** (`Instantiate`, `SpawnActor`): `engine.spawn(components, name?)` or
  `engine.clone(template, overrides?, name?)`. Unity `.prefab` files import into the owned
  library (`library_import`, decision 0040) and `prefab_instantiate` places them in a scene; for
  spawning at run time, place one as a template entity (out of view, no body or script) and
  `engine.clone` it, or keep a components object in the script. Imported prefabs keep each
  MonoBehaviour's serialized fields under `unity.scripts` in `prefab.json`. New entities get a new id (ids are never reused); their script
  starts on the next tick; physics gives them a body in the same tick.
- **Destroying** (`Destroy(obj, t)`, `K2_DestroyActor`): `engine.destroy(entity, seconds?)`;
  the entity is removed at the end of the script stage after its `onDestroy`.
  `Destroy(component)` has no equivalent yet.
- **No hierarchy.** Parenting (`SetParent`, `SetupAttachment`) is a gap: use separate entities,
  `Follow` or `LookAt`, or copy positions in `update`.

## Coordinates and units

Marionette is right-handed, +Y up, -Z forward, +X right, meters (glTF). Use `port_convert`
(`{"engine", "kind", "value"}`; kinds `position`, `direction`, `velocity`, `quaternion`,
`euler`, `scale`, `length`).

| From | Position | Quaternion | Scale |
| --- | --- | --- | --- |
| Unity (left-handed, +Y up, +Z forward, m) | (x, y, -z) | (-x, -y, z, w) | unchanged |
| Unreal (left-handed, +Z up, +X forward, cm) | (y, z, -x) / 100 | (-y, -z, x, w) | (y, z, x) |

Directions convert like positions without the division. The scanner already flips z in Unity
`Vector3` field defaults (not in scale or size fields) and leaves Unreal values unconverted, with
a note. Unreal capsule and box sizes are in centimeters and half sizes; the component notes in
the porting map give each formula.

## Blueprints

Blueprint assets are binary. Select the nodes of a graph in the Unreal editor, copy, and paste
into a text file ending in `.t3d`; `port_scan` reads its events (`ReceiveBeginPlay`,
`ReceiveTick`, overlaps) and function calls (`Delay`, `K2_SetActorLocation`, ...). The skeleton
lists them; rebuild the node chain as code.

## Burst jobs

A Unity job struct (`IJob`, `IJobFor`, `IJobParallelFor`, `IJobChunk`, `IJobEntity`; role
`job` in `port_scan`) becomes a system of the game's own Rust module (decision 0072; recipe
HOWTO.md "Add a game's own Rust module"), not a script. `port_scaffold` writes `jobs.rs` with a
Rust struct per job and per struct its fields use, the original code as comments and an empty
`execute`.

| Unity | Rust module |
| --- | --- |
| `NativeArray<T>`, `NativeList<T>` fields | `Vec<T>` fields of one resource (saved and hashed) |
| `NativeReference<T>` | a plain field of that resource |
| `NativeHashMap` | `BTreeMap` (iteration order must not change state) |
| `Allocator.*`, `Dispose`, `[BurstCompile]`, `[ReadOnly]` | not needed (`&[T]` for read-only) |
| `item0..item6` fields with switch accessors | `[i32; 7]` |
| `Execute` run on the main thread | a `Stage::Logic` system calling it each tick |
| `Schedule` over chunks | `marionette_jobs::Jobs::for_each_chunk_mut` where each chunk writes only its own slice |
| a 20 Hz job inside a 90 Hz frame | the scene's fixed step (`FixedStep`) set to the job's rate |
| `math.min/max/clamp`, integer math | the same; exact everywhere |
| `math.sin/cos/pow` | hazard: precompute |
| instanced draws of job output | a `BulkDrawHooks` render hook (as `examples/factory`) or `JobDraw` |

Derive `Clone` and `Reflect` on the resource (registration requires `Clone`, decision 0073).
Prove it with `perf_sim` on 1 and several threads (same state hash). Save with `Sim::capture`
on the tick thread and `Capture::into_scene` elsewhere: a large factory's state is megabytes
of scene text. Per-entity number loops that fit `JobData` arrays can be job TypeScript
instead (decision 0070); it has no structs and no 64-bit integers.

## Saved data (`[Serializable]` and JsonUtility)

`marionette op port_serde '{"source": "<Assets>", "out": "crates/<game>/src/serial.rs"}'`
(decision 0240) writes serde types for every `[Serializable]` class and struct: the module
reads and writes the JSON `JsonUtility` makes. Pass `roots: ["Name"]` for one type and what it
reaches, `derives: ["Reflect"]` for extra derives, `external: {"Vector3": "crate::Vec3"}` for types
the project already has (hand-written, or its own vector types; never generated, decision 0241),
`dry_run: true` to get the code back.
Read `notes` before using it: `skipped` fields are not saved by Unity either (dictionaries,
nested arrays, non-[Serializable] classes), `unresolved` fields are kept as raw JSON (type not
in the scanned source), `initializer` means a default needed a method call and is zero.
Load with `from_json::<T>(text)` and write with `to_json(&value)`. The module is generated:
change the C# or the tool, never the output. Values the game computes after loading (indexes,
caches) belong in other types, not in the generated ones.

## Not ported

C# `partial` classes are reported once, with the members of every part.

Editor code (Unity `Editor/` folders, `Editor`/`EditorWindow` classes, `OnDrawGizmos`,
`OnValidate`), networking, animation state machines, particle systems and navigation meshes have
no equivalent yet (`port_map` with `status: gap` lists all gaps). Assets go through
`library_import` (decision 0018), not through these tools.

## Extending the tables

The tables are `crates/marionette-port/data/unity.json` and
`crates/marionette-port/data/unreal.json`: sections `bases`, `hooks`, `components`, `types`,
`api`, and `roots` (engine names whose unmapped calls are reported as `unknown`). An `api` name
is a call chain with segments joined by `.` (C++ `::` and `->` are written as `.`); the longest
match wins, and a one-segment name matches only a call, a generic, an attribute, the root of a
longer chain, or a lowercase member. When the script API gains a feature (raycasts, for
example), flip the matching entries from `gap` or `rewrite` to `script`, then run
`cargo run -q -p marionette-cli -- docs` and `cargo test -p marionette-port`.
