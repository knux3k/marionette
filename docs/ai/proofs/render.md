# Proofs: Renderer and looks (plan line 3)

Line 3 (renderer):
- Reference images (320x180, 60 ticks, seed 1, MSAA 4), SHA-256 of the committed files:
  - `examples/reference/showcase.png`: `9563bd7152cc2ac32f2aff9119ad9ce8475e07cd96a085fc6c22651e6adab996`
  - `examples/reference/sprites.png`: `4b07882d40928aca85f67d169868b3e1a4bde97de84186c86d2e16c64e97e97d`
  - `examples/reference/ui.png`: `4c7417beb5c10b9bba89ad7ab9721a1a49aa8c566ca2bd4c7e9c36f4fa56df06`
- Tests: `crates/marionette-render/tests/render.rs` (`same_inputs_give_identical_images`,
  `matches_reference_images`, `rendering_does_not_change_simulation_state`).
  `screenshot_is_deterministic_and_leaves_state_alone` in `crates/marionette-cli/tests/cli.rs`.
- Frame time, `examples/showcase.scene.json`, desktop GPU of the RTX 2080 class, Vulkan,
  1920x1080, 4x MSAA, offscreen, 2,000 frames, release build: average 0.27 ms, p99 0.31 to
  0.48 ms, worst 5.7 to 6.5 ms over three runs (budget 11.1 ms). Command:
  `cargo run --release -q -p marionette-cli -- bench examples/showcase.scene.json --frames 2000 --size 1920x1080`.
- State hashes at 60 ticks, seed 1 (`run ... --headless --ticks 60 --seed 1`): showcase
  `b95606f6c44132e9b2fd82caad0e37a7d056da1b267e5e4ec24be117de832c08`, sprites
  `3c90975a249c2cb5a44bb38e83d2116ba77c804b61352db3858c3dc93baaf3d1`.

Opt-in looks (decision 0063):
- Example `examples/lighting.scene.json`, built by `examples/lighting.ops.json`
  (`marionette op batch_run '{"file":"examples/lighting.ops.json"}'` after deleting the scene).
  `run examples/lighting.scene.json --headless --ticks 60 --seed 1` prints
  `5c7cb06da50ec51e24fafdaea3ba1a00c05902d7ecc570c2ef6535c6e8c0cecb` (ground widened to 300 m
  on 2026-11-13 so fog hides its edge, decision 0066).
- Reference image `examples/reference/lighting.png` (320x180, 60 ticks, seed 1, MSAA 4, made on
  Mesa lavapipe): `2e870ee61d22184d0e725de331ae613d9fe149dd17fb381948176565aec1787e`.
- Tests: `opt_in_looks_change_the_image_only_when_present` and the lighting entries of
  `matches_reference_images` and `same_inputs_give_identical_images` in
  `crates/marionette-render/tests/render.rs`; stability mode `render-3d-lighting`.

More looks (decisions 0064 to 0067):
- Example `examples/looks.scene.json` (bloom, a street lamp with a glTF emissive lantern and a
  shadow-casting point light, a shadow-casting spot light, mirror, brushed and glossy balls,
  environment from `SkyLight` colors with sky background, fog), built by
  `examples/looks.ops.json`. `run examples/looks.scene.json --headless --ticks 60 --seed 1`
  prints `8ef0a4c6887a8796e767348f6176bacf231f761c81155244ddb80e55ac7042e4`.
- Reference image `examples/reference/looks.png` (320x180, as above, Mesa lavapipe):
  `c338965a9568ec8fb3ec7a464fb2dd64057ef97967b68047dd9bc98161f56b16`.
- Tests in `crates/marionette-render/tests/render.rs`:
  `point_and_spot_lights_cast_shadows_only_with_cast_shadows`,
  `bloom_spreads_bright_light_and_only_with_bloom`,
  `environment_lights_reflects_and_draws_the_sky`, `imports_gltf_emissive`, the looks entries
  of the reference, determinism and GPU-object tests; unit tests in `post.rs` and
  `environment.rs`. Existing reference images (showcase, sprites, ui) unchanged; lighting
  within tolerance before its ground change. Stability mode `render-3d-looks` (two eyes).
- Desktop GPU (RTX 2080 class, Vulkan, 1920x1080, MSAA 4, 2,000 frames, release, two runs
  each): lighting 0.45 ms avg (p99 0.69); looks 1.01 ms (p99 1.35); looks two eyes 1.67 ms
  (p99 2.14); looks without `Bloom` 0.63 ms, without `CastShadows` 0.86 ms, without
  `Environment` 0.97 ms. So bloom costs about 0.38 ms, two shadow-casting lights (seven maps)
  0.15 ms, the environment 0.05 ms. Worst frames 5 to 7 ms in every case, baseline included.
  GPU tests passed there (`MARIONETTE_REQUIRE_GPU=1`), release stability `render-3d-looks`
  passed with frame p99 1.21 ms (budget 11.1 ms). Command:
  `cargo run --release -q -p marionette-cli -- bench examples/looks.scene.json --frames 2000 --size 1920x1080 [--eyes 2]`.

## Culling and level of detail (decision 0080)

- `culling_drops_unseen_instances_without_changing_the_image` (`marionette-render` GPU tests):
  pixels identical with culling on and off; 121 of 123 instances not drawn in view; a box out of
  view still shadows the view.
- `level_of_detail_reduces_far_triangles_only_when_asked`: far spheres draw a reduced mesh
  (over 20% fewer triangles, mean pixel difference under 2 of 255), a tiny one is hidden.
- Unit tests in `crates/marionette-render/src/cull.rs` (frustum planes, sphere transform,
  shadow-only class, level choice, simplifier levels).
- 32x factory (`perf_factory_scene` scale 32), `bench --headset --size 1024x1024`, release,
  Mesa lavapipe in a 4-core cloud container: instances drawn per eye 9,792 -> 4,895,
  triangles 117,504 -> 58,740, record phase p50 1.12 -> 0.99 ms. GPU wait on lavapipe is fill
  bound (53.5 -> 50.5 ms) and says nothing about a hardware GPU; hardware numbers UNVERIFIED.

## Render cost on a desktop GPU (decisions 0080 to 0082)

RTX 2080, Vulkan, offscreen, release, 4x MSAA, `bench --headset` (two 2160x2160 eyes), 600
frames after 60 warmup, commit aabe416. p50 frame ms (GPU wait p50 in brackets). b = scale
0.75 sharpness 0.3, c = 0.5, d = foveation 0.6, e = b with d, f = scale 1.5.

| Scene | plain | b | c | d | e | f |
| --- | --- | --- | --- | --- | --- | --- |
| looks (bloom) | 2.65 (2.07) | 2.58 (1.97) | 1.96 (1.35) | 2.85 (2.21) | 4.40 (3.56) | 5.32 (4.61) |
| lighting | 1.12 (0.92) | 1.46 (1.23) | 1.16 (0.94) | 1.20 (0.97) | 3.09 (2.85) | 2.44 (2.20) |

- 32x factory: culling 287,232 -> 145,632 triangles, p50 4.21 -> 3.86 ms, max 12.1 -> 5.8 ms;
  the frame is bound by render record (1.4 ms) and extraction, not the GPU.
- looks at 1080p: plain 1.00, b 0.99.
- e was slow because with foveation the upscale used a 16-tap filter filling holes per tap.
  Since then the fast filter runs inside the ellipse and a 4-tap bilinear outside
  (`foveation_keeps_the_center_and_fills_the_edges` checks the combination); not yet re-timed.

Remaining lighting (decisions 0090 to 0093):
- Example `examples/raytracing.scene.json` (built by `examples/raytracing.ops.json`): HDR sky
  file, three sun cascades, mirror floor and ball, a crystal model, ten pillars, a
  shadow-casting lamp, a reflection probe, `RayTracing` with soft sun. Reference images
  `examples/reference/raytracing.png` (ray queries, Mesa lavapipe) and
  `raytracing-fallback.png` (no ray queries).
- Tests: `crates/marionette-render/tests/lighting.rs` (cascades reach far shadows a single
  map misses; traced shadows need no shadow maps; traced reflections and probes show other
  objects; `.hdr` keeps light above 1.0), the raytracing entries of the reference and
  determinism tests, `cull.rs` `ray_traced_frames_keep_everything_for_rays`, unit tests in
  `hdr.rs`, `probes.rs`, `raytrace.rs`. Stability mode `render-3d-raytracing` (two eyes).
- Desktop GPU (RTX 2080 class, Vulkan, the default backend there, 1920x1080, MSAA 4, 2,000
  frames, release, at 393b663): ray tracing on 1.78 ms avg (p99 2.23; two runs within
  0.02 ms), two eyes 3.25 ms (p99 3.83); same scene with `MARIONETTE_RAY_TRACING=0`
  (cascades, shadow maps, probe) 1.09 ms (p99 1.34-1.37). So tracing costs about 0.7 ms per
  1080p view, nearly all GPU (wait 1.38 vs 0.64 ms), with 21 draw calls instead of 53.
  Baselines in the same session: looks 1.03 ms, lighting 0.47 ms, looks two eyes 1.67 ms.
  GPU tests passed with ray queries required (`lighting` 5, `render` 18); stability
  `render-3d-raytracing` frame p99 1.81 ms, `render-3d-looks` 1.02 ms (budget 11.1 ms).
  Before the depth prepass and adaptive sun rays the same scene measured about 2.4 ms
  (noisy run).
