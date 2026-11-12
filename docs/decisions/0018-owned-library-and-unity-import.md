# 0018: Owned library, license declarations and the Unity importer

Date: 2026-11-12. Status: accepted.

## Decision

- **Crate.** A new crate `crates/marionette-import` holds the owned library, license rules,
  the Unity importer and the pool admission gate. `marionette-assets` stays the CPU loader with
  no file-system policy; the pool and search (workstream W3) can live beside it without
  touching the importer. Operations are in `crates/marionette-agent/src/library.rs`.
- **Library location.** Resolved from the operation input `library`, then the environment
  variable `MARIONETTE_LIBRARY`, then the user config file `marionette/config.json` in the
  user's config folder (`APPDATA` on Windows, `XDG_CONFIG_HOME` or `~/.config` elsewhere) with
  key `owned_library`. The path must be absolute, outside the engine repository (any ancestor
  whose workspace lists `marionette-core`) and outside the current project. A non-empty folder
  without `library.json` is refused, so a mistyped path cannot scatter files.
- **Layout.** `library.json` marker; `assets/<id>/record.json` plus the asset's files. The id
  is `<slug of the name>-<first 12 hex of the original file's SHA-256>`, so importing the same
  file twice is a no-op (`already_in_library`). Files are written first and the record last,
  so a record always means a complete asset. Records hold names inside the source only, never
  machine paths.
- **Declarations.** A person writes `*.license.json` (format
  `marionette.license_declaration/1`: store, product, license type, `other_engines_allowed`
  yes/no, optional publisher, `covers` path prefixes, pasted `license_text`). The importer uses
  `<source>.license.json` beside the source plus any passed in `declarations`. The most specific
  `covers` prefix wins. Its file name and SHA-256 are copied into every asset's license record.
  Enforcement of "an AI may not write it": no operation writes declarations, and both the
  agent's file writer (`write_atomic`) and the library writer refuse any `*.license.json` path
  with `import.declaration_write_refused`.
- **Refusal order** per file: engine content by path (Unity sources: `Packages/`,
  `Assets/Samples/`, Standard Assets, Starter Assets, TutorialInfo, TextMesh Pro, Unity
  Technologies folders, `com.unity.*`), engine publisher by Asset Store cache folder, no covering
  declaration (`import.license_unknown`), declared publisher Unity or Epic, declaration says no,
  pasted text limits use to one engine. None of these has an override. Unsupported types are
  `skipped`, not `refused`, and are listed too.
- **License text check.** A fixed phrase list (only/exclusively/solely ... with/in/for an engine,
  "for Unity only", "Unreal Engine-only", "restricted to", "not be used in any other engine").
  It can only refuse; finding nothing never replaces a yes declaration.
- **.unitypackage.** Read with a small bounded tar reader of our own (no `tar` dependency) over
  `flate2`. Entries are staged in a temporary folder under names built only from validated
  32-hex GUIDs, so archive names cannot choose where bytes go; package paths are checked (no
  absolute, `..`, empty parts; must be under `Assets/` or `Packages/`). Limits: 2 GiB per entry,
  16 GiB total.
- **What maps.** Textures: PNG as is; JPEG and TGA converted to PNG (original kept); other
  formats kept but not engine-ready. Texture `.meta` usage (normal map, linear) becomes notes.
  Models: GLB as is; glTF with its external files rewritten to `deps/`; both validated with the
  engine's loader. FBX, OBJ, blend and other formats are kept as `source.<ext>` with
  `engine_ready: false`: a converter is not built yet (FBX to glTF is a later step; candidates
  are a headless Blender recipe or a pure-Rust FBX reader). Materials: text-serialized `.mat`
  (Standard, URP Lit/Unlit, HDRP Lit, or any shader by common property names) become
  `material.json` (glTF metallic-roughness fields; colors converted from sRGB to linear; texture
  references are library ids, resolved within the import and then by Unity GUID across earlier
  imports). Unity's metallic/smoothness (or HDRP mask) texture is converted into glTF channel
  layout as `metallic_roughness.png` in the material's folder. Binary-serialized materials are
  skipped with a reason.
- **Pool gate.** `marionette_import::pool::check` (operation `pool_admission_check`) refuses a
  candidate inside an owned library or asset folder, carrying an owned record or a declaration,
  or with any file whose SHA-256 equals an owned file (original or converted). The CC0 pool
  (W3) MUST call it; it adds its own CC0-and-source rule on top.

## Alternatives considered

- Keeping it in `marionette-assets`: mixes policy and file writes into the CPU loader and puts
  W2 and W3 in the same files.
- The `tar` crate: fine, but extracting by archive names needs care; a 200-line reader that
  never touches the file system was simpler to make safe.
- Recording material data only as notes: less useful than a glTF-shaped file that the renderer
  can adopt directly.

## Consequences

- Owned assets are importable today with their licenses recorded; FBX models need a converter
  before the renderer can draw them.
- Prefabs, scenes, animations and audio from Unity are skipped and listed; they are later work.
