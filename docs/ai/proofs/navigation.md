# Proofs: round worlds and navigation

## Movement on round worlds (decision 0230)

- `cargo test -p marionette-physics --test planet`: a `PlanetCharacter` walks 40 s round a
  20 m planet, staying 0.8 m above the ground (within 0.15 m) with up radial (cosine above
  0.999) every tick and reaching the far side; a dynamic crate falls and rests on the planet;
  a character on a moon with a ranged well stands on the moon; runs repeat bit for bit.
- `cargo test -p marionette-physics --lib planet`: gravity falls with distance and is linear
  inside, `range` cuts a well off, quaternion helpers, jetpack thrust and jumps, a neutral
  fluid floats and damps, up turns at `align_speed`.

## Navigation (decisions 0231, 0232)

- `cargo test -p marionette-nav`: grid cells map back to themselves with at least 7 symmetric
  neighbours across face seams; edge sizes within 1.8×; A* round a wall within 10% of the
  shared Dijkstra's best cost; partial routes; routes cross faces.
- `crates/marionette-nav/tests/nav.rs`: all 27 agents of `examples/planet.scene.json` (24
  ground agents round 12 rocks, 3 physics characters through `MoveIntent`) arrive within 4000
  ticks; an obstacle dropped on one agent's route makes it go round (and never closer than its
  radius) while a far agent keeps its route; the 2,000-agent crowd hashes the same on 1 and 4
  threads; a snapshot restored at tick 200 matches the uninterrupted run at tick 500.
- Stability: modes `planet-3d` (600 and 216,000 ticks: p99 0.1 ms, memory flat) and
  `planet-crowd` (600 ticks: p99 2.0 ms; 108,000 ticks: p99 3.1 ms, counted heap +4.5 MiB flat
  after load).

## Crowd cost (cloud, 4 cores, release)

`marionette op perf_sim '{"scene":"examples/planet-crowd.scene.json","ticks":600}'`: 2,000
agents crossing a 40 m planet on a 96-resolution grid (55k cells) with 12 obstacles.

| Change | 1 thread avg | 4 threads avg |
| --- | --- | --- |
| first version | 3.8 ms | 2.7 ms |
| runtime in id-ordered arrays, aim and move phases on the job pool | 3.3 ms | 1.6 ms |
| weighted A*, galloping smoothing, hashed neighbour buckets | 3.0 ms | 1.4 ms |

Not yet checked: the same scene on the 8-core desktop, and a side-by-side against the Unity
navigation the ported game uses.
