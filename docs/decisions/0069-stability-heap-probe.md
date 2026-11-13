# 0069: Stability memory checks read the counted heap

Date: 2026-11-13. Status: accepted. Decision 0031 (stability) and 0032 (memory).

## Context

The stability memory check compared process resident size after warm-up with the end of the
run. In a 4-core cloud container `factory-sim` failed it in a debug build (33.8 MiB growth,
limit 32 MiB) while CI passed. Measured on 2026-11-13 with seed 1: resident growth was
33.8 MiB at 300 ticks but 23.6 MiB at 1,500 ticks, so it does not grow with run length; the
counted heap grew a flat 4.0 MiB at both lengths. The resident figure moves with allocator
arenas of the job worker threads and with how much freed memory the allocator keeps, not with
what the engine holds. Windows had no memory check at all (resident size was read from
`/proc/self/statm` only).

## Decision

When the binary installs `marionette_host::alloc::Counting` as its global allocator (the
`marionette` binary does, and game binaries are told to), the default stability probe is the
counted heap (`probe: "heap"`). Otherwise it stays resident size. The report keeps resident
growth as `resident_growth_mib`, for information only. Limits stay as they were (32 MiB, 64 MiB
for the four 3D proof games).

## Alternatives

- Raise the `factory-sim` limit: hides the question instead of measuring the right thing.
- Judge both figures: resident size would keep failing on arena growth that is not a leak.

## Consequences

- `factory-sim` is not a leak; its fast-suite failure in containers goes away.
- Memory checks now run on Windows too.
- GPU driver memory is outside the Rust heap and is not judged by this check; GPU objects are
  covered by `gpu_objects_come_back_after_repeated_setups_and_trims` in
  `crates/marionette-render/tests/render.rs`.
