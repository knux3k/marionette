# Assets and licenses (normative)

MUST, MUST NOT and MAY are used as in RFC 2119. These rules bind every agent and every tool in
the engine.

## Search order

1. An agent that needs an asset MUST search, in this order, and stop at the first acceptable
   result: (1) the project's own files; (2) the user's owned library; (3) the CC0 pool; (4) free
   CC0 libraries online (Poly Haven, ambientCG, Kenney, Quaternius); (5) generation.
2. The owned library is a per-user folder shared across that user's projects. It MUST NOT be
   inside the engine repository and MUST NOT be synced to the public pool.
3. Generation uses Qwen for images only and Tripo for 3D models. Rigging: Tripo auto-rig first
   for humanoids and common animals; agent-scripted Blender otherwise. Every rig MUST be checked
   numerically by the engine before use.

## The CC0 pool

4. The public pool (in the assets repository) MUST accept CC0 assets only, each with its source
   recorded.
5. The pool MUST refuse every imported owned asset and every asset whose license is not CC0.

## Importing owned Unity and Unreal assets

6. Every imported asset MUST carry a license record.
7. Engine-locked content MUST be refused with no override: content published by Unity or Epic,
   engine sample or starter content, or any license that limits use to that engine.
8. An asset whose license cannot be determined MUST be refused until a PERSON writes a
   declaration file stating: the store, the product name or link, the license type, and whether
   use in other engines is allowed (yes/no).
9. An AI agent MUST NOT write that declaration file.
10. An agent MAY read license text a person pastes in, and MUST refuse the asset if the text
    limits use to one engine.
11. The declaration MUST be recorded on each asset imported from the pack it covers.
12. The import report MUST list every refused asset with the reason.
12a. A file another asset depends on (a texture an FBX names, a buffer or image a glTF names)
    MUST NOT be bundled into that asset unless it passes these rules under the same declaration
    as the asset; otherwise it is left out and the reason is in the asset's notes.
13. Imported assets are private to the user and MUST be blocked from the pool.

## Declaration file format

A person writes one JSON file named `<anything>.license.json` per pack, beside the source as
`<source name>.license.json` or anywhere and passed to `library_import` in `declarations`:

```json
{
  "format": "marionette.license_declaration/1",
  "store": "where it was obtained",
  "product": "product name or link",
  "license_type": "license name as the store gives it",
  "other_engines_allowed": "yes",
  "publisher": "optional",
  "covers": ["Assets/SomePack/"],
  "license_text": "optional pasted license text"
}
```

`covers` is optional (empty means the whole source); the most specific matching prefix wins.
`other_engines_allowed` is `yes` or `no`. Tools refuse to write any `*.license.json` file
(`import.declaration_write_refused`).

## Where the rules are enforced

`crates/marionette-import` (decision 0018): `src/license.rs` (declarations, engine content,
license text), `src/import.rs` (refusal order, import report), `src/library.rs` (location
rules, records with `private: true` and `pool: "blocked"`), `src/pool.rs` (the owned-asset
gate the pool MUST call). Operations: `library_import`, `library_list`, `library_get`,
`library_info`, `license_check_text`, `pool_admission_check`. Proofs:
`crates/marionette-import/tests/import.rs`, `crates/marionette-cli/tests/library_e2e.rs`.

## Spending

14. Spending API credits MUST have the user's approval and a spend cap.
15. A generated image MUST be approved before credit is spent on a 3D model made from it.

## Assets in the engine repository

16. Assets committed to the engine repository MUST be CC0 or made for the project, each listed
    with its source in `examples/assets/LICENSE.md`. Built-in images, sounds and the font are
    generated in code (`crates/marionette-assets/src/lib.rs`,
    `crates/marionette-assets/src/font.rs`, `crates/marionette-audio/src/lib.rs`).
