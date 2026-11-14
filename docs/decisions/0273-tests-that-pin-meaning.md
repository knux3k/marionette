# 0273: Tests that pin what a value means, and normal maps the right way round

Date: 2026-11-14. Status: accepted.

## Context

An audit broke the code on purpose in ten places and ran the tests. Four breaks passed
unnoticed: integer subtraction compiled as addition in job TypeScript, a normal map's green
channel flipped, metallic and roughness read from each other's channel, and a hand that
grabbed from twice its radius. The tests in those areas compared a result with an earlier
result (a hash, a reference image) from scenes that never used the part in question, so they
could not tell right from wrong.

## Decision

Add tests that state the meaning of a value and check it on the result:

- Job TypeScript (`every_operator_matches_the_interpreter`): every operator the compiler
  accepts, on integers and on numbers, over pairs of values that include the limits, compiled
  and run in a JavaScript engine, compared element by element.
- Material textures (`crates/marionette-render/tests/material_textures.rs`): a model written
  by the test with a normal map of four known tilts and a metallic-roughness texture of two
  known halves; the test compares the brightness of the quarters under lights from two sides.
- XR (`a_hand_grabs_only_within_its_grab_radius`): blocks 0.12 m and 0.145 m from a hand that
  reaches 0.15 m are lifted, blocks 0.16 m, 0.2 m and 0.29 m away are not.
- Physics (`crates/marionette-physics/tests/behaviour.rs`): fall distance and gravity scale,
  bounce height from restitution, a box held or sliding on a slope by friction, sensors that
  report without blocking, kinematic bodies, locked rotation.

Each was checked the way the audit checked: 27 planted faults in the code under these tests
(operators swapped, channels swapped, signs flipped, a value ignored or scaled) now fail.

## A fault the new test found

The normal map test failed on the unchanged renderer: normal maps were applied inverted on
both axes, so a bump was lit like a dent. The tangent frame is built from screen derivatives,
and the two cross products it uses assume the derivatives and the normal are right-handed;
the framebuffer's y points down, so they are not. `perturb_normal` now multiplies by the sign
of the determinant, which is also right for back faces. One reference image changed:
`materials.png` (decision 0281), the only scene with a normal map, was recorded with the bumps
inverted and is rewritten. A reference image holds what was drawn, not what is right.

## Not done yet

- The view direction of an orthographic camera is taken from the camera's position, as for a
  perspective camera, so highlights on smooth surfaces vary across an orthographic picture.
- Mutation checks are run by hand; there is no mutation step in CI.
