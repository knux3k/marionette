//! The self-made test character loads with its skin, clips and morph target, and the
//! animation module poses it deterministically.

use marionette_assets::rig::load_rig;
use std::path::Path;

fn examples() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples"))
}

#[test]
fn puppet_has_a_skin_three_clips_and_a_morph_target() {
    let rig = load_rig("assets/puppet.glb", examples()).unwrap();
    assert_eq!(rig.skins.len(), 1);
    assert_eq!(rig.skins[0].joints.len(), 15);
    let names: Vec<&str> = rig.clips.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["idle", "walk", "wave"]);
    assert_eq!(rig.clip("walk").unwrap().duration, 1.0);
    assert_eq!(rig.clip("#0").unwrap().name, "idle");
    let body = rig.node("body").unwrap();
    assert_eq!(rig.morph_weights[body], vec![0.0]);
    // Rest pose: joint matrix times inverse bind is the identity for every joint.
    let rest = rig.rest_pose();
    for (j, &node) in rig.skins[0].joints.iter().enumerate() {
        let m = marionette_assets::rig::mul(&rest[node], &rig.skins[0].inverse_bind[j]);
        for (a, b) in m.iter().zip(marionette_assets::rig::IDENTITY) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    let model = marionette_assets::load_model("assets/puppet.glb", examples()).unwrap();
    let skinned: Vec<_> = model.parts.iter().filter(|p| p.skin.is_some()).collect();
    assert_eq!(skinned.len(), 2);
    for p in &skinned {
        assert_eq!(p.mesh.joints.len(), p.mesh.positions.len());
        assert_eq!(p.mesh.morph_targets.len(), 1);
    }
    let hat = model.parts.iter().find(|p| p.skin.is_none()).unwrap();
    assert_eq!(hat.node, rig.node("hat"));
}
