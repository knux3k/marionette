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
