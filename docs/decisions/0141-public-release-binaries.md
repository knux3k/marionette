# 0141: First public release: version 0.1.0, prebuilt CLI binaries, draft releases from tags

Status: accepted (2026-11-17)

## Context

The engine repository has been public, but there was no release: a new developer had to build
from source, the README described an early state, and nothing said how a person connects the AI
they already use. The owner asked for a first public release that outside developers can follow.
Publishing itself (pushing a tag, publishing a GitHub release, crates.io, announcements) stays
the owner's call (RULES.md rule 3).

## Decision

- **Version 0.1.0**, the workspace version already in `Cargo.toml`. Semantic versioning; before
  1.0 a minor version may break scenes, operations or the Rust API, and the changelog says what
  to do. `CHANGELOG.md` (Keep a Changelog) starts with 0.1.0.
- **Prebuilt binaries** of `marionette` for `windows-x64` (MSVC) and `linux-x64` (glibc 2.31
  floor through `cargo zigbuild` and the same sysroots as game packages, decision 0140), built
  with the `dist` profile (decision 0101) and default features plus `net`. Each archive holds the
  binary, README, CHANGELOG, the two human guides and `licenses/` (engine licenses and
  `THIRD-PARTY.txt` for every linked crate, made by `marionette_package::licenses::write_for`
  through the example `engine_licenses`, the same code that writes game packages' notices).
- **`net` is in the release binary.** A default source build still has no network transport
  (decision 0045). The release turns it on because the guides tell people to generate art with
  their own keys, and the gates that matter do not depend on the feature: the spend cap is zero
  until a person sets it outside every project, no operation can raise it, and image-to-3D calls
  need a person's typed approval.
- **Workflow `release.yml`.** A pull request touching the release path builds and smoke-tests
  both archives (version, drift hash, `marionette make` and its tests) as workflow artifacts. A
  `v*` tag does the same, checks the tag against `Cargo.toml` and the changelog, and opens a
  **draft** GitHub release with the archives and `SHA256SUMS`. A person publishes the draft.
- **Human docs** separate from `docs/ai`: `README.md` (rewritten for people),
  `docs/GETTING-STARTED.md` (install, make, play, change with an AI, test, art, package; every
  command run in the release checks), `docs/CONNECT-YOUR-AI.md` (coding agents through MCP on the
  person's own subscription, provider keys, caps, adding a provider), `CONTRIBUTING.md`,
  `SECURITY.md`.
- **Not in this release:** crates.io (the crates are not ready as separate libraries; names
  would be reserved for nothing), macOS, code signing, an ARM64 CLI binary (game packages for
  the Steam Frame already build), package managers.

## Alternatives

- **Release binary without `net`**: safest default, but every person who wants generation would
  have to install Rust and rebuild. Rejected; the cap and approval are the real gates.
- **Publish the release from the workflow directly**: one less click, but a tag pushed by
  mistake would publish. Rejected; drafts cost nothing.
- **A separate Python or cargo-about notice generator**: rejected, the package crate already
  writes the notices game packages ship, so both use one code path.
- **Version 1.0.0**: rejected; many features are slices (STATUS.md) and breaking changes are
  expected.

## Consequences

- Dependencies under MPL-2.0 (symphonia, triple_buffer) ship in binaries unchanged; their
  source is on crates.io and THIRD-PARTY.txt names them. No GPL-only dependency is linked
  (`r-efi` and `self_cell` are dual-licensed and used under MIT or Apache-2.0).
- The getting-started commands are exercised by the release smoke test, so a change that breaks
  them fails the release workflow.
- Packaging a game still needs a source checkout and Rust, since it builds a binary per game.
