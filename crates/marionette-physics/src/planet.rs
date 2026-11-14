//! Movement on round worlds (decision 0230): point gravity toward the centre of one or more
//! bodies, characters whose "up" follows their position, swimming in spherical fluid, and
//! moving between bodies. 3D only; everything is opt-in through components:
//!
//! - [`GravityWell`] on a body's entity (centre = its `Transform` translation) pulls every
//!   dynamic body and every [`PlanetCharacter`]. Wells add up, so a craft between two moons
//!   feels both and a character crossing from one to the other turns smoothly.
//! - [`SphereFluid`] on a well's entity is an ocean or atmosphere around the same centre:
//!   inside it gravity is reduced by `buoyancy` and motion is damped by `drag`.
//! - [`PlanetCharacter`] next to a `CharacterController` makes the character walk on the
//!   sphere: its up turns toward minus gravity, it walks in its own tangent plane, jumps
//!   along up, may fly with `thrust`, and follows a `MoveIntent` (from navigation or a script).
//!
//! Only `+ - * /` and `sqrt` are used per tick (determinism rules in ARCHITECTURE.md).

use bevy_ecs::prelude::*;
use bevy_reflect::Reflect;
use marionette_core::spatial::Vec3;
use marionette_core::vmath::*;

/// A point gravity source at the entity's `Transform` translation (a planet, a moon).
/// Gravity is `surface_gravity * (radius / r)²` at distance `r >= radius` and falls off
/// linearly to zero at the centre inside (a body of even density), so it is finite
/// everywhere. Set the scene's `PhysicsSettings.gravity` to zero when wells are used.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq)]
pub struct GravityWell {
    /// Acceleration in m/s² at `radius`.
    pub surface_gravity: f32,
    /// Reference radius in metres (the ground level).
    pub radius: f32,
    /// No pull beyond this distance from the centre (sphere of influence); 0 = everywhere.
    pub range: f32,
}

/// A spherical fluid (ocean, thick atmosphere) around the centre of the entity's `Transform`.
/// Bodies and characters closer to the centre than `radius` are inside it.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq)]
pub struct SphereFluid {
    /// Fluid surface distance from the centre in metres.
    pub radius: f32,
    /// Fraction of gravity cancelled inside (1 = floats in place, above 1 rises).
    pub buoyancy: f32,
    /// Velocity damping per second inside (0 = none).
    pub drag: f32,
}

/// Walk on round worlds. Needs a `CharacterController` (its speed, jump speed, mass and
/// actions are used; its `gravity` is used only where no `GravityWell` pulls).
#[derive(Component, Reflect, Debug, Clone, PartialEq)]
pub struct PlanetCharacter {
    /// How fast up turns toward minus gravity, in radians per second (0 = at once).
    pub align_speed: f32,
    /// Upward acceleration in m/s² while `thrust_action` is held (jetpack); 0 = none.
    /// Inside a `SphereFluid` it is the swim-up acceleration.
    pub thrust: f32,
    /// Action that fires `thrust` (empty = none).
    pub thrust_action: String,
    /// Turn to face the walking direction (AI characters); off for player-turned characters.
    pub face_movement: bool,
}

/// What a `PlanetCharacter` currently feels. Written by physics.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq, Default)]
pub struct PlanetState {
    /// The gravity acting on the character in m/s² (zero in empty space).
    pub gravity: Vec3,
    /// The character's up (unit length).
    pub up: Vec3,
    pub swimming: bool,
}

/// A well or fluid as the step sees it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Well {
    pub centre: [f32; 3],
    pub g: f32,
    pub radius: f32,
    pub range: f32,
    pub fluid: Option<SphereFluid>,
}

/// Every well of the scene, in scene id order (empty when the scene has none).
pub(crate) fn wells(world: &mut World) -> Vec<Well> {
    use marionette_core::scene::SceneId;
    use marionette_core::spatial::Transform;
    let Some(mut q) = world.try_query::<(&SceneId, &Transform, Option<&GravityWell>, Option<&SphereFluid>)>()
    else {
        return Vec::new();
    };
    let mut out: Vec<(u64, Well)> = q
        .iter(world)
        .filter(|(_, _, g, f)| g.is_some() || f.is_some())
        .map(|(id, t, g, f)| {
            let t = t.translation;
            let g = g.copied().unwrap_or(GravityWell {
                surface_gravity: 0.0,
                radius: 1.0,
                range: 0.0,
            });
            (
                id.0,
                Well {
                    centre: [t.x, t.y, t.z],
                    g: g.surface_gravity,
                    radius: g.radius.max(1e-3),
                    range: g.range,
                    fluid: f.copied(),
                },
            )
        })
        .collect();
    out.sort_by_key(|(id, _)| *id);
    out.into_iter().map(|(_, w)| w).collect()
}

/// Gravity at `p` from all wells, and the fluid `p` is in (the first by id), if any.
pub(crate) fn field(wells: &[Well], p: [f32; 3]) -> ([f32; 3], Option<SphereFluid>) {
    let mut g = [0.0f32; 3];
    let mut fluid = None;
    for w in wells {
        let d = sub(w.centre, p);
        let r2 = dot(d, d);
        let r = r2.sqrt();
        if let Some(f) = w.fluid
            && fluid.is_none()
            && r < f.radius
        {
            fluid = Some(f);
        }
        if w.g == 0.0 || r < 1e-6 || (w.range > 0.0 && r > w.range) {
            continue;
        }
        // Magnitude over r, so `d * k` is the acceleration vector.
        let k = if r >= w.radius {
            w.g * w.radius * w.radius / (r2 * r)
        } else {
            w.g / w.radius
        };
        g = add(g, scale(d, k));
    }
    (g, fluid)
}

/// Turns unit `from` toward unit `to` by at most `max` radians (chord approximation: exact
/// direction, step length within 5% for steps under 0.5 rad).
pub fn turn_toward(from: [f32; 3], to: [f32; 3], max: f32) -> [f32; 3] {
    let gap = sub(to, from);
    let chord = len(gap);
    if max <= 0.0 || chord <= max || chord < 1e-6 {
        return to;
    }
    norm(add(from, scale(gap, max / chord))).unwrap_or(to)
}

/// Inputs to one character step, gathered by the backend.
pub(crate) struct Step {
    pub rotation: [f32; 4],
    pub position: [f32; 3],
    /// Walk input: sideways and forward (-1..1), from actions or `CharacterInput`.
    pub walk: [f32; 2],
    /// World-space velocity from a `MoveIntent`, replacing `walk` when present.
    pub intent: Option<[f32; 3]>,
    pub jump: bool,
    pub thrust_held: bool,
    pub vertical_velocity: f32,
    pub grounded: bool,
    pub speed: f32,
    pub jump_speed: f32,
    pub fallback_gravity: f32,
    pub dt: f32,
}

/// The planned move: rotation to set, up for the controller, desired translation.
pub(crate) struct Plan {
    pub rotation: [f32; 4],
    pub up: [f32; 3],
    pub translation: [f32; 3],
    pub vertical_velocity: f32,
    pub jumped: bool,
    pub state: PlanetState,
}

/// Plans one character step on a round world (before collision handling).
pub(crate) fn plan(pc: &PlanetCharacter, wells: &[Well], s: &Step) -> Plan {
    let (mut g, fluid) = field(wells, s.position);
    let up_now = qrot(s.rotation, [0.0, 1.0, 0.0]);
    let g_len = len(g);
    if g_len < 1e-6 {
        g = scale(up_now, -s.fallback_gravity);
    }
    let target_up = norm(scale(g, -1.0)).unwrap_or(up_now);
    let up = turn_toward(up_now, target_up, pc.align_speed * s.dt);
    let mut rotation = qnorm(qmul(arc(up_now, up), s.rotation));

    // Walking velocity in the tangent plane.
    let right = qrot(rotation, [1.0, 0.0, 0.0]);
    let fwd = qrot(rotation, [0.0, 0.0, -1.0]);
    let mut walk = match s.intent {
        Some(v) => {
            let t = sub(v, scale(up, dot(v, up)));
            let l = len(t);
            if l > s.speed && l > 0.0 {
                scale(t, s.speed / l)
            } else {
                t
            }
        }
        None => scale(add(scale(right, s.walk[0]), scale(fwd, s.walk[1])), s.speed),
    };
    if pc.face_movement
        && len(walk) > 1e-3
        && let Some(q) = basis(up, walk)
    {
        rotation = q;
    }

    let g_mag = len(g);
    let mut vy = s.vertical_velocity;
    let mut jumped = false;
    match fluid {
        Some(f) => {
            vy -= g_mag * (1.0 - f.buoyancy) * s.dt;
            if s.thrust_held || s.jump {
                vy += pc.thrust.max(s.jump_speed) * s.dt;
            }
            let damp = 1.0 / (1.0 + f.drag * s.dt);
            vy *= damp;
            walk = scale(walk, damp.max(0.5));
        }
        None => {
            vy -= g_mag * s.dt;
            if s.grounded && s.jump {
                vy = s.jump_speed;
                jumped = true;
            }
            if s.thrust_held && pc.thrust > 0.0 {
                vy += pc.thrust * s.dt;
            }
        }
    }
    let translation = add(scale(walk, s.dt), scale(up, vy * s.dt));
    Plan {
        rotation,
        up,
        translation,
        vertical_velocity: vy,
        jumped,
        state: PlanetState {
            gravity: Vec3::new(g[0], g[1], g[2]),
            up: Vec3::new(up[0], up[1], up[2]),
            swimming: fluid.is_some(),
        },
    }
}

/// The force pulling a dynamic body of `mass` at `p` moving at `v`, with its gravity scale.
pub(crate) fn body_force(
    wells: &[Well],
    p: [f32; 3],
    v: [f32; 3],
    mass: f32,
    gravity_scale: f32,
) -> [f32; 3] {
    let (g, fluid) = field(wells, p);
    match fluid {
        Some(f) => add(
            scale(g, mass * gravity_scale * (1.0 - f.buoyancy)),
            scale(v, -mass * f.drag),
        ),
        None => scale(g, mass * gravity_scale),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn well(r: f32) -> Well {
        Well {
            centre: [0.0; 3],
            g: 9.81,
            radius: r,
            range: 0.0,
            fluid: None,
        }
    }

    #[test]
    fn gravity_points_to_the_centre_and_falls_with_distance() {
        let w = [well(100.0)];
        let (g, _) = field(&w, [0.0, 100.0, 0.0]);
        assert!((g[1] + 9.81).abs() < 1e-4 && g[0] == 0.0);
        let (g2, _) = field(&w, [200.0, 0.0, 0.0]);
        assert!((g2[0] + 9.81 / 4.0).abs() < 1e-4);
        let (g3, _) = field(&w, [0.0, 0.0, 50.0]);
        assert!((g3[2] + 9.81 / 2.0).abs() < 1e-4, "linear inside");
    }

    #[test]
    fn range_limits_a_well() {
        let mut w = well(10.0);
        w.range = 50.0;
        assert_eq!(field(&[w], [0.0, 60.0, 0.0]).0, [0.0; 3]);
    }

    #[test]
    fn arc_and_basis_are_rotations() {
        let q = arc([0.0, 1.0, 0.0], [1.0, 0.0, 0.0]);
        let v = qrot(q, [0.0, 1.0, 0.0]);
        assert!((v[0] - 1.0).abs() < 1e-5 && v[1].abs() < 1e-5);
        let q = arc([0.0, 1.0, 0.0], [0.0, -1.0, 0.0]);
        let v = qrot(q, [0.0, 1.0, 0.0]);
        assert!((v[1] + 1.0).abs() < 1e-5);
        let up = norm([1.0, 1.0, 0.0]).unwrap();
        let q = basis(up, [0.0, 0.0, 1.0]).unwrap();
        let u = qrot(q, [0.0, 1.0, 0.0]);
        let f = qrot(q, [0.0, 0.0, -1.0]);
        assert!(dot(u, up) > 0.9999, "{u:?}");
        assert!(f[2] > 0.9999, "{f:?}");
    }

    #[test]
    fn up_follows_position_around_the_planet() {
        let pc = PlanetCharacter {
            align_speed: 0.0,
            thrust: 0.0,
            thrust_action: String::new(),
            face_movement: false,
        };
        let s = Step {
            rotation: [0.0, 0.0, 0.0, 1.0],
            position: [0.0, 0.0, -101.0],
            walk: [0.0, 1.0],
            intent: None,
            jump: false,
            thrust_held: false,
            vertical_velocity: 0.0,
            grounded: true,
            speed: 2.0,
            jump_speed: 5.0,
            fallback_gravity: 9.81,
            dt: 0.1,
        };
        let p = plan(&pc, &[well(100.0)], &s);
        assert!((p.up[2] + 1.0).abs() < 1e-5, "up is outward: {:?}", p.up);
        // Forward input walks in the tangent plane (no part along up besides gravity).
        let along_up = dot(p.translation, p.up);
        assert!(along_up < 0.0 && along_up > -0.2);
        assert!(p.state.gravity.z > 9.0);
    }

    fn step(vy: f32, jump: bool, thrust: bool) -> Step {
        Step {
            rotation: [0.0, 0.0, 0.0, 1.0],
            position: [0.0, 101.0, 0.0],
            walk: [0.0, 0.0],
            intent: None,
            jump,
            thrust_held: thrust,
            vertical_velocity: vy,
            grounded: true,
            speed: 2.0,
            jump_speed: 5.0,
            fallback_gravity: 9.81,
            dt: 0.1,
        }
    }

    fn pc(thrust: f32) -> PlanetCharacter {
        PlanetCharacter {
            align_speed: 0.0,
            thrust,
            thrust_action: "t".into(),
            face_movement: false,
        }
    }

    #[test]
    fn jetpack_thrust_beats_gravity_and_jumps_start_on_ground() {
        let w = [well(100.0)];
        let p = plan(&pc(20.0), &w, &step(0.0, false, true));
        assert!(
            p.vertical_velocity > 0.9,
            "thrust 20 against 9.6: {}",
            p.vertical_velocity
        );
        let p = plan(&pc(0.0), &w, &step(0.0, true, false));
        assert!(p.jumped && p.vertical_velocity == 5.0);
    }

    #[test]
    fn a_neutral_fluid_floats_and_damps() {
        let mut w = well(100.0);
        w.fluid = Some(SphereFluid {
            radius: 110.0,
            buoyancy: 1.0,
            drag: 2.0,
        });
        let p = plan(&pc(3.0), &[w], &step(-4.0, false, false));
        assert!(p.state.swimming);
        assert!(
            p.vertical_velocity > -4.0 && p.vertical_velocity < 0.0,
            "damped: {}",
            p.vertical_velocity
        );
        let p = plan(&pc(3.0), &[w], &step(0.0, false, true));
        assert!(p.vertical_velocity > 0.0, "swims up");
    }

    #[test]
    fn up_turns_gradually_with_align_speed() {
        let mut c = pc(0.0);
        c.align_speed = 1.0;
        let mut s = step(0.0, false, false);
        s.position = [101.0, 0.0, 0.0]; // gravity wants up = +X, current up = +Y
        let p = plan(&c, &[well(100.0)], &s);
        let turned = dot(p.up, [1.0, 0.0, 0.0]);
        assert!(turned > 0.05 && turned < 0.2, "about 0.1 rad: {turned}");
    }
}
