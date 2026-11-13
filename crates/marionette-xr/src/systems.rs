//! The XR tick: locomotion, then tracking, then hand motion, then grab and throw.
//! One exclusive system so the order of every write is fixed; entities are visited by
//! `SceneId`.

use crate::math::*;
use crate::pose::{self, XrSource};
use crate::*;
use marionette_core::actions::{ActionState, PRESSED};
use marionette_core::builtin::{DEFAULT_DT, FixedStep, Velocity};
use marionette_core::module::{RayCaster, RayFilter};
use marionette_core::scene::SceneId;
use marionette_core::spatial::Transform;
use marionette_physics::components::{AngularVelocity, BodyKind, RigidBody};
use std::collections::{BTreeMap, BTreeSet};

/// Cosine of the steepest slope a teleport may land on (about 45 degrees).
const TELEPORT_MIN_UP: f32 = 0.7;

fn ids_of<C: Component>(world: &mut World) -> Vec<(u64, Entity)> {
    let mut q = world.query_filtered::<(Entity, &SceneId), With<C>>();
    let mut v: Vec<(u64, Entity)> = q.iter(world).map(|(e, id)| (id.0, e)).collect();
    v.sort_unstable_by_key(|(id, _)| *id);
    v
}

fn was_pressed(a: &ActionState, action: &str) -> bool {
    a.previous.get(action).copied().unwrap_or(0.0) > PRESSED
}

pub(crate) fn xr_tick(world: &mut World) {
    let rigs = ids_of::<XrRig>(world);
    if rigs.is_empty() {
        return;
    }
    let Some(actions) = world.get_resource::<ActionState>().cloned() else {
        return;
    };
    let dt = world.get_resource::<FixedStep>().map_or(DEFAULT_DT, |s| s.dt);
    let mut vignette: f32 = 0.0;
    let mut teleported = BTreeSet::new();
    for &(id, e) in &rigs {
        if world.get::<XrLocomotion>(e).is_some() {
            let (v, jumped) = locomotion(world, e, &actions, dt);
            vignette = vignette.max(v);
            if jumped {
                teleported.insert(id);
            }
        }
    }
    let rig_of: BTreeMap<u64, Entity> = rigs.into_iter().collect();
    track(world, &rig_of, &actions);
    hand_motion(world, &teleported, dt);
    grab(world, &actions, dt);
    if let Some(mut c) = world.get_resource_mut::<XrComfort>() {
        c.vignette = vignette.clamp(0.0, 1.0);
    }
}

fn head_local(actions: &ActionState, rig: &XrRig) -> Vec3 {
    let head = pose::read(actions, XrSource::Head);
    add(head.position, Vec3::new(0.0, rig.height_offset, 0.0))
}

/// Turns the rig by `q` around the head, so the player turns in place.
fn turn_about_head(t: &mut Transform, head: Vec3, q: Quat) {
    let head_world = add(t.translation, rotate(t.rotation, head));
    t.rotation = qnormalize(qmul(q, t.rotation));
    t.translation = sub(head_world, rotate(t.rotation, head));
}

/// Returns (vignette wanted, teleported).
fn locomotion(world: &mut World, e: Entity, a: &ActionState, dt: f32) -> (f32, bool) {
    let loc = world.get::<XrLocomotion>(e).cloned().expect("checked");
    let rig = world.get::<XrRig>(e).copied().unwrap_or_default();
    let mut state = world
        .get::<XrLocomotionState>(e)
        .copied()
        .unwrap_or(XrLocomotionState {
            snap_armed: true,
            ..Default::default()
        });
    let Some(mut t) = world.get::<Transform>(e).copied() else {
        return (0.0, false);
    };
    let head = head_local(a, &rig);
    let mut vignette: f32 = 0.0;

    // Turning.
    if !loc.turn.is_empty() {
        let v = a.value(&loc.turn);
        match loc.turn_mode {
            TurnMode::Snap => {
                if state.snap_armed && v.abs() > 0.7 {
                    let sign = if v > 0.0 { -1.0 } else { 1.0 };
                    turn_about_head(&mut t, head, yaw(sign * loc.snap_degrees.to_radians()));
                    state.snap_armed = false;
                } else if v.abs() < 0.3 {
                    state.snap_armed = true;
                }
            }
            TurnMode::Smooth => {
                if v != 0.0 {
                    let angle = -v * loc.smooth_degrees_per_second.to_radians() * dt;
                    turn_about_head(&mut t, head, yaw(angle));
                    vignette = vignette.max(v.abs());
                }
            }
            TurnMode::Off => {}
        }
    }

    // Stick movement on the ground plane.
    let mx = if loc.move_x.is_empty() {
        0.0
    } else {
        a.value(&loc.move_x)
    };
    let my = if loc.move_y.is_empty() {
        0.0
    } else {
        a.value(&loc.move_y)
    };
    if mx != 0.0 || my != 0.0 {
        let dir = pose::read(a, loc.direction);
        let f = flat_forward(qmul(t.rotation, dir.rotation));
        let right = Vec3::new(-f.z, 0.0, f.x);
        let mut m = add(scale(right, mx), scale(f, my));
        let len = length(m);
        if len > 1.0 {
            m = scale(m, 1.0 / len);
        }
        t.translation = add(t.translation, scale(m, loc.speed * dt));
        vignette = vignette.max(len.min(1.0));
    }

    // Teleport: aim while held, jump on release.
    let mut jumped = false;
    if !loc.teleport.is_empty() {
        if a.pressed(&loc.teleport) {
            let aim = pose::read(a, loc.teleport_aim);
            let caster = world.get_resource::<RayCaster>().copied();
            state.teleport_valid = false;
            if let (true, Some(caster)) = (aim.tracked, caster) {
                let local = add(aim.position, Vec3::new(0.0, rig.height_offset, 0.0));
                let (origin, rot) = compose(&t, local, aim.rotation);
                let dir = rotate(rot, Vec3::new(0.0, 0.0, -1.0));
                if let Some(p) = teleport_point(world, caster.0, origin, dir, loc.teleport_range) {
                    state.teleport_point = p;
                    state.teleport_valid = true;
                }
            }
        } else if was_pressed(a, &loc.teleport) && state.teleport_valid {
            let head_world = add(t.translation, rotate(t.rotation, head));
            let p = state.teleport_point;
            t.translation = Vec3::new(
                t.translation.x + p.x - head_world.x,
                p.y,
                t.translation.z + p.z - head_world.z,
            );
            state.teleport_valid = false;
            jumped = true;
        } else {
            state.teleport_valid = false;
        }
    }

    if let Some(mut tr) = world.get_mut::<Transform>(e) {
        *tr = t;
    }
    world.entity_mut(e).insert(state);
    (vignette * loc.vignette, jumped)
}

/// Where an aim ray lands: the first hit if it faces up, else straight down from just before
/// the hit (or from the end of the ray). `None` if there is no walkable ground.
fn teleport_point(
    world: &World,
    ray: marionette_core::module::RayFn,
    origin: Vec3,
    dir: Vec3,
    range: f32,
) -> Option<Vec3> {
    let filter = RayFilter {
        exclude: None,
        skip_sensors: true,
    };
    let to3 = |v: Vec3| [v.x, v.y, v.z];
    let from3 = |v: [f32; 3]| Vec3::new(v[0], v[1], v[2]);
    let hit = ray(world, to3(origin), to3(dir), range, filter);
    let drop_from = match hit {
        Some(h) if h.normal[1] >= TELEPORT_MIN_UP => return Some(from3(h.point)),
        Some(h) => add(origin, scale(normalize(dir), (h.distance - 0.1).max(0.0))),
        None => add(origin, scale(normalize(dir), range)),
    };
    let down = ray(world, to3(drop_from), [0.0, -1.0, 0.0], range * 2.0, filter)?;
    (down.normal[1] >= TELEPORT_MIN_UP).then(|| from3(down.point))
}

fn track(world: &mut World, rigs: &BTreeMap<u64, Entity>, a: &ActionState) {
    for (_, e) in ids_of::<XrTracked>(world) {
        let tracked = *world.get::<XrTracked>(e).expect("queried");
        let Some(&rig_e) = rigs.get(&tracked.rig.0) else {
            continue;
        };
        let p = pose::read(a, tracked.source);
        if !p.tracked {
            continue;
        }
        let rig = world.get::<XrRig>(rig_e).copied().unwrap_or_default();
        let rig_t = world.get::<Transform>(rig_e).copied().unwrap_or_default();
        let local = add(p.position, Vec3::new(0.0, rig.height_offset, 0.0));
        let (pos, rot) = compose(&rig_t, local, p.rotation);
        if let Some(mut t) = world.get_mut::<Transform>(e) {
            t.translation = pos;
            t.rotation = rot;
        }
    }
}

fn hand_motion(world: &mut World, teleported: &BTreeSet<u64>, dt: f32) {
    for (_, e) in ids_of::<XrHand>(world) {
        let Some(t) = world.get::<Transform>(e).copied() else {
            continue;
        };
        let rig = world.get::<XrTracked>(e).map(|x| x.rig.0);
        let mut m = world.get::<XrHandMotion>(e).cloned().unwrap_or_default();
        if rig.is_some_and(|r| teleported.contains(&r)) {
            m.positions.clear();
        }
        let first_tick = m.positions.is_empty();
        m.positions.push(t.translation);
        if m.positions.len() > MOTION_TICKS {
            m.positions.remove(0);
        }
        let n = m.positions.len();
        m.velocity = if n >= 2 {
            scale(
                sub(m.positions[n - 1], m.positions[0]),
                1.0 / ((n - 1) as f32 * dt),
            )
        } else {
            Vec3::default()
        };
        m.angular_velocity = if first_tick {
            Vec3::default()
        } else {
            angular_velocity(m.last_rotation, t.rotation, dt)
        };
        m.last_rotation = t.rotation;
        world.entity_mut(e).insert(m);
    }
}

fn is_dynamic(world: &World, e: Entity) -> bool {
    world
        .get::<RigidBody>(e)
        .is_some_and(|b| b.kind == BodyKind::Dynamic)
}

fn grab(world: &mut World, a: &ActionState, dt: f32) {
    let hands = ids_of::<XrHand>(world);
    if hands.is_empty() {
        return;
    }
    let grabbables = ids_of::<Grabbable>(world);
    for (hand_id, he) in hands {
        let hand = world.get::<XrHand>(he).cloned().expect("queried");
        let Some(ht) = world.get::<Transform>(he).copied() else {
            continue;
        };
        let side = match world.get::<XrTracked>(he).map(|t| t.source) {
            Some(XrSource::LeftGrip | XrSource::LeftAim) => Hand::Left,
            _ => Hand::Right,
        };
        let held = grabbables
            .iter()
            .copied()
            .find(|&(_, ge)| world.get::<Held>(ge).is_some_and(|h| h.hand.0 == hand_id));
        let pressed = a.pressed(&hand.grab_action);
        match held {
            None if a.just_pressed(&hand.grab_action) => {
                let mut best: Option<(f32, Entity)> = None;
                for &(_, ge) in &grabbables {
                    if world.get::<Held>(ge).is_some() {
                        continue;
                    }
                    let Some(gt) = world.get::<Transform>(ge) else {
                        continue;
                    };
                    let d = length(sub(gt.translation, ht.translation));
                    if d <= hand.grab_radius && best.is_none_or(|(bd, _)| d < bd) {
                        best = Some((d, ge));
                    }
                }
                if let Some((_, ge)) = best {
                    let g = *world.get::<Grabbable>(ge).expect("queried");
                    let gt = *world.get::<Transform>(ge).expect("checked");
                    let (ot, or) = if g.snap {
                        (Vec3::default(), Quat::default())
                    } else {
                        relative(ht.translation, ht.rotation, gt.translation, gt.rotation)
                    };
                    let mut ent = world.entity_mut(ge);
                    ent.insert(Held {
                        hand: SceneRef(hand_id),
                        offset_translation: ot,
                        offset_rotation: or,
                    });
                    if is_dynamic(world, ge) {
                        let mut ent = world.entity_mut(ge);
                        if !ent.contains::<Velocity>() {
                            ent.insert(Velocity::default());
                        }
                        if !ent.contains::<AngularVelocity>() {
                            ent.insert(AngularVelocity::default());
                        }
                    }
                    if let Some(mut h) = world.get_resource_mut::<XrHaptics>() {
                        h.push(HapticPulse {
                            hand: side,
                            amplitude: 0.4,
                            seconds: 0.03,
                        });
                    }
                }
            }
            None => {}
            Some((_, ge)) if !pressed => {
                world.entity_mut(ge).remove::<Held>();
                let m = world.get::<XrHandMotion>(he).cloned().unwrap_or_default();
                if let Some(mut v) = world.get_mut::<Velocity>(ge) {
                    let l = scale(m.velocity, hand.throw_scale);
                    (v.x, v.y, v.z) = (l.x, l.y, l.z);
                }
                if let Some(mut w) = world.get_mut::<AngularVelocity>(ge) {
                    let r = m.angular_velocity;
                    (w.x, w.y, w.z) = (r.x, r.y, r.z);
                }
            }
            Some((_, ge)) => {
                let h = *world.get::<Held>(ge).expect("found");
                let (pos, rot) = compose(&ht, h.offset_translation, h.offset_rotation);
                if is_dynamic(world, ge) {
                    let cur = world.get::<Transform>(ge).copied().unwrap_or_default();
                    let lin = scale(sub(pos, cur.translation), 1.0 / dt);
                    let ang = angular_velocity(cur.rotation, rot, dt);
                    if let Some(mut v) = world.get_mut::<Velocity>(ge) {
                        (v.x, v.y, v.z) = (lin.x, lin.y, lin.z);
                    }
                    if let Some(mut w) = world.get_mut::<AngularVelocity>(ge) {
                        (w.x, w.y, w.z) = (ang.x, ang.y, ang.z);
                    }
                } else if let Some(mut t) = world.get_mut::<Transform>(ge) {
                    t.translation = pos;
                    t.rotation = rot;
                }
            }
        }
    }
}
