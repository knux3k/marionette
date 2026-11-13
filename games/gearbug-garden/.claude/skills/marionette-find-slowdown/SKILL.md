---
name: marionette-find-slowdown
description: Measure and fix slow ticks, slow frames, hitches and memory growth in a Marionette game with the engine's own timing tools. Use when a game misses its frame budget, stutters, hitches on spawn or load, grows in memory, needs to hold 90 Hz in VR, or before claiming any performance number.
---

# Find a slowdown

Measure first, with a release build, and say which machine and GPU the number came from. A
debug build or a software GPU gives timings that mean nothing for a budget.

Budget to hold in VR: 11.1 ms per frame at 90 Hz for simulation, render CPU and GPU together,
both eyes.

## Split the frame

1. Simulation only:
   `marionette op perf_sim '{"scene": "scenes/main.scene.json", "ticks": 1200}' --json`
   gives `avg_ms`, `p50_ms`, `p99_ms`, `max_ms` per tick and `load_ms`.
2. Whole frame, no window: `marionette bench <scene> --frames 300` reports phase medians
   (simulation, extract, record, GPU wait) and draw calls, instances and triangles. Add
   `--headset` for two eyes at headset resolution.
3. A live session: `observe_stats` after `session_step` (step time, heap bytes).

Read p99 and max, not only the average: one stalled frame is a visible hitch.

## Then fix the largest part

| Symptom | Likely cause | Do |
| --- | --- | --- |
| High tick time, grows with entity count | a script looping over many items | move the loop to a job (skill `marionette-write-job`) |
| High tick time in one script | repeated `engine.query` or `engine.find` every tick | keep ids in `self.state` |
| Tick time rises over a run | entities or events never removed | compare entity counts at start and end; destroy what is done |
| Draw calls grow with object count | many different models or materials | reuse one model and material so copies batch into one instanced draw; many copies of one thing: `JobDraw` |
| GPU wait dominates | resolution, shadows, bloom, shadow-casting local lights, far detail | skill `marionette-light-scene`, section Cost; skill `marionette-tune-render-cost` |
| One long frame when something first appears | a large model uploaded on first use (the window loop already decodes late models in the background) | slim the model: fewer triangles, smaller textures |
| Slow save | building the whole scene text on the tick | save from a copy off the tick (a Rust module can; skill `marionette-add-rust-code`) |
| Result changes with thread count | a job writing outside its chunk | fix the job; run `perf_sim` with `threads` 1 and 4 and compare hashes |

Change one thing, measure again with the same seed and ticks, keep the number.

## Memory and long runs

`marionette stability --mode <name> --long` checks memory growth after warm-up, hangs and
time budgets over a long seeded run (skill `marionette-test-game`).

## Checks

Report before and after with: build type, machine class, GPU, scene, ticks, p99 and max.
Never compare against another engine unless both ran the same content on the same machine.
