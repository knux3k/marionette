# 0241: `port_serde` on a real project: constants, properties, external types

Date: 2026-11-13. Status: accepted.

## Context

Decision 0240 was checked on a synthetic fixture. Run over the save classes of a large real
game (about 90 [Serializable] files; kept private), it produced 93 types and reported what it
could not map. The report showed what the fixture had not:

- an expression-bodied property (`public bool X => ...;`) was read as a field with a garbage
  type; the scanner treated `=>` as the end of a field;
- initializers that name a const or static field of the class (`= CurrentVersion`,
  `= HomeSystem`), numeric limits (`float.MinValue`), `new a.b.Type()` with a namespace, and
  a few more Unity colors became the zero value, which is wrong for a version number;
- the game already had hand-written Rust for part of the data, and its own vector, quaternion
  and item types; the generator emitted duplicates, and a quaternion default of all zeros
  where Unity has identity;
- a list of an unknown element type became one raw JSON value instead of a list.

## Decision

- The scanner ends a field at `=>` and treats the member as a property, not a field.
- `port_serde` resolves an initializer that is a const or static field of the class (or its
  bases, also written `Class.Name`) to that field's own initializer, and the numeric limits
  (`int.MaxValue`, `float.MinValue`, `Mathf.Infinity`, ...) to Rust's.
- `external` (operation input, `Options::external`): C# type name -> Rust path. The type is
  used as is and never generated, also when it is a Unity value type (`Vector3` ->
  `crate::Vec3`): a Unity constant or constructor for it then becomes a struct literal of the
  external path, so `Quaternion.identity` stays identity. The external type must serialize as
  JsonUtility does (fields `x`, `y`, `z`, `w`).
- A list or array of an unknown element type is `Vec<serde_json::Value>` with a note.

## Alternatives

- **Generate everything, delete hand-written types**: the hand-written factory types are owned
  by another piece of work and carry behaviour. `external` lets generated and hand-written
  types meet without either giving way.
- **Fail on an untranslated initializer**: the zero value plus a note keeps generation going;
  the notes list stays the place to look.

## Consequences

- On the real project, no `initializer` or `unresolved` note remains; the remaining notes are
  fields JsonUtility does not save. Not yet compared with real save files (the game does that).
