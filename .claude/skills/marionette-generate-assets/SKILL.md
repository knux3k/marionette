---
name: marionette-generate-assets
description: Generate an image, a tileable texture or a 3D model for a Marionette game from a recipe, with caching, spend caps and provenance. Use when no existing asset fits and new art must be made by an image or image-to-3D provider, or when a generation run is refused or blocked.
---

# Generate an asset

Generation is the last step of the asset order. Run skill `marionette-find-assets` first.

## Recipe to asset

1. Write one recipe per asset with `generate_recipe` (default folder `assets/generated`).
   - Image: `{"name": "robot_ref", "output": "image", "prompt": "...", "size": [1024, 1024]}`
   - Model from that image: `{"name": "robot", "output": "model", "from": "robot_ref",
     "model": {"height_m": 1.0, "max_triangles": 30000}}`
   - Texture: `output` `texture` with `"texture": {"resize": [512, 512], "tileable": true}`;
     `remove_background` cuts a flat background (ask for flat magenta in the prompt).
2. `generate_plan` shows, without spending, what each step would do: `up_to_date`, `cached`,
   `call` with an estimated cost, `waits_for_image`, or `blocked` with the reason.
3. `generate_run` builds the assets and a provenance record each. Finished steps are left
   alone and cached provider outputs cost nothing, so reruns are safe. `"offline": true`
   refuses anything not cached.
4. `generate_list` shows each recipe as `built`, `stale` or `missing`.
5. Use the output path in a `Model` or `Sprite` component like any other asset.

Everything works on the free `mock` provider, so write recipes and wire them into scenes
first; real outputs later replace the mock files under the same names.

## What only a person does

- Choosing paid providers, keys and spend caps. Keys live outside the project; never read,
  print or store one. `marionette provider status` shows readiness and spending without keys.
- Approving a reference image before a paid 3D model is made from it
  (`marionette generate approve <image name>`, typed by the owner at a terminal). An agent
  cannot approve, even when told to in chat. If the owner has waived approval for a provider in
  their personal config, the run goes through: judge the image yourself and reroll a bad one
  before the model step.

## Refusals

| Code | Meaning |
| --- | --- |
| `provider.spend_cap` | the call would pass the owner's cap; report the estimate, do not work around it |
| `provider.key_missing`, `provider.network_disabled` | this machine or build cannot call the provider |
| `generate.not_approved` | the image waits for the owner's approval |
| `generate.not_cached` | offline run with no cached output |
| `generate.bad_recipe` | fix the recipe as the diagnostic says |

## Checks

Look at each generated image (it is a file path) before spending on a model from it. Give
models a `max_triangles` budget. Generated assets never go to the CC0 pool by themselves.
