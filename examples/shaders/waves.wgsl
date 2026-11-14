// Game shader example (decision 0181, docs/ai/SHADERS.md): water whose vertices move with
// two sine waves over game time (shadows follow, because the depth pass runs `vertex` too).
// params[0] = (amplitude m, wavelength m, speed m/s, roughness); params[1].rgb = color.
fn vertex(v: ptr<function, Vertex>) {
    let p = game.params[0];
    let k = 6.2831853 / p.y;
    let phase = ((*v).position.x + (*v).position.z * 0.6) * k - (*v).time * p.z * k;
    (*v).position.y = (*v).position.y + p.x * sin(phase);
    // Normal of the displaced surface (the derivative of the height along x and z).
    let slope = p.x * k * cos(phase);
    (*v).normal = vec3<f32>(-slope, 1.0, -slope * 0.6);
}

fn surface(s: ptr<function, Surface>, i: SurfaceIn) {
    (*s).base_color = vec4<f32>(game.params[1].rgb, 1.0);
    (*s).roughness = game.params[0].w;
    (*s).metallic = 0.0;
}
