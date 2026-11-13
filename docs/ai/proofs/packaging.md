# Proofs: packaging (decision 0140)

| What | How to rerun | Result (2026-11-13, x86-64 Linux container) |
| --- | --- | --- |
| Package plan of every proof game: entry kept, tests and recipes out, named assets kept, features detected | `cargo test -p marionette-package` | passes |
| Reproducible zips | `zips_are_reproducible_and_list_every_file` | passes |
| All six proof games packaged for `linux-x64` and `steam-frame` | `marionette package games/<g> --target linux-x64 --target steam-frame --out target/packages` with sysroots | 12 packages; zips 12.6 to 16.0 MB; x64 smoke runs pass; every binary needs glibc 2.30 at most |
| Cloudstep for `linux-x64`, `steamdeck`, `steam-frame` in one call | same, three targets | three packages, one x64 build reused |
| ARM64 determinism | packaged `steam-frame` binaries under `qemu-aarch64`, `run scenes/main.scene.json --headless --ticks 300 --seed 1` | cloudstep `e39f2532...7b56` and ember-run `8a559377...6097`, equal to x86-64; drift hash `7441258f...6558` equal |
| No-argument start plays the entry scene | `./cloudstep` in a package without a display | fails at window creation (`render.surface`, no display), so the manifest path is taken |
| Real ARM64 machine (GitHub Ubuntu 24.04 ARM64 runner) | CI workflow `package`, job `frame-on-arm64`: the cross-built steam-frame zip, `run scenes/main.scene.json --headless --ticks 600 --seed 1` | cloudstep `331d72c7...d642`, equal to x86-64 |
| Full test suite on ARM64, GPU tests on lavapipe | job `test-arm64` | passes; drift hash equal |
| Windows package | job `windows` | builds, smoke run passes |

## Gamepads in packages (2026-11-13)

After windowed play (decision 0172) landed, packages carry the `gamepad` feature. Linux sysroots
hold Ubuntu 20.04 libudev 245. Cloudstep packaged for linux-x64 (smoke passed, glibc 2.30 needed)
and steam-frame (glibc 2.30) from the cloud container. No gamepad was connected.
