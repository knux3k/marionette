# 0184: Images made in game code (`GameImages`, `game:<name>` images)

Date: 2026-11-16. Status: accepted. Focuses 1 (modularity, custom code), 3 (performance) and
4 (graphics), decision 0060.

## Context

Images came from PNG files, built-ins, or `generated:<key>` (the world UI's font atlases),
and every one was uploaded once and cached for good. A game that paints a picture every frame
(a screen inside the world showing a game, a minimap, a painted canvas, a camera feed drawn on
the CPU) had no way to show it: storing a new `generated:` image under the same key was never
picked up, and a new key per frame would allocate a texture and bind group every frame.

## Decision

1. **Store**: resource `marionette_core::image::GameImages`, name to `Arc<GameImage>` plus a
   version (as `GameMeshes`, decision 0180). `GameImage` is RGBA8, sRGB, row 0 at the top,
   1 to 2048 pixels a side, and a `nearest` flag. `set` checks sizes and returns the problem
   as text; `update(name, |pixels| ...)` rewrites the pixels of a stored image in place
   (copy on write only while a frame still holds the old copy).
2. **Drawing**: the image name `game:<name>` works wherever an image name goes: `Sprite`,
   `UiImage`, world quads, particles and `ShaderMaterial::textures`. Unknown names are errors
   as data (`render.asset`), and a name that failed is tried again once the game stores it.
3. **Updates**: the renderer keeps one GPU texture per game image and writes each new version
   into it (`queue.write_texture`) when the size and `nearest` flag are unchanged, so the
   bind groups that use it stay valid and nothing is allocated per frame. A new size, a new
   flag or `remove` drops the texture and every cached bind group that bound it.
4. **Sampling**: sprites, UI and world quads sample a `nearest` image with the nearest
   sampler (pixel art stays sharp). A `ShaderMaterial` shares one linear sampler, so a
   pixel-exact game shader reads the image with `textureLoad`.
5. **Not simulation state**: render data only, not in scenes, snapshots or the state hash.

## Alternatives

- **Re-read `generated:` images every frame**: a mutex and a full copy per image per frame,
  and the registry holds only eight; rejected.
- **A component holding the pixels**: megabytes in scene text and the state hash; rejected.
- **Mipmaps for game images**: a per-frame mip build costs more than the image is usually
  worth for a screen seen up close; left out (one level). A game wanting them can store a
  smaller image.

## Proof

`crates/marionette-render/tests/game_image.rs` (a sprite: stored, rewritten in place with
nothing new cached, resized, removed) and `game_shader.rs`
`a_game_image_binds_as_a_shader_texture_and_follows_updates`, on Mesa lavapipe. Core unit
test `image::tests`.
