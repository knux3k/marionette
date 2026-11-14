//! Writes the textured material test models to `examples/assets/` (decision 0280):
//! `material-ball.glb` (base color, metallic-roughness and normal textures), `fence.glb`
//! (double-sided cut-out) and `glass.glb` (blended). Made entirely in code; CC0 1.0.
//!
//! Run from the repository root: `cargo run -p marionette-render --example make_materials`

#[path = "../tests/support/materials.rs"]
mod materials;

fn main() {
    std::fs::create_dir_all("examples/assets").expect("create examples/assets");
    for (name, bytes) in [
        ("material-ball", materials::material_ball(false, false)),
        ("fence", materials::fence()),
        ("glass", materials::glass()),
    ] {
        let path = format!("examples/assets/{name}.glb");
        std::fs::write(&path, &bytes).expect("write model");
        println!("wrote {path} ({} bytes)", bytes.len());
    }
}
