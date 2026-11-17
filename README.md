# Marionette

An open source game engine built to be driven by coding AIs, such as Claude Code, Codex, Gemini
CLI or Cursor, with a person steering. Everything is a text file, every action is a command with
JSON in and out, the whole engine is offered to AI agents as MCP tools, it runs with no window
for tests, and its errors say what went wrong and how to fix it.

Written in Rust on wgpu. Runs on Windows and Linux. Licensed MIT OR Apache-2.0.

**Status: 0.1.0, early.** The core (scenes, deterministic simulation, tests, operations, MCP) is
solid and heavily tested. Rendering, physics, input, saves, scripting, the editor and packaging
are usable. VR, networking, ray tracing and several importers are first slices. Each feature's
level, with evidence, is in [docs/ai/STATUS.md](docs/ai/STATUS.md). Expect breaking changes before 1.0.

## What you get

- **A game from one sentence.** `marionette make "a 2d platformer where a fox collects acorns"`
  writes a playable, tested game project, proven winnable by an autopilot.
- **AI-first tooling.** More than 100 operations (create entities, run, look at a screenshot, test,
  profile) as a CLI and as an MCP server; skills that teach an agent the engine's procedures;
  handoff files so several agents can share a project.
- **Deterministic simulation.** Same seed, same inputs, same state hash on every machine, so
  tests and replays are exact.
- **Rendering:** PBR with glTF models, shadows, sky, HDR environments, bloom, fog, particles,
  level of detail, MSAA, optional ray-traced reflections, sprites and screen UI.
- **Gameplay:** TypeScript scripts, parallel TypeScript jobs compiled to WebAssembly, game code
  in Rust, Rapier physics, navigation on round planets, saves, audio and a synth, OpenXR
  headsets, co-op lockstep networking with optional Steam.
- **Assets:** a public CC0 asset pool, Unity and Unreal importers and porting tools, and optional
  image and 3D model generation with your own provider keys and a spend cap.
- **Shipping:** `marionette package` builds a zip per target (Windows, Linux, Steam Deck, Steam
  Frame) with the licenses included.

## Quick start

1. Get `marionette`: download the archive for your system from the
   [releases page](https://github.com/knux3k/marionette/releases), unpack it and put the folder
   on your `PATH`. Or build it from source (needs Rust 1.95 or newer; on Debian or Ubuntu first run
   `sudo apt update && sudo apt install libasound2-dev libudev-dev pkg-config`, and expect a
   first build of about 5 minutes):

   ```bash
   git clone https://github.com/knux3k/marionette
   cd marionette
   cargo install --locked --path crates/marionette-cli
   ```

2. Make a game and play it:

   ```bash
   marionette make "a 2d platformer where a fox collects acorns and avoids hedgehogs" --path fox
   marionette run scenes/main.scene.json --project fox
   ```

3. Connect your coding AI (Claude Code shown; any MCP client works):

   ```bash
   cd fox
   claude mcp add marionette -- marionette --project . mcp --tools core --lean
   claude
   ```

   Then ask it for changes in plain words: "make the hedgehogs faster and add a second level".

The full path, including tests, art and packaging a game for players, is in
**[docs/GETTING-STARTED.md](docs/GETTING-STARTED.md)**. How to use the AI app subscription you
already have, other agents, and provider API keys is in
**[docs/CONNECT-YOUR-AI.md](docs/CONNECT-YOUR-AI.md)**.

## Commands

All commands accept `--json` and print one JSON object.

| Command | What it does |
| --- | --- |
| `marionette make "<idea>"` | Makes a playable, tested game project |
| `marionette run <scene>` | Plays a scene in a window (`--headless --ticks N --seed S` prints a state hash instead) |
| `marionette edit <scene>` | Opens the scene editor |
| `marionette screenshot <scene> --out f.png` | Renders one image with no window |
| `marionette op <name> '<json>'` | Runs one operation; `marionette ops --brief` lists them |
| `marionette mcp` | Serves the operations as MCP tools over stdio |
| `marionette provider list` | AI providers for generation and whether their keys are set |
| `marionette package <game>` | Builds a zip of the game for players |
| `marionette validate <scene>` | Checks a scene; errors carry a code, location and suggested fix |

Every command and flag: [docs/ai/reference/cli.md](docs/ai/reference/cli.md). Every operation:
[docs/ai/reference/operations.md](docs/ai/reference/operations.md).

## Documentation

- People: [Getting started](docs/GETTING-STARTED.md),
  [Connect your AI](docs/CONNECT-YOUR-AI.md), [Changelog](CHANGELOG.md),
  [Contributing](CONTRIBUTING.md), [Security](SECURITY.md), [Design](docs/DESIGN.md).
- AI agents making a game with the engine: the `marionette-make-game` skill in `.claude/skills`
  and [Getting started](docs/GETTING-STARTED.md). AI agents changing the engine: start at [docs/ai/START-HERE.md](docs/ai/START-HERE.md) and
  [AGENTS.md](AGENTS.md) (the same as [CLAUDE.md](CLAUDE.md)). The `docs/ai` folder is written
  for agents: short, exact and checked by tests. Decisions are in [docs/decisions/](docs/decisions/).

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option. Unless you explicitly state otherwise, any
contribution intentionally submitted for inclusion in this work, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or conditions.

Prebuilt binaries and packaged games include `THIRD-PARTY.txt` with the license of every Rust
crate linked in. Most are MIT or Apache-2.0; the audio decoder (symphonia) and one audio buffer
crate are MPL-2.0, whose source is available unchanged on crates.io. The asset pool is CC0.
