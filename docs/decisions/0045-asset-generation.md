# 0045: Asset generation from recipes, cached by request, gated by a person

Date: 2026-11-12. Status: accepted.

## Decision

- **Last step of the lookup order.** Generation runs only after the project's files, the owned
  library, the CC0 pool and online CC0 libraries (ASSETS-AND-LICENSES.md rule 1). It owns no
  search; `generate_plan` lists the earlier steps in `before_generating`, and the pool's search
  (when built) points at `generate_plan` and `generate_run` when it finds nothing.
- **Crate.** `crates/marionette-generate`: recipes, cache, provenance, approval, post-processing.
  It depends on core, assets, providers, glam and gltf. Operations in
  `crates/marionette-agent/src/generate.rs`: `generate_recipe`, `generate_plan`, `generate_run`,
  `generate_list`. CLI: `marionette generate plan|run|list|approve`.
- **Recipes.** One `<name>.recipe.json` (format `marionette.recipe/1`) per asset, in
  `assets/generated/` by default; the output (`<name>.png` or `<name>.glb`) and its provenance
  record sit beside it. Three outputs: `image` (PNG as generated), `texture` (PNG resized to
  powers of two, optionally tileable), `model` (GLB from a prompt and/or `from` an image recipe).
  A recipe may pin a provider; otherwise `providers.json` decides, and once an asset is built
  its recorded provider is kept while the recipe is unchanged (switching providers never
  invalidates finished assets).
- **Cache keyed by the request.** `.marionette/generate-cache/<key>.<png|glb>` plus a record.
  The key is SHA-256 of canonical JSON of provider, model, kind, prompt, size and the reference
  image's SHA-256. Post-processing options are not in the key, so changing a texture size or a
  model height reuses the paid output. A damaged entry is an error, never a silent re-buy.
  A paid output that fails post-processing stays cached (it was paid for).
- **Provenance.** `<name>.provenance.json` (`marionette.generated_asset/1`): recipe and its hash,
  provider, model, `provider_verified`, prompt, size, reference image (name, hash, approved),
  cache key, raw and final hashes, original cost, post-processing report, license status
  `generated` with a terms note, and `pool: "blocked"`. No time stamps: rebuilding gives
  identical files.
- **The pool.** Generated assets are never offered to the CC0 pool: who owns an output depends
  on each provider's terms and the user's plan, so it is not CC0 by default.
  `marionette_generate::provenance::pool_status(file, dirs)` reports a generated asset (by its
  record, or a copy by hash) as not admissible; the pool's admission check calls it.
- **Approval (rule 15).** A model made from an image with a paid 3D provider needs
  `<image>.approval.json` binding the image's SHA-256. Only `marionette generate approve <name>`
  writes it, after a person types the name at an interactive terminal. No operation writes it
  and every write helper refuses the name (`generate.approval_write_refused`). Changing the image
  voids the approval. Like the cap, this guards against an agent approving its own spending by
  accident; it is not a boundary against a program that writes files directly.
- **Spending.** Checked in this order before anything is sent: the whole run's estimates against
  the caps (a run never spends half and then stops), approval, the cap for this call, the key,
  the network. A person may give one provider its own cap: `MARIONETTE_SPEND_CAP_<NAME>_USD`
  or `provider_caps_usd` in the user config file, a number or `unlimited`. A provider with its
  own cap is checked against its own spending only; others share the total cap over their own
  spending. The project owner set Qwen to unlimited and Tripo to 10 US dollars for the proof
  scenes.
- **Keys.** Environment first, then a person's files outside every project: a keys file
  (`MARIONETTE_KEYS_FILE` or `keys_file` in the user config; `NAME=value` lines or JSON) or one
  file per key (`key_files` in the user config; the key is found by `NAME=`, by the provider's
  key prefix, or as the file's only line). A file inside the project folder is refused and
  not read. Values are never printed or returned.
- **Network.** `http::Network` (ureq with rustls) behind the cargo feature `net`, off by
  default, so a default build still cannot reach a paid API. Build with
  `cargo build --release -p marionette-cli --features net` on the machine that holds the keys.
  Tested against a server on the same machine only.
- **Qwen regions.** A Model Studio key works only in its own region. The plug-in tries the
  international, China and US addresses in that order and moves on after a 401 (a refused key
  is not charged); it polls the region that accepted the job. Found with the first live check.
- **Tripo with a local image.** Uploaded with `POST /v2/openapi/upload` (multipart, field
  `file`), then `image_to_model` with `file_token`.
- **Post-processing, deterministic.** Images: decode to RGBA8, exact integer box resampling,
  seam blending of four half-size wrap offsets with separable weights (no seam at the borders or the center; the first live texture showed a center seam with one offset), canonical PNG re-encode (drops metadata). Models:
  valid glTF 2.0, self-contained, bounds through the node tree from accessor min/max, one root
  node added that grounds (lowest point at y = 0), centers x and z and scales to `height_m`;
  mesh data untouched; triangle budget reported, not enforced; the engine's own loader must
  accept the result.

## Why

- The brief asks that a generated asset be a recipe plus a cached output keyed by content hash,
  that spending have the user's approval and a cap, and that an image be approved before 3D
  credit is spent. Keying by request rather than by output is what stops paying twice.
- Plain files (recipes, records, approvals) work with any agent and survive in git; the cache
  holds the paid bytes so assets rebuild offline.
- Per-provider caps: the project owner asked for unlimited Qwen and a 10 dollar Tripo cap.
- Key files: the project owner keeps keys in personal files and asked that they never be read
  into chat or committed; the engine reads them only at call time.

## Rejected

- Post-processing options in the cache key: every tweak would buy the asset again.
- Time stamps or run ids in provenance: rebuilds would never be byte-identical.
- An operation that approves images: an agent could approve its own spending.
- Offering generated assets to the pool with a person's dedication: provider terms make CC0
  status uncertain; revisit only with a reviewed terms check per provider.
- Network on by default: one configuration step from spending.

## Not done yet

- Live verification of the Qwen and Tripo formats (`verified: false` until the first approved
  live call; sample answers in `crates/marionette-generate/tests/recorded/` are hand-written).
- Tripo auto-rigging and the numeric rig checks; agent-scripted Blender rigging; decimation;
  texture maps beyond base color (normal, roughness); sprite sheets.
- A per-user cache shared across projects; per-period spending.
