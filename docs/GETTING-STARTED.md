# Getting started with Marionette

This guide takes you from nothing to a game a player can download: install, make a game, play
it, change it with your AI, test it, add art and package it. Every command here is run by the
release checks, so it should work as written. If it does not, please open an issue.

You need Windows 10 or 11, or Linux (x86-64, glibc 2.31 or newer), and a graphics driver with
Vulkan, DirectX 12 or OpenGL. Headless commands (tests, `--headless` runs) need no graphics at all.

## 1. Install

**Prebuilt (fastest).** Download `marionette-<version>-windows-x64.zip` or
`marionette-<version>-linux-x64.tar.gz` from the
[releases page](https://github.com/knux3k/marionette/releases), unpack it and add the folder to
your `PATH`. Check it:

```bash
marionette --version
```

**From source.** Needed for packaging games and for game code written in Rust. Install Rust 1.95
or newer from rustup.rs (on Windows, the MSVC toolchain). On Linux, also install the ALSA and
udev headers, for example `sudo apt install libasound2-dev libudev-dev pkg-config` on Debian or
Ubuntu. Then:

```bash
git clone https://github.com/knux3k/marionette
cd marionette
cargo install --locked --path crates/marionette-cli
```

The first build takes 5 to 10 minutes. `cargo install` puts `marionette` in `~/.cargo/bin`; if the
command is not found, add that folder to your `PATH`. After a plain `cargo build --release`, run
`target/release/marionette` instead. Without a terminal (an AI agent), `marionette make` needs
`--answer id=value` for each question or `--yes`; list the ids with `marionette op game_interview`. Add `--features net` to that command if you want the
engine to call paid image and 3D model providers with your own keys (prebuilt binaries already
include it; see [Connect your AI](CONNECT-YOUR-AI.md)).

Check the engine works on your machine. From the source folder this prints a fixed hash,
`7441258fdf64141911d44e0608c48498897564168684fdd86cd244e149926558`, on every machine:

```bash
marionette run examples/drift.scene.json --headless --ticks 1000 --seed 1
```

## 2. Make a game

```bash
marionette make "a 2d platformer where a fox collects acorns and avoids hedgehogs" --path fox
```

On a terminal it first asks a short interview (view, setting, player, art style, difficulty and
so on), each with a default you can accept with Enter. Add `--yes` to skip it. It then writes a
complete project in `fox/` and proves the level can be won by letting an autopilot play it:

```text
made "Fox And The Acorns" (2d platformer) in fox: the autopilot wins in 16.2 s (seed 5192); tests 2 passed, 0 failed; ...
play: marionette run scenes/main.scene.json --project fox
```

Two templates exist today, a maze and a platformer, each in 2D or 3D. Anything else you build
from there with your AI, or start from an empty project with
`marionette op project_init '{"path": "mygame"}'`.

What is in the folder:

| Path | What it is |
| --- | --- |
| `scenes/main.scene.json` | The level: every entity and its components, as plain JSON |
| `scenes/scripts/game.ts` | Gameplay in TypeScript (run deterministically by the engine) |
| `scenes/main.actions.json` | Input: keys and gamepad buttons mapped to actions |
| `tests/*.test.json` | Scenario tests: autopilot wins, doing nothing loses |
| `brief.json` | The design the game was made from; edit it and `make --refresh` to remake |
| `assets/generated/*.recipe.json` | Art still drawn as placeholders, ready to generate |
| `.claude/skills`, `.agents/skills` | Procedures your AI agent loads by itself |
| `docs/ai/` | Handoff notes so the next agent session knows where things stand |

## 3. Play it

```bash
marionette run scenes/main.scene.json --project fox
```

A window opens. Arrow keys or A and D move, Space jumps, R restarts, F1 lets the autopilot play,
Escape quits. Gamepads work too. To look around and change things by hand, open the editor:

```bash
marionette edit scenes/main.scene.json --project fox
```

Fly with the right mouse button and WASD, click an entity to inspect it, P plays, and the keys
are listed in the bottom bar.

## 4. Change it with your AI

The intended way to build on Marionette is to describe what you want to a coding AI that has the
engine's tools. With Claude Code, from inside the game folder:

```bash
cd fox
claude mcp add marionette -- marionette --project . mcp --tools core --lean
claude
```

Then, for example: "Make the hedgehogs twice as fast, add a double jump and a second level, and
keep the tests passing." The agent edits scenes through validated operations, runs the game
headless, takes screenshots to look at the result and runs the tests. Other agents (Codex, Gemini
CLI, Cursor, VS Code, Claude Desktop) and the details are in
[Connect your AI](CONNECT-YOUR-AI.md).

You can also do everything yourself: the same operations run from the command line,
`marionette op <name> '<json>'`, and `marionette ops --brief` lists them.

## 5. Test it

```bash
marionette --project fox op test_run '{}'
```

Each `tests/*.test.json` runs the game with a seed and scripted input for a number of ticks and
checks events, component values and the final state hash. Same seed and input give the same
hash on every machine, so a changed hash means the game's behavior changed. After an intended
change, ask your agent to re-pin the tests (or `marionette make --refresh --path fox` for a
made game).

Screenshots without a window, for your own eyes or your agent's:

```bash
marionette screenshot scenes/main.scene.json --project fox --ticks 120 --seed 1 --size 960x540 --out shot.png
```

## 6. Add art

A made game starts with placeholder shapes. In order, the engine looks for art in your project,
your own asset library, the public CC0 pool, online CC0 libraries, and only then generates it.
To generate the placeholders with an image and 3D model provider (paid, your own key and a spend
cap you set; see [Connect your AI](CONNECT-YOUR-AI.md#3-api-keys-for-generating-art)):

```bash
cd fox
marionette generate plan          # what would be generated, with which provider, at what cost
marionette generate run           # generate (refused until a key and a spend cap are set)
marionette make --refresh --path .
```

Without keys the placeholders simply stay and the game works as it is. To try the whole
pipeline offline first, switch to the free stand-in provider, which draws simple test images:
`marionette provider use image mock` and `marionette provider use model3d mock`.

## 7. Package it for players

Packaging builds a game binary from the engine source, so it runs in a source checkout (step 1,
"From source") and takes a few minutes. From the engine folder:

```bash
marionette package path/to/fox                      # a zip for this machine
marionette package path/to/fox --target windows-x64 --target linux-x64 --target steamdeck
marionette package --list path/to/fox               # what this machine can build, and what is missing
```

The result is `dist/fox-<target>.zip`: the game binary, its files, a `README.txt` for players
and the licenses. A player unzips it and double-clicks the game. Windows packages are built on
Windows. Linux packages are held to glibc 2.31 so they run on older systems and in the Steam
runtime; building them needs zig and cargo-zigbuild (`pip install ziglang`,
`cargo install cargo-zigbuild --locked`) plus link libraries from
`scripts/package-sysroots.sh`. `--list` tells you exactly what is missing. Details:
[docs/ai/PACKAGING.md](ai/PACKAGING.md).

## Where next

- Write gameplay: TypeScript scripts (`.claude/skills/marionette-write-script`), fast parallel
  loops (`marionette-write-job`), or Rust (`marionette-add-rust-code`). Your agent finds these by
  itself; they are also good reading for people.
- Port from Unity or Unreal: `marionette-port-unity-unreal`.
- VR, multiplayer and Steam, saves, audio: the skill with that name.
- What each feature can do today, honestly rated: [docs/ai/STATUS.md](ai/STATUS.md).
