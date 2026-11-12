# Generating assets

Generation is the last step of the asset lookup order (ASSETS-AND-LICENSES.md rule 1): search
the project, the owned library and CC0 sources first. Design: decision 0045. Code:
`crates/marionette-generate`, operations in `crates/marionette-agent/src/generate.rs`.

## Recipe to asset

1. Write a recipe (operation `generate_recipe`, or a file `assets/generated/<name>.recipe.json`):

   ```json
   {"format": "marionette.recipe/1", "name": "robot_ref", "output": "image",
    "prompt": "small round robot, front view, plain light background", "size": [1024, 1024]}
   ```

   ```json
   {"format": "marionette.recipe/1", "name": "robot", "output": "model",
    "from": "robot_ref", "model": {"height_m": 1.0, "max_triangles": 30000}}
   ```

   ```json
   {"format": "marionette.recipe/1", "name": "grass", "output": "texture",
    "prompt": "short grass seen from above, even light", "size": [1024, 1024],
    "texture": {"resize": [512, 512], "tileable": true}}
   ```

   Fields: `name` (lowercase, digits, `_`, `-`), `output` (`image`, `texture`, `model`),
   `prompt`, `size` (image and texture), `texture` {`resize`, `tileable`, `remove_background`},
   `from` (model: an image or texture recipe), `model` {`height_m`, `max_triangles`},
   `provider` {`provider`, `model`} to pin one, `notes`. `remove_background` makes a flat
   background transparent after the resize: the key is the mean of the four corner pixels,
   colours near it fade out, and a band along the cut loses the key's tint (ask the model for
   a flat magenta background, which subjects rarely use). Post-processing is not part of the
   cache key, so changing these options rebuilds offline for free.
2. `generate_plan` (or `marionette generate plan`): per step `up_to_date`, `cached`, `call`,
   `waits_for_image` or `blocked` with the reason; estimates and caps.
3. `generate_run` (`marionette generate run [names] [--offline]`): writes `<name>.png|.glb`
   and `<name>.provenance.json`. Steps already built are left alone; cached steps cost nothing.
4. A model from an image with a paid 3D provider: the run builds the image and refuses the model
   (`generate.not_approved`). The project owner looks at the image and runs
   `marionette generate approve <image name>` in a terminal; run again. For many images at
   once, `marionette generate approve --all` from the project root lists every reference image
   models wait on and approves them all after the person types `yes`. Agents cannot approve,
   even when the owner says so in chat: the owner runs the command.
5. `generate_list`: state of every recipe (`built`, `stale`, `missing`).

Use the outputs like any asset: a texture or image path in a `Sprite` or material, a model path
in a `Model` component. Everything works on the free `mock` provider, so recipes can be written
and wired into scenes before any paid call; real outputs replace the mock files under the same
names later.

## Paid providers (a person sets these up; agents never do)

- Providers: `marionette provider use image qwen`, `marionette provider use model3d tripo`.
- Keys, on the machine that makes the calls, outside every project: environment variables
  (`DASHSCOPE_API_KEY`, `TRIPO_API_KEY`), or the user config file
  (`%APPDATA%\marionette\config.json` on Windows, `~/.config/marionette/config.json` elsewhere)
  naming files:

  ```json
  {"key_files": {"DASHSCOPE_API_KEY": "<path to a file>", "TRIPO_API_KEY": "<path to a file>"},
   "provider_caps_usd": {"qwen": "unlimited", "tripo": 10}}
  ```

- Caps: `spend_cap_usd` (total, default zero) and `provider_caps_usd` in that file, or
  `MARIONETTE_SPEND_CAP_USD` and `MARIONETTE_SPEND_CAP_<PROVIDER>_USD`.
- Qwen keys from any Model Studio region work: the international, China and US addresses are
  tried in order.
- A Model Studio plan key (`sk-sp-...`) needs the plan's own address and model, in the same
  file (decision 0046): `"provider_endpoints": {"qwen": {"url": "<address, or a file holding
  it>", "model": "qwen-image-3.0-pro"}}`. Coding-plan addresses serve chat models only.
- In Windows PowerShell 5.1, inner JSON quotes are stripped from arguments: run `marionette op`
  with JSON from Git Bash or `cmd`, or escape quotes as `\"`.
- A network build: `cargo build --release -p marionette-cli --features net`.
- `marionette provider status` shows readiness, spending and caps without showing any key.

## Refusal codes

`provider.spend_cap`, `provider.key_missing`, `provider.network_disabled`,
`provider.keys_file_in_project`, `generate.not_approved`, `generate.approval_needs_person`,
`generate.approval_write_refused`, `generate.not_cached` (offline), `generate.waits_for_image`,
`generate.postprocess`, `generate.cache`, `generate.bad_recipe`, `generate.not_found`.

## Proofs

`crates/marionette-generate/tests/generate.rs`, `crates/marionette-cli/tests/generate_e2e.rs`,
`crates/marionette-providers/tests/network.rs` (`--features net`), unit tests in
`crates/marionette-generate/src/texture.rs` and `crates/marionette-providers/src/keys.rs`.
