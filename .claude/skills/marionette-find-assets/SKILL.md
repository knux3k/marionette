---
name: marionette-find-assets
description: Find a model, texture, sound or prefab for a Marionette game in the required order (project files, owned library, CC0 pool, online CC0 libraries, generation last) and bring it into the project with its license intact. Use when a scene needs art or audio, when importing a Unity package or prefab the user owns, or when adding an asset to the CC0 pool.
---

# Find and import an asset

The order is a rule, not a suggestion: stop at the first step that has an acceptable result.

1. Project files. 2. The user's owned library. 3. The CC0 pool. 4. Online CC0 libraries.
5. Generation (skill `marionette-generate-assets`).

## Search

`marionette op asset_search '{"query": "wooden crate", "kind": "model"}' --json` walks the
steps in order and stops at the first with results (`found_in`). Add `"online": true` to
include step 4. `kind` is texture, model, material, sound or other.

## Use what it found

| Found in | Next |
| --- | --- |
| project | use the path in a `Model`, `Sprite` or `AudioSource` component |
| owned library | `library_get` for the record; prefabs go in with `prefab_instantiate`, which copies the files beside the scene |
| CC0 pool | `pool_fetch` with the id copies it into the project and returns the main file path |
| online | `pool_ingest` downloads it into the pool with source and proof, then `pool_fetch` |

## Importing what the user owns

`library_import` takes a Unity package, a Unity or Unreal project folder, or plain files.
Run it with `"dry_run": true` first and read `refused` and `skipped`.

- Every pack needs a license declaration file written by a person. Never write one, and
  never edit one. If it is missing, tell the user which pack needs it and stop on that pack.
- Content published by an engine vendor, sample content, and any license that limits use to
  one engine is refused with no override. `license_check_text` checks pasted license text;
  finding nothing never grants permission.
- Owned assets are private: they never go to the CC0 pool (`pool_admission_check` enforces it).

## Adding to the CC0 pool

Only CC0 assets with proof: made for the project (`pool_add` with origin `made`) or from a
known CC0 library. Run `pool_add` with `"dry_run": true` first, then `pool_verify`.
Generated assets are refused unless a person wrote a CC0 dedication; never write one.
