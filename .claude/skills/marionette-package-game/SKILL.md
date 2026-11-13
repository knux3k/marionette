---
name: marionette-package-game
description: Package a Marionette game into a folder and zip that players can run, for Windows x64, Linux x64, Steam Deck or Steam Frame (Linux ARM64). Use when asked to build, ship, export, release, zip or hand out a game, make a Steam Deck or Steam Frame build, or check why a package fails to build or start.
---

# Package a game

One command builds the game's binary (cargo profile `dist` when the workspace has it, else release), copies the runtime files and licenses,
writes `game.package.json`, and zips the folder. The binary plays the entry scene when started
with no arguments.

## Steps

1. See what this machine can build and what the package will hold:
   `marionette package --list <game-folder> --json` (operation `package_targets`).
   Check `game.entry`, `game.features` and `game.left_out` (assets no scene or script names).
2. Build: `marionette package <game-folder> --target <t> --json` (operation `package_build`),
   `--target` repeatable, or `all`. Targets: `windows-x64`, `linux-x64`, `steamdeck`,
   `steam-frame`. Output: `dist/<game>-<target>/` and `dist/<game>-<target>.zip`.
3. Read each result: `smoke` is `passed` when the binary ran the entry scene headless here;
   other targets say `skipped`. `glibc_required` must be 2.31 or lower.

## Settings (optional, `marionette.json` under `package`)

`entry` (default `scenes/main.scene.json`), `title`, `window` (e.g. `1280x800`), `exclude`
(globs), `features`, `keep_unreferenced`. An asset loaded by a name built at run time is not
seen by the reference check: list it in a scene or set `keep_unreferenced`.

## Traps

- `windows-x64` builds only on Windows.
- Linux targets need zig and cargo-zigbuild plus link-time sysroots in
  `MARIONETTE_SYSROOT_X86_64` / `MARIONETTE_SYSROOT_AARCH64` (the engine repository has a
  script that makes them). Error `package.toolchain` names what is missing.
- `package.glibc`: the binary was linked against a newer glibc; build through zigbuild.
- A game with its own `Cargo.toml` is built from it and needs exactly one binary.
- Steam Frame packages play as a flat window (no VR yet). On Deck and Frame, controls need a
  Steam Input keyboard and mouse layout (no gamepad backend yet).
- Uploading to Steam, signing, or sending a package to anyone is the owner's call: ask first.
