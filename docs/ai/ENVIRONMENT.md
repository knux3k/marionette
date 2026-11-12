# Environment

## Platforms and toolchain

- Windows and Linux (x86_64). macOS is not a target but may work.
- Rust stable, 1.95 or newer (`rust-version` in `Cargo.toml`); Windows uses the MSVC toolchain
  (needs the Visual Studio C++ build tools).
- No other system libraries are needed on Windows. On Linux, windowed mode needs the usual X11
  or Wayland development packages for winit, and audio playback needs ALSA development files for
  cpal; headless builds and tests need neither at run time.

## Commands

| Purpose | Command | Rough duration |
| --- | --- | --- |
| Build (debug) | `cargo build` | several minutes from clean; seconds when warm |
| All tests | `cargo test` | about 30 s warm (includes GPU tests) |
| Lint | `cargo clippy --all-targets -- -D warnings` | about 1 min warm |
| Format check | `cargo fmt --all -- --check` | seconds |
| Regenerate reference docs | `cargo run -q -p marionette-cli -- docs` | seconds |
| Check reference docs | `cargo run -q -p marionette-cli -- docs --check` | seconds |
| Frame-time benchmark | `cargo run --release -q -p marionette-cli -- bench examples/showcase.scene.json --frames 2000 --size 1920x1080` | release build several minutes |

## Keeping the build cache small

- `CARGO_INCREMENTAL=0` avoids the incremental cache.
- The dev profile already uses line tables only for workspace crates and no debug info for
  dependencies (root `Cargo.toml`).
- Use `--release` only for benchmarks; `cargo clean` at the end of a session.
- A full debug build plus tests needs a few GB of free disk space.

## Machines without a GPU

GPU tests skip with a message when no adapter exists, unless `MARIONETTE_REQUIRE_GPU=1` (CI sets
it), which turns a skip into a failure.

- Linux: install Mesa's Vulkan drivers (package `mesa-vulkan-drivers` on Debian/Ubuntu) for the
  lavapipe software rasterizer; `WGPU_BACKEND=vulkan`.
- Windows: the WARP software rasterizer ships with Windows:
  `WGPU_BACKEND=dx12 WGPU_ADAPTER_NAME="basic render"`.
- Run GPU tests with `cargo test -- --test-threads=1` on software rasterizers (WARP crashed
  with several concurrent devices).

`WGPU_BACKEND` and `WGPU_ADAPTER_NAME` select the backend and adapter (substring match).

## Repository

The engine repository has no remote yet. CI (`.github/workflows/ci.yml`) runs fmt, build, tests
(software GPU) and clippy on Windows and Linux once the repository is pushed. Do not push,
publish or create remotes without the owner's approval.
