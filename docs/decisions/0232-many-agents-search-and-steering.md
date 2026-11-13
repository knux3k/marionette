# 0232: Many agents: shared searches, capped queue, parallel steering

Date: 2026-11-13. Status: accepted.

## Context

Crowds of wildlife or drones ask for many routes at once, often to the same place, and must
avoid each other every tick. The cost must stay flat when thousands move, and results must not
depend on the thread count.

## Decision

- **Route requests go through a queue** capped by `NavSettings.searches_per_tick` (oldest
  first), so a burst of requests spreads over ticks instead of spiking one.
- **Agents with the same goal cell share one search**: a Dijkstra from the goal until all their
  start cells are settled; a lone agent uses A* with the straight-line estimate weighted 110%
  (routes at most 10% dearer than the best, far fewer cells expanded). Unreached goals give a
  partial route to the closest place (status `Partial`). Searches run on the job pool, one job
  per goal, with per-thread scratch memory reset by generation stamps.
- **Routes are string-pulled** by galloping line-of-sight checks (doubling then halving), so a
  route is a few corners and smoothing costs about log n checks per corner.
- **Steering per agent, in parallel chunks**: seek the next corner, slow down to arrive,
  predict the closest approach to each neighbour within `horizon` seconds and sidestep, push
  apart when overlapping. Neighbours come from agents sorted by spatial bucket with an
  open-addressing table (looked up, never iterated). Each agent writes only its own result;
  results are applied in id order.
- **`Direct` agents** are moved along the ground by navigation (slide along blocked cells, walk
  back to the cell centre when cornered, plan again after 45 ticks without progress);
  **`Intent` agents** write `MoveIntent` for a physics character.
- **`Jobs::map_chunks_heavy`**: parallel whenever there is more than one chunk, for work where
  each item costs microseconds (the 65,536-item threshold of `map_chunks` is for cheap items).

## Alternatives

- A full flow field per goal kept in a cache: costs memory per goal and a full-grid pass even
  for nearby agents; the bounded shared search gives the same routes for the agents that ask.
- ORCA (reciprocal velocity obstacles with linear programs): smoother in dense crowds, several
  times the cost; can replace the steering function later without changing components.

## Consequences

- 2,000 agents crossing a planet: 1.4 ms per tick on a 4-core cloud machine (single thread
  2.8 ms); proofs in `docs/ai/proofs/navigation.md`.
- Dense crowds can jostle; avoidance is local and does not plan around jams.
