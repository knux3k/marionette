//! Decision 0320: `Hidden` stops an entity being drawn (models, sprites, lights) without
//! removing it; taking it off brings it back.

use marionette_core::spatial::Hidden;
use marionette_core::{Sim, default_registry};
use marionette_render::extract;

const SCENE: &str = r#"{"format": "marionette.scene", "version": 1, "resources": {},
  "entities": [
    {"id": 1, "name": "box", "components": {"Model": {"asset": "primitive:cube"},
      "Transform": {"translation": {"x": 0, "y": 0, "z": 0}, "rotation": {"x": 0, "y": 0, "z": 0, "w": 1}, "scale": {"x": 1, "y": 1, "z": 1}}}},
    {"id": 2, "name": "lamp", "components": {"PointLight": {"color": {"r": 1, "g": 1, "b": 1, "a": 1}, "intensity": 5.0, "range": 4.0},
      "Transform": {"translation": {"x": 0, "y": 2, "z": 0}, "rotation": {"x": 0, "y": 0, "z": 0, "w": 1}, "scale": {"x": 1, "y": 1, "z": 1}}}},
    {"id": 3, "name": "hidden box", "components": {"Model": {"asset": "primitive:sphere"}, "Hidden": {},
      "Transform": {"translation": {"x": 2, "y": 0, "z": 0}, "rotation": {"x": 0, "y": 0, "z": 0, "w": 1}, "scale": {"x": 1, "y": 1, "z": 1}}}}
  ]}"#;

#[test]
fn hidden_entities_are_not_drawn_until_shown_again() {
    let mut sim = Sim::from_text(SCENE, None, default_registry(), 1).unwrap();
    let f = extract(&sim.world);
    assert_eq!(f.meshes.len(), 1);
    assert_eq!(f.meshes[0].entity, 1);
    assert_eq!(f.local_lights.len(), 1);

    let mut q = sim
        .world
        .query::<(bevy_ecs::entity::Entity, &marionette_core::scene::SceneId)>();
    let ids: Vec<_> = q.iter(&sim.world).map(|(e, id)| (e, id.0)).collect();
    for (e, id) in ids {
        if id == 3 {
            sim.world.entity_mut(e).remove::<Hidden>();
        } else {
            sim.world.entity_mut(e).insert(Hidden);
        }
    }
    let f = extract(&sim.world);
    assert_eq!(f.meshes.len(), 1);
    assert_eq!(f.meshes[0].entity, 3);
    assert!(f.local_lights.is_empty());
}
