# Game shaders

How a game draws a model with its own shader code (decision 0181). Read only for that task.

## Use

1. Write WGSL with a `surface` function, a `vertex` function, or both (contract below).
2. Put it in a `.wgsl` file next to the scene, or store it from Rust:
   `GameShaders::set_in(world, "water", SOURCE)` (`marionette_core::shader`).
3. Give the entity `ShaderMaterial { shader, params, textures }` next to its `Model`:
   `shader` is the stored name or the file path (relative to the scene file); `params` up to 16
   numbers; `textures` up to 4 images (PNG paths or `builtin:` images). `Material`, if
   present, still sets the starting color, metallic and roughness. Bulk draws take
   `BulkDraw::shader`.
4. Errors (unknown name, compile error, bad option) are `render.shader` diagnostics; the
   message gives the line where the game source starts in the composed shader.

Examples: `examples/shaders/*.wgsl`, scene `examples/game-shader.scene.json`.

## Contract

The engine runs its standard material first, then calls `surface` to change the result, then
lights it. So a shader sets only what it changes, and lights, shadows, environment, fog,
tone mapping and bloom still apply.

```wgsl
fn surface(s: ptr<function, Surface>, i: SurfaceIn) { ... }   // optional
fn vertex(v: ptr<function, Vertex>) { ... }                    // optional
```

| Type | Fields |
| --- | --- |
| `Surface` (read and write) | `base_color: vec4` (linear rgb, alpha), `metallic: f32`, `roughness: f32`, `normal: vec3` (world, normalized, facing the viewer), `emissive: vec3` (added after lighting), `unlit: f32` (> 0.5: result is `base_color.rgb + emissive`) |
| `SurfaceIn` | `world_pos: vec3`, `normal: vec3` (geometric), `uv: vec2`, `color: vec4` (vertex color, white if none), `eye: vec3`, `time: f32` (game seconds) |
| `Vertex` (read and write, world space) | `position: vec3`, `normal: vec3` (need not be normalized), `uv`, `color`, `base_color`, `params` (instance values), `time: f32` |

Bindings available to both functions: `game.params[k]` (`vec4`, `k` 0 to 3: material numbers
`4k` to `4k+3`), `tex0` to `tex3` (`texture_2d<f32>`, white when not given, sampled as sRGB
color), `game_samp` (linear, repeating). Use `textureSampleLevel` in `vertex`. Everything in
`crates/marionette-render/src/shaders/pbr.wgsl` is visible too (for example `PI`, `view`).

Options are comment lines anywhere in the source:

| Line | Effect |
| --- | --- |
| `// @blend alpha` | mixed by `base_color.a`, drawn after opaque models, no depth write, casts no shadow |
| `// @blend additive` | added to what is behind (glow, holograms), same rules as alpha |
| `// @double_sided` | back faces drawn too |
| `// @no_shadows` | an opaque shader that casts no shadow |

A `vertex` function also runs in shadow passes, so moved vertices cast moved shadows.

## Limits (v1)

- Skinned or morphed parts keep the standard shader.
- Blended draws are not sorted back to front between objects.
- Alpha-cutout (discarding pixels) does not cut shadows.
- No full-screen or sky shaders yet; textures are 8-bit color images.
