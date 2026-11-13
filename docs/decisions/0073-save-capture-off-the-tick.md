# 0073: Saving copies state on the main thread and writes it elsewhere (`Sim::capture`)

Date: 2026-11-13. Status: accepted.

## Context

The large VR factory game the engine targets allows 2 ms of main-thread time for a save. In
the port of its factory tick (decision 0072) the tick itself took 0.12 ms at 4x, but
`Sim::snapshot` plus scene text took about 50 ms at 4x (4.7 MB of JSON) and 120 ms at 8x. The
engine's own factory benchmark at 4x took 72 ms. Measured split (4x, release): reflection to
JSON text 18 ms, parsing that text back into JSON values 14 ms, pretty printing 10 ms. The
canonical text cannot change without changing every state hash, so making the text faster has
limits; the main thread does not need to wait for it.

## Decision

- `Sim::capture()` (`scene::capture`) copies every registered resource and component with the
  type's own `Clone`, keyed and ordered as in a scene. `Capture` is `Send`;
  `Capture::into_scene(&Registry)` builds the same `Scene` that `Sim::snapshot` builds, on any
  thread, while the simulation goes on.
- `Registry::register_component` and `register_resource` now require `Clone`; `TypeEntry`
  keeps a typed copy function next to `get`. Every registered type in the engine and examples
  already derived `Clone`.
- Budget: `save capture 4x` <= 2 ms (`perf_budgets::save_capture_budget`). Exactness:
  `perf_proofs::capture_saved_on_another_thread_equals_snapshot` (factory and swarm scenes, the
  scene built on another thread equals the snapshot byte for byte).

## Alternatives

- **Reflection's `reflect_clone`** (no new bound): field by field through reflection; 12 ms at
  4x for the ported state and 7.4 ms for the engine factory, unless every type adds
  `#[reflect(Clone)]`. Rejected for the typed copy.
- **Compact text for long number arrays** (base64 or one line per array): smaller files and a
  faster write, but it changes the canonical text and so every state hash, including the drift
  hash. Left as a separate decision if save size matters.
- **Skip the JSON round trip in `Registry::to_json`**: `serde_json::to_value` writes `f32` as
  its `f64` widening (0.1 becomes 0.10000000149011612), which changes the text. Rejected.

## Consequences

- Focus 3: main-thread save cost at 4x went from 72 ms to 0.75 ms (engine factory) and from
  about 50 ms to 0.6 ms (ported game state); the text still takes 50 to 120 ms on another
  thread. Loading the 4x saved state took 0.36 s (budget 30 s).
- A game type registered without `Clone` no longer compiles; derive `Clone`.
- `state_hash` still builds the full text (about 70 ms at 4x); call it when needed, not every
  tick.
