# 0233: Gravity zones and hover packs

Date: 2026-11-15. Status: accepted. Focus 1 (modularity), decision 0060; extends 0230.

## Context

On small worlds a game often treats the ground near a landing site as flat: within some
distance of the site, up is straight up and falling uses a fixed acceleration, so buildings
and the player stand upright and line up with each other; further out, gravity points at the
centre. Point wells alone tilt a character by several degrees at the edge of such an area.
Separately, a jetpack that eases up to a steady climb speed with gravity off (a hover pack)
could not be expressed: `PlanetCharacter.thrust` only adds acceleration on top of gravity.

## Options

1. Game code overrides the character's vertical speed and rotation every tick. Fights the
   engine's step and repeats its maths.
2. A `GravityZone` component (uniform gravity inside a sphere, wells ignored there) and a
   `climb_speed` field on `PlanetCharacter` (0 keeps the old meaning).
3. A general gravity function hook. More power than any current game needs; not data.

## Decision

Option 2. Zones are checked in scene id order and the first one containing the point wins;
fluids still apply inside a zone. With `climb_speed > 0`, holding the thrust action turns
gravity off and moves the vertical speed toward `climb_speed` by `thrust` m/s².

## Consequences

- A flat landing area is one entity with a `GravityZone`; characters turn upright inside it
  at their `align_speed`. A zone has no hysteresis at its edge.
- Scenes must give `climb_speed` (scenes list every field); existing scenes use 0.
- Tests: `a_zone_overrides_wells_inside_it_only`,
  `a_hover_pack_eases_to_its_climb_speed_without_gravity` (marionette-physics).
