# 0020: Porting Unity and Unreal code

Date: 2026-11-12. Status: accepted.

## Context

The project owner wants Unity (C# MonoBehaviour) and Unreal (C++ and Blueprint) gameplay code to
convert into Marionette as easily as possible. Asset import already exists (decision 0018);
this record is about code. Scripts are TypeScript (decision 0022): an ES module named by a
`Script { path }` component, hooks `start`, `update`, `onCollisionEnter`, `onCollisionExit`
taking `self: Entity`, and `engine.*` globals.

## Decision

- **Porting is agent-assisted, not a transpiler.** A crate `crates/marionette-port` scans the
  source, maps every engine concept to its Marionette equivalent with a status, and writes
  TypeScript skeletons that keep the original code as annotated comments. The agent writes the
  final code. Alternatives rejected: a C#/C++ to TypeScript transpiler (most lines that matter
  are engine API calls whose meaning changes: frame time to fixed ticks, components as objects
  to components as data, coroutines to tick counters; a transpiler would produce code that
  compiles and behaves wrongly, and needs a full C# and C++ front end); running C# or C++ inside
  the engine (breaks determinism, platform-specific, huge); familiar-name shims in the script
  API such as a fake `transform.position` object (hides that components are copies, so writes
  would silently not apply, and doubles the API surface to keep deterministic).
- **Concept tables are data.** `crates/marionette-port/data/unity.json` and `unreal.json` hold
  base classes, hooks, components, types and API call patterns with statuses `ready`, `script`,
  `rewrite`, `gap`, `hazard` (scans add `unknown`). The same tables feed the scanner, the
  `port_map` operation and the generated `docs/ai/reference/porting-map.md`, so they cannot drift.
  Gaps are explicit: the scan's `top_gaps` says which engine features ported games need most.
- **Scanner.** Our own lexer for C-family source (comments, strings, C# verbatim and
  interpolated strings, C++ raw strings, preprocessor lines) and a structural reader for classes,
  fields (attributes and `UPROPERTY` specifiers, literal defaults), methods, C++ out-of-class
  definitions (merged with the header class by name) and call chains (`a.b().c`, `->`, `::`,
  `?.`, generics). No parser dependency: tree-sitter grammars or Roslyn would add native or .NET
  build requirements for a tool that only needs structure. Classes get roles from their base
  class, followed through scanned classes (`Enemy : Character : MonoBehaviour`). Delegate,
  input, listener, timer and coroutine bindings (`AddDynamic(this, &AFoo::OnOverlap)`,
  `BindAxis`, `Invoke("Respawn")`, `StartCoroutine(Blink())`) mark their handler methods, so
  overlap handlers land in `onCollisionEnter` and input handlers are called from `update`.
  Blueprints are read from graph text copied out of the editor (`.t3d`), since `.uasset` is
  binary and version-specific.
- **Skeletons** (`port_scaffold`): one `.ts` per behaviour, actor, actor component or
  Blueprint; Unity Awake before Start and FixedUpdate before Update inside the merged hook;
  fields as `self.state.x = self.state.x ?? default`; non-hook methods as functions taking
  `self`; `PORT-NOTES.md` with scripts, components, actions, hazards and gaps. Skeletons compile
  under `tsc --strict` (tested against a stand-in for the script typings) and never overwrite
  files without `overwrite: true`.
- **Coordinates** (`port_convert`): Unity (x, y, z) -> (x, y, -z) and quaternion
  (-x, -y, z, w); Unreal (x, y, z) cm -> (y, z, -x) / 100 m, quaternion (-y, -z, x, w), scale
  (y, z, x); Euler angles follow each engine's own rotation order (Unity Z, X, Y; Unreal
  FRotator to FQuat). Conversion runs at port time, so its trigonometry does not affect
  determinism. Unity `Vector3` field defaults are flipped automatically (not scale or size
  fields); Unreal defaults are left as written, with a note, because positions and directions
  convert differently there.
- **Influence on the scripting choice.** Porting ease was sent to the scripting spike: C# and
  C++ classes, static types, enums, generics and camelCase methods map onto TypeScript far more
  directly than onto Luau. TypeScript was chosen there (decision 0022) also for determinism.

## Consequences

- Entries marked `script` depend on the script runtime (decision 0022). When it gains spawning,
  despawn and `onDestroy`, trigger hooks, timers or raycasts, flip the matching `gap`/`rewrite`
  entries and regenerate the reference.
- The scanner reads structure, not semantics: macros that hide declarations, partial classes
  split oddly, or code generators can be missed. Unknown engine calls are reported as `unknown`
  rather than guessed.
- Tests: `crates/marionette-port/tests/port.rs` (fixtures under
  `crates/marionette-port/tests/fixtures/`), `crates/marionette-cli/tests/port_e2e.rs`, unit
  tests in `src/lex.rs`, `src/map.rs`, `src/convert.rs`.
