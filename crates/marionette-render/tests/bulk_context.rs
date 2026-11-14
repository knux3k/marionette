//! Bulk draw hooks see the frame: smooth-motion alpha and where the camera is (no GPU needed).

use bevy_ecs::world::World;
use marionette_core::spatial::{BulkDraw, BulkDrawContext, BulkDrawHooks};
use marionette_core::{Sim, default_registry};
use marionette_render::extract_at;
use std::sync::Mutex;

static SEEN: Mutex<Option<BulkDrawContext>> = Mutex::new(None);

fn record(_world: &World, frame: &BulkDrawContext, _out: &mut Vec<BulkDraw>) {
    *SEEN.lock().unwrap() = Some(*frame);
}

const SCENE: &str = r#"{"format":"marionette.scene","version":1,"resources":{},"entities":[
 {"id":1,"name":"camera","components":{
  "Camera":{"clear_color":{"r":0,"g":0,"b":0,"a":1},"projection":{"Perspective":{"far":100.0,"fov_y_degrees":40.0,"near":0.1}}},
  "LookAt":{"target":{"x":0.0,"y":1.6,"z":0.0},"up":{"x":0.0,"y":1.0,"z":0.0}},
  "Transform":{"rotation":{"w":1.0,"x":0.0,"y":0.0,"z":0.0},"scale":{"x":1.0,"y":1.0,"z":1.0},"translation":{"x":3.0,"y":1.6,"z":0.0}}}}]}"#;

#[test]
fn hooks_get_alpha_eye_and_forward() {
    let mut sim = Sim::from_text(SCENE, None, default_registry(), 1).unwrap();
    BulkDrawHooks::add(&mut sim.world, record);
    extract_at(&sim.world, 0.25);
    let seen = SEEN.lock().unwrap().expect("hook ran");
    assert_eq!(seen.alpha, 0.25);
    let eye = seen.eye.expect("camera eye");
    assert!(
        (eye[0] - 3.0).abs() < 1e-5 && (eye[1] - 1.6).abs() < 1e-5 && eye[2].abs() < 1e-5,
        "{eye:?}"
    );
    // Looks at the origin column along -X.
    let f = seen.forward.expect("camera forward");
    assert!(
        (f[0] + 1.0).abs() < 1e-4 && f[1].abs() < 1e-4 && f[2].abs() < 1e-4,
        "{f:?}"
    );
}
