# 0182: `// @no_fog` game shader option

Date: 2026-11-15. Status: accepted. Focuses 1 (modularity, custom code) and 4 (graphics),
decision 0060.

## Context

A game that paints its own sky (a gradient dome per world, a sun disc, bands of colour) draws it
as a large inverted mesh with an unlit game shader (decision 0181). Distance fog is applied to
every model, so a dome a few kilometres away comes out as flat fog colour on any world with fog.
The engine has no sky shader yet.

## Options

1. A full sky shader hook (a game `sky(direction)` function replacing the environment lookup in
   the background pass). Most general, but a new pipeline, bindings and stereo variant.
2. A per-pixel `fog` field in `Surface`. Changes the struct every shader builds.
3. A `// @no_fog` comment option, like `// @no_shadows`: the composed shader multiplies the fog
   distance by a constant 0.

## Decision

Option 3. The hook block now also defines `const GAME_FOG: f32` (1 by default, 0 with
`// @no_fog`), and both `finish` calls in `fs_main` multiply the fog distance by it. Tone
mapping still applies. Standard models and every existing shader compose to the same result
(the constant is 1). A full sky hook (option 1) stays open as a gap.

## Consequences

- Sky domes and far backdrops keep their own horizon colour under fog.
- No new bindings, pipelines or per-frame cost.
- Test: `no_fog_shaders_keep_their_color_in_thick_fog` (marionette-render), option parse in
  `options_come_from_comment_lines`.
