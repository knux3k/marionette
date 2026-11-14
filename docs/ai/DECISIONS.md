# Decisions index

Files are in `docs/decisions/`. Add a line here with every new record.

| # | Decision | Status |
| --- | --- | --- |
| 0001 | ECS is `bevy_ecs` standalone; schema from `bevy_reflect` | accepted |
| 0002 | Scene files are canonical JSON | accepted |
| 0003 | SplitMix64 RNG and SHA-256 state hash | accepted |
| 0004 | Render data components live in core; renderer is a separate crate | accepted |
| 0005 | Renderer architecture: wgpu forward PBR, batched sprites, per-view data | accepted |
| 0006 | Reference images and their tolerance | accepted |
| 0007 | Exact line and column for scene diagnostics | accepted |
| 0008 | MCP server on the official Rust SDK (`rmcp`) | accepted |
| 0009 | Live sessions and scene files are separate targets | accepted |
| 0010 | One operation table for CLI and MCP | accepted |
| 0011 | Engine modules and fixed stages | accepted |
| 0012 | Physics on Rapier with written-back state | accepted, proofs and cost done |
| 0013 | Input as actions, recorded at action level | accepted |
| 0014 | Audio decided in the simulation, played by the presentation layer | accepted |
| 0015 | Minimal UI as components, built-in bitmap font | accepted |
| 0016 | No identifying details in the repository | accepted, permanent |
| 0017 | The window hands input and frames to its caller through hooks | accepted |
| 0018 | Owned library, license declarations and the Unity importer | accepted |
| 0019 | Swappable AI providers, offline by default, spending gated twice | accepted |
| 0020 | Porting Unity and Unreal code: scan, concept tables, TypeScript skeletons, coordinate conversion | accepted |
| 0021 | Scripts spawn, clone and destroy entities (`onDestroy`) and set timers; all saved as state | accepted |
| 0022 | Game scripts are TypeScript on QuickJS-ng (spike: TypeScript against Luau) | accepted |
| 0023 | Performance budgets, checked by tests | accepted |
| 0024 | Deterministic data-parallel jobs | accepted |
| 0025 | Instanced, multi-view renderer and bulk draws | accepted |
| 0026 | Factory simulation module | accepted |
| 0027 | FBX to glTF conversion inside the importer | accepted |
| 0028 | Skeletal animation: clips on the fixed tick, GPU skinning and blend shapes | accepted |
| 0029 | The Unreal importer reads exports (FBX, glTF, images, T3D materials), diagnoses packages | accepted |
| 0030 | Job TypeScript: a numbers-only TypeScript subset compiled to WebAssembly for job bodies | accepted |
| 0031 | Stability suite per game mode: seeded soak runs, crash reports, every scene registered | accepted |
| 0032 | Memory and crash hygiene: no leaks, nothing outlives a crash (`marionette-host`) | accepted |
| 0040 | Unity prefabs become flat Marionette prefabs in the owned library (`prefab_instantiate`) | accepted |
| 0041 | The CC0 pool: manifest with provenance and proof, admission rules, online libraries over `curl`, the asset lookup order | accepted |
| 0045 | Asset generation from recipes, cached by request, gated by a person | accepted |
| 0046 | Personal provider addresses and models (Qwen plan addresses) | accepted |
| 0047 | Owner may waive reference image approval in their personal config | accepted |
| 0048 | Slim generated models to their triangle and texture budgets | accepted |
| 0050 | Proof scenes are small original games built from operation recipes and tested by play | accepted |
| 0060 | Four standing focuses (modularity, token use, performance, graphics) in FOCUSES.md | accepted |
| 0061 | Small always-read docs with a size test; selectable and lean MCP tool listings | accepted |
| 0062 | Games add their own Rust modules and operations (`Engine::builder`, `marionette_cli::main_with`); optional engine parts are cargo features | accepted |
| 0063 | Opt-in looks: shadows, point and spot lights, sky light, emissive, tone mapping, fog | accepted |
| 0064 | Bloom through an HDR target, opt-in (`Bloom`) | accepted |
| 0065 | Shadows from point and spot lights, per light (`CastShadows`) | accepted |
| 0066 | Environment lighting, reflections and sky background (`Environment`) | accepted |
| 0067 | glTF emissive materials (factor, texture, emissive strength) | accepted |
| 0068 | Models first needed mid-game decode on a background thread (`set_streaming`; window loop and render stability modes) | accepted |
| 0069 | Stability memory checks read the counted heap when the binary installs it (all platforms) | accepted |
| 0070 | Scenes and scripts run job TypeScript over job arrays saved with the scene (`JobData`, `JobSchedule`, `engine.jobs`, `JobDraw`) | accepted |
| 0071 | More CC0 sources: KayKit (GitHub mirrors, prefix-scoped) and cgbookcase; ShareTextures refused | accepted |
| 0072 | Unity Burst jobs port to the game's own Rust module; the scanner sees jobs, structs and partial classes; `port_scaffold` writes `jobs.rs` | accepted |
| 0073 | Saving copies state on the main thread and writes it elsewhere (`Sim::capture`); registered types need `Clone` | accepted |
| 0074 | Blend spaces (gradient band weights, phase sync) and masked or additive overlays as optional components | accepted |
| 0075 | 3D sound through one `hear` formula (distance curves, rear muffle, Doppler, head pose), live voices, layered `Music` | accepted |
| 0076 | Save games: `marionette-save` with slots, crash-safe `.msave` files and backups, versioned saves with data and Rust migrations, background writer, autosave | accepted |
| 0180 | Meshes built in game code: `GameMeshes` resource, `mesh:<name>` assets, in-place updates, vertex colors for every mesh | accepted |
| 0181 | Game shaders: WGSL `surface`/`vertex` hooks spliced into the PBR shader (`ShaderMaterial`, `GameShaders`, `.wgsl` files), blend options, game time, `primitive:grid` | accepted |
| 0080 | Frustum culling always on (pixel-identical, shadow casters kept); level of detail opt-in (`LevelOfDetail`, generated levels) | accepted |
| 0081 | Render scale 0.5 to 2 with a clamped Catmull-Rom upscaler and sharpening (`RenderScale`), opt-in | accepted |
| 0082 | Foveated rendering by skipping 2x2 quads outside an ellipse and filling them (`Foveation`), opt-in | accepted |
| 0083 | No DLSS (license forbids open source use; owner chose not to use it); vendor-neutral upscaling only | accepted |
| 0090 | Sun shadow cascades (`ShadowCascades`), sphere-fitted and texel-snapped | accepted |
| 0091 | HDR environment files: Radiance `.hdr` (own decoder), OpenEXR `.exr` (feature `exr`) | accepted |
| 0092 | Reflection probes (`ReflectionProbe`): captured on change, box-corrected | accepted |
| 0093 | Ray-traced shadows and reflections through ray queries (`RayTracing`), opt-in with fallback | accepted |
| 0100 | One shared performance suite with output checks (`perf_suite`, `perf_compare`), shared-machine lock and idle checks | accepted |
| 0101 | Shipping builds use fat LTO with one codegen unit (`--profile dist`); `release` stays | accepted |
| 0110 | Bulk draw hooks get the frame's camera (`BulkDrawContext`: alpha, eye, forward) | accepted |
| 0140 | Game packages per target with one command (`marionette package`); Linux held to the glibc 2.31 Steam runtime floor; Steam Frame = Linux ARM64 | accepted |
| 0130 | Scene editor view: crate `marionette-editor` (feature `editor`), overlay as UI draws, CPU picking, edits as operation calls with undo, offscreen `editor_view` | accepted |
| 0150 | Game starter: a brief (or a prompt) and genre templates with an autopilot make a playable, tested game (`game_brief`, `game_make`, `marionette make`) | accepted |
| 0160 | Task skills in `.claude/skills` that agents load by themselves; compiled in, installed into games (`skills_install`), readable through operations, checked by a test | accepted |
| 0161 | Handoff files in every game project (`docs/ai`, `docs/decisions`), one file per claim and session; `handoff_status`, `handoff_claim` (refuses overlaps, reserves decision blocks), `handoff_note` | accepted |
| 0162 | Proofs in one file per area under `docs/ai/proofs/`; `PROOFS.md` is the index | accepted |
| 0170 | Windowed play picks present mode, frame queue, frame cap; borderless fullscreen toggle; cursor lock | accepted |
| 0171 | Focus loss releases held input; hidden windows sleep; vsync frame times snap to the refresh | accepted |
| 0172 | Gamepads through gilrs in `marionette-gamepad`, polled by window hooks | accepted |
| 0173 | Every windowed run measures frame pacing and input latency (`FrameStats`, `--frame-stats`) | accepted |
| 0190 | Headsets through OpenXR: poses enter as recorded input, gameplay in `marionette-xr`, the runtime in `marionette-openxr` (Vulkan through wgpu-hal, one swapchain per eye) | accepted |
| 0200 | World-space UI: one entity per panel, distance-field quads through a generic `QuadDrawHooks`, optional crate `marionette-ui3d` (feature `ui3d`), no shipped font | accepted |
| 0201 | UI input is a `UiPointers` hook (rays and touch) any source fills; events and item values change in `Stage::Logic` | accepted |
| 0210 | Co-op networking is host-stamped lockstep over a pluggable transport (`marionette-net`): reliability layer, loopback with simulated loss, UDP, hash checks, late join by snapshot | accepted |
| 0211 | Networked input is per-player actions `p<slot>.<action>` and a `NetPlayers` resource | accepted |
| 0212 | Steam is an optional backend that loads the game's own steam_api library at run time (no SDK files) | accepted |
| 0213 | Achievements, stats and cloud files behind interfaces, offline backend, three-way save-folder sync (`marionette-platform`) | accepted |
| 0220 | Real-time synth crate (lock-free commands, sample-accurate notes, generators, disk streaming), offline render with numeric checks, `audio-check` device run | accepted |
| 0230 | Round worlds: `GravityWell` (summed point gravity), `SphereFluid`, `PlanetCharacter` with up from gravity, `MoveIntent` in core | accepted |
| 0231 | Navigation on round, changing worlds: cube-sphere cell grid per `NavSurface`, ground probing, `NavObstacle` stamps, crate `marionette-nav` | accepted |
| 0232 | Many agents: capped search queue, shared goal searches, weighted A*, galloping smoothing, parallel steering, `Jobs::map_chunks_heavy` | accepted |
| 0240 | `port_serde`: Rust serde types from C# [Serializable] classes and structs, following JsonUtility's rules | accepted |
| 0241 | `port_serde` on a real project: const and static initializers, expression-bodied properties, `external` types (also Unity value types) | accepted |
| 0260 | Merge queue: pull requests run a fast Linux check, the queue and main run the full suite, `CI gate` is the required check | accepted |
| 0270 | Scene edits cost the change: `SceneIndex`, indexed file and session edits with the whole-scene path as fallback, one write or world load per batch, lazy JSON positions | accepted |
| 0280 | Model textures get CPU-made mipmaps (linear-light color, vector normals, cut-out coverage kept) and 16x anisotropic filtering | accepted |
| 0281 | Cut-out and see-through materials from glTF alpha modes, double-sided materials; lazily built pipeline variants; see-through parts sorted back to front | accepted |
| 0282 | Particles computed on the GPU from game time alone (`ParticleEmitter`): no stored state, one draw per emitter | accepted |
| 0283 | Opt-in BC7 model textures (`TextureCompression`): ISPC encoder, every mip level, bounded per-user disk cache; RGBA8 where BC is missing | accepted |
| 0284 | Realistic render benchmark (`perf_realistic_scene`): 18M triangles, 257 materials, 99 2K textures, 3,200 movers; same scene buildable in Unity (`tools/unity-bench`) | accepted |
| 0285 | Single-pass stereo: both eyes in one multiview pass, shader multiview forms made from the normal sources, a pass per eye where not covered | accepted |
| 0290 | Physics, audio, animation and XR are cargo features of `marionette-agent`; the factory module moves to `examples/factory` with its benchmark, budgets and stability modes | accepted |
| 0271 | Scripts pass component values directly instead of JSON text (`convert`: read plans, `patch`, text path as fallback and reference); turns skip unused work; script budgets | accepted |
| 0272 | Parent and child transforms: `ChildOf` (local placement, `Transform` stays world), placed at load and before and after physics, cycles refused, `entity_attach` | accepted |
