# 0040: Unity prefabs become flat Marionette prefabs in the owned library

Date: 2026-11-12. Status: accepted.

## Context

The Unity importer (decision 0018) brought in textures, models and materials and skipped
`.prefab` files. A prefab is what a Unity developer actually places: a tree of game objects with
transforms, meshes, materials, colliders, bodies, lights, cameras and scripts, often nesting
other prefabs and models, or overriding a base prefab (a variant). Porting a Unity game needs
those trees, not only their parts. Marionette has no prefab asset and no parent-child
transforms (`docs/ai/PORTING.md`, "No hierarchy").

## Decision

- **Reader.** `crates/marionette-import/src/unity_yaml.rs` reads Unity's text serialization
  (document headers with class id, file id and `stripped`; block and flow YAML as Unity writes
  it, including flow maps wrapped over lines and sequences at their key's indent). Every scalar
  stays a string, so file ids and values are not rounded. Binary-serialized prefabs are skipped
  with `import.binary_serialized`. No YAML crate: `serde_yaml` is unmaintained and general YAML
  parsers reject or misread Unity's `--- !u!` headers and `stripped` markers.
- **Tree building** (`src/prefab.rs`). Prefab instances (class 1001) are expanded recursively:
  a nested `.prefab` from the same source, or a model from the owned library (this import or an
  earlier one, matched by Unity GUID). Objects of a nested instance get Unity's outer file id
  `(instance ^ inner) & 0x7fff...`, and `stripped` placeholders are also resolved through
  `m_CorrespondingSourceObject`, so overrides, added components and added children land on the
  right object. Overrides (`m_Modifications`) are applied to the raw values by property path
  (including `Array.data[n]` and `Array.size`) before mapping; `m_RemovedComponents` and
  `m_RemovedGameObjects` are honoured. A variant is a file whose root is an instance. Cycles and
  nesting deeper than 16 levels become placeholder entities with a note.
- **Models are kept whole.** A converted glTF is one `Model`. Overrides on parts inside a model
  are listed in the prefab notes, not applied. For a model instance the root is the target of its
  `m_LocalPosition` overrides (Unity always writes them for an instance root).
- **Flattening.** Entities get transforms relative to the prefab root (the root's own position
  and rotation are dropped; they are the placement point), converted to Marionette axes
  (`(x, y, z) -> (x, y, -z)`, quaternion `(-x, -y, z, w)`). Each entity records its hierarchy
  path as its name, its parent's key and `active`. A converted glTF model gets an extra half turn
  about +Y, because Unity mirrors imported models on X. A non-uniform parent scale over a rotated
  child cannot be represented (no shear) and is noted.
- **Component mapping.** MeshFilter/MeshRenderer/SkinnedMeshRenderer to `Model` (library model,
  or built-in Cube/Sphere/Plane primitives, the plane scaled 10x); the first material of a
  primitive to `Material` (factors from the library material); Box/Sphere/Y-Capsule/Mesh
  colliders to `Collider` (Unity default friction 0.6, bounciness 0; trigger to `sensor`);
  Rigidbody to `RigidBody` with Unity's mass turned into a density by the collider's volume;
  directional Light to `DirectionalLight`; Camera to `Camera`; AudioListener to `AudioListener`.
  Offset colliders and second colliders on a fixed object become their own entities; on a body
  they are listed as unmapped, as are colliders on children of a body (no compound bodies).
  MonoBehaviours are kept as data (`unity.scripts`: script path, class name, serialized fields)
  for the porting tools; point, spot and area lights, audio sources, animators and anything else
  are listed under `unity.unmapped`. Nothing is dropped silently.
- **Storage and licenses.** A prefab is an owned-library asset of kind `prefab` (files
  `prefab.json`, format `marionette.prefab/1`, and `source.prefab`) under the same rules as every
  import: it needs a covering declaration; a nested prefab is expanded only if it passes the gate
  under the same declaration (rule 12a); models and materials are referenced by library id
  (`library:<id>/<file>`), never copied into the prefab, and listed in the record's `uses`.
  Prefabs are processed after textures, models and materials.
- **Placement.** Operation `prefab_instantiate` adds one entity per active object to a scene file
  in one validated edit, composed with `at`, with fresh ids, and copies the library files the
  prefab uses (never `source.*`) to `<scene dir>/library/<id>/` so the scene stays portable.

## Alternatives considered

- Converting a prefab into one glTF scene: keeps the hierarchy for drawing but loses colliders,
  bodies, lights, cameras and scripts, which are most of a prefab's meaning.
- Adding parent-child transforms to the engine first: a core and determinism change touching
  physics write-back and every system that reads `Transform`; not needed to make prefabs
  usable, and the flat form converts directly once hierarchy exists (entities already carry
  their parent's key).
- Keeping instance placement in a script (`engine.spawn` from a JSON object): possible later;
  a scene edit is what agents and the porting recipe need first.

## Consequences

- Unity prefabs, nested prefabs and variants import with their license; agents place them with
  one operation. Proofs: `crates/marionette-import/tests/prefab.rs`,
  `crates/marionette-cli/tests/prefab_e2e.rs`, unit tests in `src/unity_yaml.rs` and
  `src/prefab.rs`.
- Placed entities do not follow their parent when it moves at run time.
- Not covered: sub-mesh selection inside a model (the whole model is drawn), overrides on model
  parts, skinned rendering (bind pose until skinning lands), audio, point/spot lights, physic
  materials, capsules along X or Z, compound bodies, Unity scenes (`.unity`, same reader, later
  work), checking the result against the Unity editor on real projects.
