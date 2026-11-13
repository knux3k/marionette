# 0140: Game packages per target with one command; Linux held to the Steam runtime floor

Status: accepted (2026-11-13)

## Context

The owner asked for one-command game builds that can be zipped and handed to someone, for
Windows and Linux, plus Steam Deck, plus ARM64 builds for Valve's Steam Frame (no Quest).
Valve's Steam Frame pages (2026-11-13) say the Frame runs SteamOS on a Snapdragon 8 Gen 3
(ARM64), Vulkan is native, and native Linux ARM64 builds run in Steam Linux Runtime 3.0 ARM64
(sniper). Sniper ships glibc 2.31; the Deck's runtime is the x86-64 sniper.

## Decision

- New crate `marionette-package` (operations `package_targets`, `package_build`, CLI
  `marionette package`, cargo feature `package`, on by default). Targets: `windows-x64`,
  `linux-x64`, `steamdeck` (the same binary as `linux-x64` with Deck notes and a 1280x800
  window), `steam-frame` (aarch64 Linux).
- **glibc 2.31 floor** for every Linux package, checked on the built binary. Builds go through
  `cargo zigbuild` (alternatives: building inside Valve's sniper SDK containers needs Docker on
  every build machine; a plain build on a newer distribution would need glibc 2.34+ and fail in
  the runtime). Only ALSA is linked from the system; its link-time copy comes from Ubuntu 20.04
  packages fetched by `scripts/package-sysroots.sh`.
- Windows packages are built on Windows (CI job `package`); cross-building MSVC binaries from
  Linux was not tried.
- **The binary is the launcher**: started with no arguments it plays the entry scene from
  `game.package.json` beside it; no separate launcher program or script. The manifest reader
  lives in `marionette-cli` without the packaging crate so lean game binaries stay lean.
- **Smallest useful package**: the engine binary gets only the runtime features the game uses;
  editor tools (import, pool, generation, porting, packaging) are never in it; unreferenced
  assets are left out and listed.
- **Reproducible zips** written by a small built-in writer (fixed timestamps, sorted entries,
  Unix modes kept), so the same folder always gives the same bytes.
- Licenses travel with the game: engine license files plus every linked crate's license text.

## Consequences

- All six proof games package for Linux x64 and Steam Frame; on ARM64 (emulated) their state
  hashes equal x86-64. Proofs: `docs/ai/proofs/packaging.md`.
- Steam Frame packages play flat until an OpenXR module exists; Deck and Frame controls need a
  Steam Input keyboard layout until a gamepad backend exists.
- Focus check (FOCUSES.md): modularity (own crate behind a feature; games with their own Rust
  binary package the same way), tokens (one command, compact results, PACKAGING.md read only
  for this task), performance (release builds, smaller binaries: 46 MB to 34 MB without job
  TypeScript), graphics unchanged.
