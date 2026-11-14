# 0107: Draw groups are looked up by model address with a small hash

Date: 2026-11-14. Status: accepted. Part of the optimization pass (decision 0100), focus 3.

## Context

Recording a frame puts every visible mesh instance into a draw group (model, part, material
source, lamp, level of detail, game shader slot, look). The lookup was a `BTreeMap` keyed by
the asset name: a tree walk of string compares for every instance whose key differs from the
previous one, which is most of them when entities are not sorted by model. Profiling the
realistic benchmark scene put about 17 percent of recording time in that search.

## Decision

The lookup is a `HashMap` keyed by the model's address (the renderer holds one `GpuModel` per
asset name) plus the small fields, with `FastHasher`, a multiply-rotate hash (the one rustc
uses) instead of SipHash: the keys are made by the engine, so the attack resistance SipHash
buys is not needed. Each group's full key is kept in a list and groups are sorted by it after
the loop, so draw order is the same as with the tree and the same on every run.

## Proof

Realistic scene (3,619 entities, 24,897 instances), cloud container, lavapipe, two alternated
runs of 20 frames: median record 10.01 / 10.60 ms before, 9.04 / 9.70 ms after (about 9
percent less). Render tests pass with the same reference images.

## Rejected

- Hashing the asset string (`HashMap<&str, ..>`): hashes 20 to 40 bytes per lookup; tried
  first, no clear gain in the cloud.
- A crate for the hasher: twenty lines do it.
