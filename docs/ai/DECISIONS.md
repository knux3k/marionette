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
| 0150 | Game starter: a brief (or a prompt) and genre templates with an autopilot make a playable, tested game (`game_brief`, `game_make`, `marionette make`) | accepted |
