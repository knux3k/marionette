# 0003: SplitMix64 RNG and SHA-256 state hash

Date: 2026-11-11. Status: accepted.

## Decision

- Simulation randomness comes from `SimRng`, a SplitMix64 generator seeded from `--seed`.
- The state hash is SHA-256 over the canonical scene text of the current world, plus the tick
  counter and the RNG state.

## Why

- SplitMix64 is a few lines of integer arithmetic with a fully specified output, so it is the
  same on every platform and its whole state is one `u64` that is easy to save and hash. It is
  not cryptographic; it does not need to be.
- Hashing the canonical scene text means the hash covers exactly what a save would contain,
  with one definition of "state". SHA-256 (`sha2` crate) is stable across platforms and versions.

## Consequences

Anything that should count as simulation state must be either a registered component or
resource (so it appears in the scene text) or added explicitly to `Sim::state_hash`.
