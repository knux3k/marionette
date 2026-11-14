// Game shader example (decision 0181, docs/ai/SHADERS.md): tints the model by params 0..2
// and tiles texture 0 params[3] times across the UVs. Lighting, shadows and fog stay the
// engine's.
fn surface(s: ptr<function, Surface>, i: SurfaceIn) {
    let p = game.params[0];
    let tile = textureSample(tex0, game_samp, i.uv * p.w);
    (*s).base_color = vec4<f32>(p.rgb * tile.rgb, 1.0);
}
