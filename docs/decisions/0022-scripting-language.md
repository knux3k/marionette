# 0022: Game scripts are TypeScript on QuickJS-ng (spike: TypeScript against Luau)

Date: 2026-11-12. Status: accepted.

## Decision

Game logic is written in **TypeScript**. Types are removed by **oxc** (no type checking at run
time) and the JavaScript runs in **QuickJS-ng** (through `rquickjs`), an interpreter with no
JIT. Crate: `crates/marionette-script`. Luau (through `mlua`) was measured the same way and
not chosen.

Binding (details in the crate docs and `docs/ai/reference/marionette.d.ts`):

- An entity with `Script { path }` runs that `.ts` file (relative to the scene). The file is an
  ES module exporting `update(self)` and optionally `start`, `onCollisionEnter`,
  `onCollisionExit`, `onTriggerEnter`, `onTriggerExit`. Names follow Unity/Unreal habits so
  ported code keeps its shape (the porting thread maps `MonoBehaviour`/`AActor` methods to them).
- Scripts run in a new stage, `Stage::Script`, between `Input` and `Logic`, entity-id order.
  Contacts reported by physics on tick N reach the hooks on tick N+1 (physics runs later in
  the tick). A contact where either collider is a sensor goes to the trigger hooks.
- Scripts read and write any registered component by its scene name (`self.get("Velocity")`),
  validated by the same registry as scene files. Per-entity state is `self.state`, saved as the
  `ScriptState` component (in the state hash and snapshots). The script host is rebuilt from
  the files on load, reload and restore, so module-level variables are not state.
- `engine.*`: tick, dt, seeded `rand()`, input actions, `emit` (events), `log`, `entity(id)`,
  `find(name)`, `query(component)`, `resource(name)`.
- Hot reload: operation `script_reload` (live session); typings: operation `script_types` and
  `marionette docs` (writes `docs/ai/reference/marionette.d.ts`), generated from the schema.
- Errors as data: `script.read`, `script.syntax` (file, line, column), `script.load`,
  `script.no_update`, `script.engine` at load; a throw during a tick becomes a `script_error`
  event and the tick continues. Inside scripts, bad component values throw `script.bad_value`,
  unknown names `script.unknown_component` / `script.unknown_resource`.

## Why (the spike)

Harness: `spikes/script-spike` (not a workspace member, so CI does not build it). The same
game script in both languages: per entity, read position and velocity through host calls,
gravity, bounce with a seeded random kick, write both back. Release build, one cloud Linux
container (4 vCPUs), medians of 50 runs where repeated. Rerun:
`cargo run --release --features luau -- all`, `--features ts -- all`,
`--features luau,ts -- eval`, and `-- order` twice (in `spikes/script-spike`).

| Measure | Luau (mlua 0.12, Luau 0.740) | TypeScript (oxc 0.158 + QuickJS-ng, rquickjs 0.14) |
| --- | --- | --- |
| Startup: new VM, host API, run script | 272 µs | 374 µs |
| Compile source (Luau bytecode / TS type strip) | 62 µs | 36 µs |
| Hot reload of a changed script | 76 µs | 115 µs (strip included) |
| Empty call Rust to script | 71 ns | 51 ns |
| Entity update (4 host calls + math) | 480 ns | 1,924 ns idiomatic (`[x, y, z]` destructuring); 1,202 ns with index access |
| Pure script math, 1M iterations | 32 ms | 60 ms |
| VM heap after load | 472 KB | 102 KB |
| Clean release build of the binding | 48 s | 66 s |
| Binary size over a hello-world (441 KB) | +2.4 MB | +5.1 MB |
| Same hash twice, 1,000 entities x 1,000 ticks | yes | yes (and the same hash as Luau: identical IEEE results) |
| Iteration order of a table/`Map` keyed by objects, two processes | **different each run** (address hashing) | insertion order, identical |
| AI-written first attempts, 10 tasks / 33 cases (`eval/`) | 33/33 | 33/33 |

Reading the numbers:

1. **Determinism (deciding).** Luau iterates tables with object keys in address order, which
   changed between two runs of the same program. A script that loops over such a table changes
   the simulation run to run, and nothing in the language stops it. JavaScript fixes property,
   `Map` and `Set` order by specification. Both needed the same sandboxing (clock, unseeded
   random, transcendental math from the C library); see Consequences.
2. **Speed is enough either way.** Luau is 2 to 4 times faster, but even the idiomatic
   TypeScript path runs about 500 scripted entities in 1 ms of an 11.1 ms frame; hot paths move
   to Rust (design principle 2). Both reload in well under 1 ms, far inside the 1 s target.
3. **Coding AIs.** Both languages scored 33/33 on first attempts written by a coding AI, so the
   test did not separate them. Two things still favor TypeScript: far more TypeScript than Luau
   exists in training data and tooling (`tsc --strict` checks scripts against the generated
   `marionette.d.ts` with no engine-specific tools), and the Luau attempt needed care on
   1-based indexing and on `table.sort` not being stable, where TypeScript's defaults were
   already right.
4. **Porting from Unity and Unreal (owner requirement, 2026-11-12).** C# and C++ gameplay code
   (classes, typed fields, enums, interfaces, generics, camelCase methods) maps nearly
   one-to-one onto TypeScript; Luau needs idiom rewrites (metatables, 1-based arrays, no enums).
5. **Embedding cost** favors Luau (smaller, faster build). Both need only a C/C++ compiler,
   already required by other dependencies. Accepted.

## Consequences

- Determinism rules for scripts, enforced by the host: `Date`, `performance`, `Math.random`,
  `WeakRef`, `FinalizationRegistry` are removed; `Math.sin`/`cos`/`tan`/`asin`/`acos`/`atan`/
  `atan2`/`sinh`/`cosh`/`tanh`/`asinh`/`acosh`/`atanh`/`exp`/`expm1`/`log`/`log1p`/`log2`/
  `log10`/`cbrt`/`pow`/`hypot` come from the pure-Rust `libm` crate (bit-identical across
  platforms); `a ** b` is rewritten to `Math.pow(a, b)` at strip time. Pending promise jobs are
  drained after every hook call.
- Unsafe code: the host functions reach the world through a pointer set only while
  `run_scripts` calls into JavaScript (`Bridge` in `crates/marionette-script/src/lib.rs`);
  `ScriptHost` is marked Send + Sync because rquickjs' `parallel` feature locks the runtime on
  every `Context::with`.
- Component values cross the boundary as JSON text through the registry (simple, validated,
  slower than direct field access). Measure before optimizing.

## Rejected

- **Luau**: faster and smaller, but object-key iteration order is not deterministic, and it
  is a poorer fit for coding AIs and for porting C#/C++.
- **V8 or another JIT engine (`deno_core`)**: large, slow to build, JIT tiers make timing and
  memory behavior harder to control; nothing in the numbers asks for that speed.
- **Boa (pure-Rust JS)**: avoids C, but much slower interpreter than QuickJS-ng.
- **Type checking inside the engine** (bundling `tsc`): too heavy; agents run `tsc` when they
  want checking, using the generated declarations.

## Not done yet

- Spawning and despawning entities from scripts (with an `onDestroy` hook); timers or
  generator-based waits (coroutines, `Invoke`, `SetTimer`); `import` between script files
  (no module loader yet: one file per script).
- Window mode does not watch script files; hot reload is the `script_reload` operation.
- A speed pass on the component bridge (direct field access for common types).
