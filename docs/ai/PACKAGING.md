# Packaging games

Normative for game packages (decision 0140). Read when you package a game, add a target, or
change what a package holds.

## One command

```bash
marionette package games/cloudstep                       # this machine's own target
marionette package games/cloudstep --target steam-frame  # repeat --target, or --target all
marionette package --list games/cloudstep                # targets this machine can build, files a package holds
```

Same as operations `package_build` and `package_targets` (MCP group `package`). Output:
`dist/<game>-<target>/` and `dist/<game>-<target>.zip` (folder name inside the zip). Flags:
`--out`, `--no-zip`, `--no-smoke`, `--debug`.

| Target | Rust target | Built with | Runs in |
| --- | --- | --- | --- |
| `windows-x64` | `x86_64-pc-windows-msvc` | cargo, on Windows only | Windows 10/11 |
| `linux-x64` | `x86_64-unknown-linux-gnu` | cargo zigbuild (glibc 2.31) | any Linux with glibc >= 2.31 and Vulkan |
| `steamdeck` | `x86_64-unknown-linux-gnu` | same binary as `linux-x64` | Steam Linux Runtime 3.0 (sniper) |
| `steam-frame` | `aarch64-unknown-linux-gnu` | cargo zigbuild (glibc 2.31) | Steam Linux Runtime 3.0 ARM64 (sniper) |

## What a package holds

- The binary, named after the game (`marionette.json` `name`). With no arguments it reads
  `game.package.json` beside itself and opens a window on the entry scene; with arguments it is
  the full CLI (`./cloudstep run scenes/main.scene.json --headless`). On Windows a
  double-clicked game drops its console window; start errors show in a message box.
- The game's runtime files, same layout as the game folder. Left out: `tests/`,
  `build.ops.json`, `providers.json`, `README.md`, `*.recipe.json`, `*.approval.json`, dot
  files, a Rust game's `Cargo.toml`/`src/`, and any file under `assets/` whose name no scene,
  map or script names (listed in `left_out`; `package.keep_unreferenced` keeps them).
- `game.package.json`: entry, title, window, target, features, newest glibc needed, every
  file's size and SHA-256.
- `licenses/`: the engine's MIT and Apache-2.0 files, and `THIRD-PARTY.txt` listing every
  linked crate with its license and license files (identical texts once).
- `README.txt` for the player, with Steam Deck and Steam Frame steps on those targets.

VR games (a scene with `XrRig`, decision 0190) get `"vr": true` in the manifest: the binary
tries the headset through OpenXR first and opens a window when no runtime or headset answers
(`xr.no_*`). Linux and the Frame use the system OpenXR loader; on Windows put
`openxr_loader.dll` in the game folder (the build warns when it is missing).

The engine binary gets `gamepad` (decision 0172) and only the other runtime features the game uses: `script` when it has
TypeScript, `jobs-ts` when text names `JobData`, `JobSchedule` or `engine.jobs` (wasmtime is
about 12 MB), `xr` when a scene has an `XrRig`. A game with its own `Cargo.toml` (decision 0062) is built from that package.

Game settings in `marionette.json` (all optional):

```json
"package": { "entry": "scenes/main.scene.json", "title": "Cloudstep", "window": "1280x800",
             "exclude": ["assets/wip/**"], "features": ["script"], "keep_unreferenced": false }
```

## Build machine setup

- Windows target: Rust (MSVC) on Windows. A Linux machine cannot build it.
- Linux targets: `pip install ziglang`, `cargo install cargo-zigbuild --locked`,
  `rustup target add x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu`, then
  `scripts/package-sysroots.sh target/sysroots` and export the two variables it prints
  (`MARIONETTE_SYSROOT_X86_64`, `MARIONETTE_SYSROOT_AARCH64`). The sysroots hold the ALSA
  and udev (gamepad) libraries of Ubuntu 20.04 (glibc 2.31) so the binary never links the build machine's newer one.
- A Linux build machine whose own glibc is 2.31 or older builds its own architecture with plain
  cargo and needs neither.

Builds use the workspace's cargo profile `dist` (fat LTO, decision 0101) when `Cargo.toml` defines
`[profile.dist]`, else `--release`; `--debug` builds debug.

Checks every package run makes: the Linux binary's newest `GLIBC_` symbol version must be <= 2.31
(`package.glibc`), and when the binary can run on the build machine it runs the entry scene
headless for 60 ticks (`package.smoke`).

## Steam Frame facts (Valve's Steamworks pages, read 2026-11-13)

- SteamOS on a Snapdragon 8 Gen 3 (ARM64); Vulkan is the native graphics API; x86 games run
  through FEX, Windows games through Proton plus FEX, Android through a container layer.
- Native Linux ARM64 builds use runtime "Steam Linux Runtime 3.0 ARM64 (sniper)", chosen in the
  SteamOS Devkit Client upload (fields: Local Folder, Start Command, Runtime).
- VR: OpenXR recommended; Frame Verified needs at least 72 fps at 1728x1728 per eye in normal
  play and support for the Frame controllers.
- VR games start in the headset through OpenXR; other games play as a flat window.

## Gaps

- Gamepads never tested on a Deck or Frame: Steam Input may present the built-in controls as a
  virtual pad; keyboard-and-mouse layouts also work.
- VR packages never ran on a real headset or in the Frame's runtime.
- Ray tracing is not tested on ARM64: Mesa's llvmpipe crashes in its JIT with ray queries
  on, so the ARM64 CI job sets `MARIONETTE_RAY_TRACING=0`. The Frame's own GPU is unchecked.
- No code signing (Windows SmartScreen warns on first start), no macOS target, no Steam depot
  upload (use SteamPipe with the zip contents).
