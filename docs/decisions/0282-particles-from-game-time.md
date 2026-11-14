# 0282: Particles computed on the GPU from game time alone (`ParticleEmitter`)

Date: 2026-11-14. Status: accepted. Focuses 1 (modularity), 2 (tokens), 3 (performance) and 4
(graphics), decision 0060.

## Context

The engine had no particles (audit 2026-11-14). Games need fire, sparks, smoke, dust and magic.
The engine's rules make simulated particles costly: every piece of state is hashed, saved and
replayed, and per-tick work counts against the tick budget.

## Decision

1. **Stateless**: a `ParticleEmitter` component (rate, lifetime, speed, cone, start radius,
   gravity, sizes, colors, image, `Additive` or `Alpha` blend, seed). Particle `n` starts at
   `n / rate` seconds of game time; its start point and direction come from hashing `n` with
   the seed. Its position at any time is a closed formula (straight line plus gravity), so
   nothing is stored, simulated, saved or hashed, and the same tick always draws the same
   picture.
2. **GPU only**: one 256-byte uniform per emitter, one instanced draw of six vertices per
   particle slot; the CPU works out which particle numbers can be alive in double precision
   (`particles::span`, exact after hours of game time). At most 100,000 per emitter.
3. **Drawing**: after see-through models, depth-tested without depth writes, premultiplied
   (additive or covering). Into the bloom target they write linear light (values above 1 glow);
   into the final target they are tone mapped like models. Not in probe captures.
4. Built-in image `builtin:soft` (white dot fading to clear) for soft particles.

## Limits (kept simple on purpose)

- Particles stay attached to their emitter: moving the entity moves the particles already in
  the air. Trails left in world space need stored state and are not done.
- No collisions, no lighting of particles, no sorting within an emitter (additive does not
  need it; alpha smoke looks right at the sizes games use), no soft fade against geometry.

## Alternatives

- CPU simulation into a vertex buffer: per-particle tick cost, state to hash and save.
- GPU compute simulation: needs stored buffers whose contents differ between GPUs and must be
  rebuilt after a load; the stateless formula gives the same picture everywhere.

## Consequences

- Example `examples/particles.scene.json` (fire, sparks, smoke with bloom), reference image,
  stability mode `render-3d-particles`, tests in `crates/marionette-render/tests/particles.rs`
  (pure function of time, one draw per emitter at any rate, drawn along the emitter).
