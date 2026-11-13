# Marionette

An open source game engine built to be driven by coding AIs. Everything is text, it runs
with no window, it describes itself as JSON Schema, and it reports errors as data.

Status: early. Plan lines 1, 2, 3 and 5 of [`docs/DESIGN.md`](docs/DESIGN.md) are in place and line 6 (input, physics, audio, UI) is in progress: an ECS
world, a schema registry, a canonical JSON scene format with a validator, a deterministic
fixed-step loop, a wgpu renderer (glTF PBR models and 2D sprites) with offscreen screenshots,
and the `marionette` CLI.

## Quick start

Requires stable Rust (MSVC toolchain on Windows).

```bash
cargo build
cargo run -p marionette-cli -- run examples/drift.scene.json --headless --ticks 1000 --seed 1 --json
```

Commands (all accept `--json`):

| Command | What it does |
| --- | --- |
| `marionette schema` | JSON Schema of the scene format and every component and resource |
| `marionette validate <scene>` | Checks a scene; errors have a code, line, entity and suggested fix |
| `marionette run <scene> --headless --ticks N --seed S` | Simulates and prints a SHA-256 state hash |
| `marionette run <scene>` | Opens a window (Escape closes; `--ticks N` closes after N ticks) |
| `marionette screenshot <scene> --ticks N --seed S --camera ID --size WxH --out f.png` | Renders one PNG with no window |
| `marionette bench <scene> --frames N --size WxH` | Measures offscreen frame time |
| `marionette ops` | Lists every operation with its JSON Schemas |
| `marionette op <name> '<json>'` | Runs one operation (`--project DIR`, `--read-only`) |
| `marionette mcp [--tools core,pool,...] [--lean]` | Serves operations as MCP tools over stdio (all by default) |
| `marionette provider use <kind> <provider>` | Switches the AI provider for `language`, `image` or `model3d` (`provider list`, `provider status`) |

Operations: `project_init`, `project_validate`, `project_schema`, `scene_create`,
`scene_validate`, `scene_format`, `scene_run`, `entity_create`, `entity_update`,
`entity_delete`, `component_set`, `component_remove`, `session_start`, `session_step`,
`session_stop`, `session_list`, `session_save`, `observe_tree`, `observe_query`,
`observe_entity`, `observe_screenshot`, `observe_logs`, `observe_stats`, `test_run`,
`library_*`, `license_check_text`, `pool_admission_check`, `provider_list`, `provider_status`,
`provider_use`, `provider_generate`. Full list: `docs/ai/reference/operations.md`.

AI providers default to an offline mock. Real providers read keys from environment variables
(`ANTHROPIC_API_KEY`, `DASHSCOPE_API_KEY`, `TRIPO_API_KEY`) and are limited by a spend cap the
owner sets (`MARIONETTE_SPEND_CAP_USD`); see decision 0019.

Start a game project:

```bash
marionette op project_init '{"path": "mygame"}' --json
```

## Connecting an agent over MCP

Claude Code: add a `.mcp.json` file in the game project's folder (or run
`claude mcp add marionette -- marionette --project . mcp --tools core --lean`):

```json
{
  "mcpServers": {
    "marionette": {
      "command": "marionette",
      "args": ["--project", ".", "mcp", "--tools", "core", "--lean"]
    }
  }
}
```

`--tools core` lists only the edit, run, observe and test operations (about a third of the
table); add groups as the game needs them, for example `--tools core,anim,pool`, or leave the
flag out for all of them. `--lean` leaves output schemas out of the listing. Together they cut
the tool listing the client keeps in context to a small fraction (decision 0061).

Use `"args": ["--project", ".", "--read-only", "mcp"]` for an agent that may look but not change
anything.

Any other MCP client: start the process `marionette --project <game folder> mcp` and speak MCP
over its stdin/stdout (newline-delimited JSON-RPC 2.0). Tool results carry `structuredContent`;
failures have `isError: true` and `{"ok": false, "diagnostics": [...]}`. Agents without MCP can
call the same operations with `marionette op <name> '<json>' --json`.

## Skills

The engine ships task skills: short, tested procedures (build a scene, find or generate assets,
write a script or a job, port Unity or Unreal code, light a scene, animate, test, find a
slowdown, add Rust code). A new project (`project_init`) gets them in `.claude/skills` and
`.agents/skills`, where agents that support skills load the matching one by themselves;
`marionette op skills_install '{}'` adds or refreshes them in an existing project. Any other
agent reads them with `skills_list` and `skills_get`.
| `marionette scene fmt <scene> [--check]` | Rewrites a scene in canonical form |

## For agents

Start at [`docs/ai/START-HERE.md`](docs/ai/START-HERE.md).

Read [`AGENTS.md`](AGENTS.md) (same as [`CLAUDE.md`](CLAUDE.md)) for build steps, design
principles and determinism rules. Decisions are recorded in [`docs/decisions/`](docs/decisions/).

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option. Unless you explicitly state otherwise, any
contribution intentionally submitted for inclusion in this work, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or conditions.
