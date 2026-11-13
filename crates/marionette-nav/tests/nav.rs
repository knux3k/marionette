//! Navigation on round worlds (decisions 0231, 0232): agents cross a planet round obstacles,
//! physics characters follow navigation through `MoveIntent`, a new obstacle on a route
//! makes only the affected agents plan again, runs repeat on any thread count, and a
//! restored snapshot continues exactly.

use marionette_core::Sim;
use marionette_core::scene::SceneId;
use marionette_core::spatial::Transform;
use marionette_jobs::Jobs;
use marionette_nav::{NavAgentState, NavObstacle, NavStatus};
use serde_json::{Value, json};
use std::sync::Arc;

fn modules() -> marionette_core::Modules {
    Arc::new(vec![
        Box::new(marionette_input::InputModule) as Box<dyn marionette_core::Module>,
        Box::new(marionette_physics::PhysicsModule),
        Box::new(marionette_nav::NavModule),
    ])
}

fn example(name: &str) -> Sim {
    let dir = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples"));
    let text = std::fs::read_to_string(dir.join(name)).unwrap();
    Sim::with_modules(&text, Some(name), 1, modules(), dir).unwrap()
}

fn statuses(sim: &mut Sim) -> Vec<(u64, NavStatus)> {
    let mut q = sim.world.query::<(&SceneId, &NavAgentState)>();
    let mut v: Vec<(u64, NavStatus)> = q.iter(&sim.world).map(|(i, s)| (i.0, s.status)).collect();
    v.sort_by_key(|x| x.0);
    v
}

#[test]
fn every_agent_crosses_the_planet_round_the_rocks() {
    let mut sim = example("planet.scene.json");
    for _ in 0..4000 {
        sim.step();
    }
    let st = statuses(&mut sim);
    assert_eq!(st.len(), 27);
    let late: Vec<_> = st.iter().filter(|(_, s)| *s != NavStatus::Arrived).collect();
    assert!(late.is_empty(), "not arrived: {late:?}");
}

fn t(p: [f32; 3]) -> Value {
    json!({ "translation": { "x": p[0], "y": p[1], "z": p[2] }, "rotation": { "x": 0, "y": 0, "z": 0, "w": 1 },
            "scale": { "x": 1, "y": 1, "z": 1 } })
}

/// A smooth planet (no physics) with two agents: one crossing the north pole along x, one
/// far away on the south side.
fn small() -> Sim {
    let r = 30.0;
    let agent = |p: [f32; 3], goal: [f32; 3]| {
        json!({ "Transform": t(p),
                "NavAgent": { "surface": 1, "goal": { "x": goal[0], "y": goal[1], "z": goal[2] }, "active": true,
                              "speed": 3.0, "acceleration": 8.0, "radius": 0.4, "height": 0.0,
                              "arrive_distance": 0.5, "drive": "Direct" } })
    };
    let k = r * std::f32::consts::FRAC_1_SQRT_2;
    let text = json!({
        "format": "marionette.scene", "version": 1, "resources": {},
        "entities": [
            { "id": 1, "name": "planet", "components": { "Transform": t([0.0, 0.0, 0.0]),
                "NavSurface": { "radius": r, "resolution": 48, "max_grade": 1.0, "probe": 0.0, "sea_level": 0.0, "clearance": 0.3 } } },
            { "id": 2, "name": "north", "components": agent([-k, k, 0.0], [k, k, 0.0]) },
            { "id": 3, "name": "south", "components": agent([0.0, -r, 0.0], [0.0, -k, k]) },
            { "id": 9, "name": "wall-spot", "components": { "Transform": t([0.0, r, 0.0]) } }
        ]
    })
    .to_string();
    Sim::with_modules(&text, None, 1, modules(), Default::default()).unwrap()
}

fn corners(sim: &mut Sim, id: u64) -> u32 {
    let mut q = sim.world.query::<(&SceneId, &NavAgentState)>();
    q.iter(&sim.world).find(|(i, _)| i.0 == id).unwrap().1.corners
}

fn entity(sim: &mut Sim, id: u64) -> bevy_ecs::entity::Entity {
    let mut q = sim.world.query::<(bevy_ecs::entity::Entity, &SceneId)>();
    q.iter(&sim.world).find(|(_, i)| i.0 == id).unwrap().0
}

#[test]
fn an_obstacle_on_a_route_replans_that_agent_only_and_it_still_arrives() {
    let mut sim = small();
    for _ in 0..5 {
        sim.step();
    }
    assert_eq!(corners(&mut sim, 2), 1, "straight over the pole");
    let south_before = corners(&mut sim, 3);
    // Drop a big obstacle on the pole, in the north agent's way.
    let e = entity(&mut sim, 9);
    sim.world.entity_mut(e).insert(NavObstacle {
        surface: marionette_core::builtin::SceneRef(1),
        radius: 6.0,
    });
    for _ in 0..3 {
        sim.step();
    }
    assert!(corners(&mut sim, 2) >= 2, "goes round the obstacle now");
    assert_eq!(corners(&mut sim, 3), south_before, "the far agent kept its route");
    let mut min = f32::MAX;
    for _ in 0..1500 {
        sim.step();
        let mut q = sim.world.query::<(&SceneId, &Transform)>();
        let p = q.iter(&sim.world).find(|(i, _)| i.0 == 2).unwrap().1.translation;
        let d = (p.x * p.x + (p.y - 30.0) * (p.y - 30.0) + p.z * p.z).sqrt();
        min = min.min(d);
    }
    assert!(min > 5.5, "never walked into the obstacle: {min}");
    let st = statuses(&mut sim);
    assert!(st.iter().all(|(_, s)| *s == NavStatus::Arrived), "{st:?}");
}

#[test]
fn crowd_runs_match_on_one_and_many_threads() {
    let run = |threads: usize| {
        let mut sim = example("planet-crowd.scene.json");
        sim.world.insert_resource(Jobs::with_threads(threads));
        for _ in 0..240 {
            sim.step();
        }
        sim.state_hash()
    };
    let one = run(1);
    assert_eq!(one, run(4));
    assert_eq!(one, run(1));
}

#[test]
fn a_restored_snapshot_continues_exactly() {
    let mut a = example("planet.scene.json");
    for _ in 0..200 {
        a.step();
    }
    let snap = a.save_state();
    for _ in 0..300 {
        a.step();
    }
    let mut b = example("planet.scene.json");
    b.restore(&snap).unwrap();
    for _ in 0..300 {
        b.step();
    }
    assert_eq!(a.state_hash(), b.state_hash());
}
