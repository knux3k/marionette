# 0240: Rust serde types from C# [Serializable] types (`port_serde`)

Date: 2026-11-13. Status: accepted.

## Context

A game moving from Unity has save files, config and content written by `JsonUtility`. During
the move the old and new builds must read each other's files, so the Rust side needs types
that match those files exactly, for hundreds of classes. Writing them by hand repeats the
C# and drifts from it. The scanner (decisions 0020, 0072) already reads classes and fields.

## Decision

`marionette_port::serde_gen::generate` (operation `port_serde`, guide PORTING.md "Saved data")
turns the `[Serializable]` classes and structs of a scan into one Rust module:

- **Saved fields** follow JsonUtility: public or `[SerializeField]`, not static, const,
  readonly or `[NonSerialized]`; properties never. JSON key = C# field name (Rust name is
  snake case with `#[serde(rename)]` when it differs). Base class fields come first (an
  assumption about JsonUtility's order, not yet checked against real files, see STATE.md).
- **Defaults**: every type is `#[serde(default)]` with a `Default` that carries the C#
  initializers (numbers, strings, booleans, `new List<T>()`, `new T[0]`, collection
  initializers, `new T()`, `Enum.Member`, `new Vector3(..)` and common vector, color and
  quaternion constants). A missing key keeps the constructor value, as `FromJson` does. An
  initializer it cannot translate (a method call) becomes the zero value and is reported.
- **Nested objects** are plain values, never `Option`. Arrays and `List<T>` are `Vec<T>`.
  Dictionaries, nested arrays, nullable and non-[Serializable] classes are not saved by
  JsonUtility and are skipped (reported).
- **Enums** become a newtype over the declared integer type with one associated constant per
  member (values evaluated, including `1 << n`, `A | B`), so an unknown integer survives a
  round trip.
- **Unity value types** (Vector2/3/4, Vector2Int/3Int, Quaternion, Color, Color32, Rect) and
  object references (`{"instanceID"}`) are written into the module when used. A field of a
  type not in the scanned source is kept as `serde_json::Value`, so no data is lost; the
  report lists it.
- Generated `from_json` (ignores a byte order mark) and `to_json` (compact, declaration
  order). Unknown keys are ignored, as in JsonUtility.
- The scanner records class attributes (`RawClass::attributes`) and enum members
  (`source::enum_decls`).

## Alternatives

- **Hand-written types**: drift and thousands of lines. Rejected.
- **`serde_repr` for enums**: a new dependency and loses unknown values. Rejected.
- **Option for objects/strings**: JsonUtility never writes null for them; `Option` would add
  a state the old game cannot represent.

## Consequences

- Known differences, not yet addressed: Unity writes `1` for the float `1.0` (serde_json
  writes `1.0`; both read fine); Unity's float exponent format differs; only literal
  initializers and a table of Unity constants are translated; generic classes,
  `[SerializeReference]` (kept raw), polymorphism and `ISerializationCallbackReceiver` need
  hand work.
- Checked on a synthetic fixture only (`crates/marionette-port/tests`). A check against real
  Unity-written files is still owed by the game that uses it.
