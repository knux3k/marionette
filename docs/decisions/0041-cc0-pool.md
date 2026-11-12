# 0041: The CC0 pool, its admission rules and the asset lookup order

Date: 2026-11-12. Status: accepted.

## Context

`docs/ai/ASSETS-AND-LICENSES.md` requires a public pool of CC0 assets with each asset's source
recorded, refusal of owned and non-CC0 assets, and a fixed lookup order (project files, owned
library, pool, online CC0 libraries, generation). The pool is meant to live in a separate public
assets repository that does not exist yet; the tooling and a seed set live in the engine
repository until then. The online libraries named by the design are Poly Haven, ambientCG,
Kenney and Quaternius.

## Decision

- **Crate.** `crates/marionette-pool` (not `marionette-assets`: the pool must call
  `marionette_import::pool::check`, and `marionette-import` already depends on
  `marionette-assets`). Operations in `crates/marionette-agent/src/pool.rs`.
- **Layout.** A pool is a folder: `pool.json` (format `marionette.cc0_pool/1`), one
  `assets/<id>/` per asset with its files and `asset.json` (format `marionette.pool_asset/1`:
  kind, name, tags, files with SHA-256, main file, engine readiness, source, license), and a
  generated `index.json`. The engine repository's `pool/` is the seed; the assets repository
  will hold the same layout. Location: operation input `pool`, then `MARIONETTE_POOL`, then
  `cc0_pool` in the user config file. A pool inside an owned library is refused.
- **Provenance and proof.** `source.origin` is `made` (made for the project: `method` required,
  `generator` when code in the engine repository makes it; dedicated to the public domain under
  CC0 by the Marionette Contributors) or `online` (`library`, `page`, `source_id`, `author`,
  `retrieved` date, `downloads`). `license.spdx` is always `CC0-1.0`; `license.proof` lists the
  evidence: `made_for_project`, `library_policy` (the library's license page and the sentence it
  was checked against), `license_file` (with its SHA-256), `download_integrity` (published size
  and MD5 matched), `person_dedication`.
- **Admission** (`src/admit.rs`, every addition, no override): (1) the owned gate on the
  original location and on the staged bytes, compared with the owned library when configured;
  (2) the generated gate: a `*.provenance.json` of format `marionette.generated_asset/1` (what
  the generation workstream writes beside provider output) refuses the asset unless a
  person-written `*.cc0.json` (format `marionette.cc0_dedication/1`, `dedicated_by`,
  `statement`) is present; tools refuse to write dedications, as they refuse declarations;
  (3) license files shipped with the asset must say CC0 or public domain and must not name
  another license (CC BY, NonCommercial, ShareAlike, GPL, "all rights reserved", store EULAs);
  (4) proof by origin: made needs a method; online needs a known library, a page on its domain,
  downloads from its domains, and for archive libraries (Kenney, Quaternius) the archive's own
  CC0 license file; (5) safe paths, and the main file must load (PNG decodes, glTF loads, WAV/OGG
  header). Ids are `<name slug>-<12 hex of the content hash>` unless given; re-adding the same
  content is a no-op, a different asset under a taken id is refused.
- **Online libraries** (`src/sources/`). Poly Haven: `api.polyhaven.com` `/assets`, `/info`,
  `/files`; textures keep color, OpenGL normal, roughness, AO and displacement maps; models keep
  the glTF with its included files; every file is checked against the published size and MD5;
  JPEG is converted to PNG (the engine's loader reads PNG) and glTF image paths are updated;
  HDRIs are not pooled yet. ambientCG: `api/v2/full_json`, materials only, the ZIP of the chosen
  resolution (PNG preferred) checked against its published size, maps renamed to the same
  names. Kenney and Quaternius have no search API: a pack is ingested from its archive (URL on
  the library's own domain, or a file a person downloaded) and must carry its license file;
  `include` picks part of a pack. API shapes were read from live answers on 2026-11-12 and are
  parsed by key name.
- **Transport.** HTTP requests are data (`marionette_providers::http`); the CLI sends them with
  the system `curl` (https only, redirects limited to https, 1 GiB cap), which ships with
  Windows 10 and later and with common Linux systems and honours the user's proxy settings.
  Tests use a scripted transport, so the suite needs no network. Downloading CC0 files is free;
  no spending gate applies.
- **Lookup order** (`src/search.rs`, operation `asset_search`): project files (by path), owned
  library, pool (name, tags, id), online libraries (only with `online: true`), then a pointer to
  `generate_plan`/`generate_run`. It stops at the first step with results unless `all_steps`.
- **Operations.** `asset_search`, `pool_info`, `pool_list`, `pool_get`, `pool_add` (files,
  folders or a library archive), `pool_ingest` (Poly Haven, ambientCG by id; Kenney, Quaternius
  by archive URL), `pool_fetch` (copy into the project after checking SHA-256), `pool_index`.
- **Seed set.** Five assets made in code by `crates/marionette-pool/src/seed.rs` (prototype
  grid, red bricks, gray noise, wooden crate, blip sound); `cargo run -p marionette-pool
  --example make_seed -- pool` rebuilds `pool/` byte for byte, and a test compares them.

## Alternatives considered

- A Rust HTTP client (ureq with rustls, reqwest): a TLS stack in the build, its own root
  certificates (proxies with their own CA fail), more compile time; `curl` is already on the
  target systems. The transport is a trait, so a native client can replace it later.
- Pooling online assets by reference only (URL plus hash): the pool must keep working if a
  library moves or disappears, and the assets repository is the place to keep the bytes.
- Trusting a library's site-wide policy alone for archive libraries: packs on those sites are
  uploaded over many years; the license file inside the archive is the per-pack evidence.

## Consequences

- Agents can find, ingest and use CC0 assets with their provenance; nothing owned, generated
  without a person's dedication, or under another license can enter.
- Not covered: Poly Haven HDRIs, ambientCG non-material types, sounds from online libraries,
  ZIP64 archives, publishing to the assets repository (it does not exist yet), a live run of
  the online adapters from this build (the cloud build machine's network blocks those sites).
