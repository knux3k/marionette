// Game shader example (decision 0181, docs/ai/SHADERS.md): an additive, unlit hologram.
// Color from params 0..2, brightness params[3]; scan lines move up with game time; edges
// facing away from the camera glow more.
// @blend additive
// @double_sided
fn surface(s: ptr<function, Surface>, i: SurfaceIn) {
    let p = game.params[0];
    let view = normalize(i.eye - i.world_pos);
    let rim = pow(1.0 - abs(dot(view, i.normal)), 2.0);
    let scan = 0.6 + 0.4 * step(0.5, fract(i.world_pos.y * 12.0 - i.time * 0.5));
    (*s).base_color = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    (*s).emissive = p.rgb * p.w * (0.25 + rim) * scan;
    (*s).unlit = 1.0;
}
