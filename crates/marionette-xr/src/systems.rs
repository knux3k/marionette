//! The XR tick: locomotion, then tracking, then hand motion, then grab and throw.
//! One exclusive system so the order of every write is fixed; entities are visited by
//! `SceneId`.

use crate::math::*;
use crate::pose::{self, XrSource};
use crate::*;
use marionette_core::actions::{ActionState, PRESSED};
use marionette_core::builtin::{DEFAULT_DT, FixedStep, MoveIntent, Velocity};
use marionette_core::module::{RayCaster, RayFilter};
use marionette_core::scene::SceneId;
use marionette_core::spatial::Transform;
use marionette_physics::PlanetState;
use marionette_physics::components::{AngularVelocity, BodyKind, Collider, ColliderShape, RigidBody};
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
            let (v, jumped) = match body_of(world, e) {
                Some(body) => ride(world, e, body, &actions, dt),
                None => locomotion(world, e, &actions, dt),
            };
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

/// After physics: rigs riding a body follow it, then tracked entities and held objects are
/// placed again from the moved rigs.
pub(crate) fn xr_follow(world: &mut World) {
    let rigs = ids_of::<XrCharacter>(world);
    if rigs.is_empty() {
        return;
    }
    let Some(actions) = world.get_resource::<ActionState>().cloned() else {
        return;
    };
    let mut moved = false;
    for &(_, e) in &rigs {
        if let Some(body) = body_of(world, e) {
            moved |= follow(world, e, body, &actions);
        }
    }
    if moved {
        let all: BTreeMap<u64, Entity> = ids_of::<XrRig>(world).into_iter().collect();
        track(world, &all, &actions);
        carry_held(world);
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
    let Some(loc) = world.get::<XrLocomotion>(e).cloned() else {
        return (0.0, false);
    };
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
        let Some(&tracked) = world.get::<XrTracked>(e) else {
            continue;
        };
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
        let Some(hand) = world.get::<XrHand>(he).cloned() else {
            continue;
        };
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
                if let Some((_, ge)) = best
                    && let (Some(&g), Some(&gt)) = (world.get::<Grabbable>(ge), world.get::<Transform>(ge))
                {
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
                let Some(&h) = world.get::<Held>(ge) else {
                    continue;
                };
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

/// The body entity of a rig with [`XrCharacter`], if it exists.
fn body_of(world: &mut World, rig: Entity) -> Option<Entity> {
    let id = world.get::<XrCharacter>(rig)?.body.0;
    let mut q = world.query::<(Entity, &SceneId)>();
    q.iter(world).find(|(_, s)| s.0 == id).map(|(e, _)| e)
}

/// A body's up: what `PlanetCharacter` feels, else its own +Y.
fn body_up(world: &World, body: Entity, t: &Transform) -> Vec3 {
    match world.get::<PlanetState>(body) {
        Some(s) if length(s.up) > 0.5 => normalize(s.up),
        _ => normalize(rotate(t.rotation, Vec3::new(0.0, 1.0, 0.0))),
    }
}

/// Distance from a body's centre down to its feet (the bottom of its collider).
fn feet_depth(world: &World, body: Entity) -> f32 {
    match world.get::<Collider>(body).map(|c| &c.shape) {
        Some(ColliderShape::Capsule { half_height, radius }) => half_height + radius,
        Some(ColliderShape::Ball { radius }) => *radius,
        Some(ColliderShape::Cuboid { half_extents }) => half_extents.y,
        _ => 0.0,
    }
}

/// Rotation of `angle` radians about unit `axis`.
fn axis_angle(axis: Vec3, angle: f32) -> Quat {
    let (s, c) = (libm::sinf(angle * 0.5), libm::cosf(angle * 0.5));
    Quat {
        x: axis.x * s,
        y: axis.y * s,
        z: axis.z * s,
        w: c,
    }
}

fn arr(v: Vec3) -> [f32; 3] {
    [v.x, v.y, v.z]
}

fn quat(q: [f32; 4]) -> Quat {
    Quat {
        x: q[0],
        y: q[1],
        z: q[2],
        w: q[3],
    }
}

/// The shortest rotation turning unit `a` onto unit `b`.
fn arc(a: Vec3, b: Vec3) -> Quat {
    quat(marionette_core::vmath::arc(arr(a), arr(b)))
}

/// `v` without its part along unit `up`.
fn flatten(v: Vec3, up: Vec3) -> Vec3 {
    sub(v, scale(up, dot(v, up)))
}

/// A rotation whose +Y is `up` and whose -Z points along `forward` (flattened).
fn upright(up: Vec3, forward: Vec3) -> Option<Quat> {
    marionette_core::vmath::basis(arr(up), arr(forward)).map(quat)
}

/// The head in the world for a rig at `t`.
fn head_world(t: &Transform, a: &ActionState, rig: &XrRig) -> Vec3 {
    add(t.translation, rotate(t.rotation, head_local(a, rig)))
}

/// Puts the rig on the body: upright on the body's up, its floor at the body's feet, the head
/// straight above the body's centre, facing where the body faces.
fn place_on_body(t: &mut Transform, a: &ActionState, rig: &XrRig, body_t: &Transform, up: Vec3, depth: f32) {
    let head = head_local(a, rig);
    let body_fwd = rotate(body_t.rotation, Vec3::new(0.0, 0.0, -1.0));
    let head_fwd = flat_forward(pose::read(a, XrSource::Head).rotation);
    // Turn the play space so the head looks where the body faces.
    let base = upright(up, body_fwd).unwrap_or(body_t.rotation);
    let undo = conj(upright(Vec3::new(0.0, 1.0, 0.0), head_fwd).unwrap_or_default());
    t.rotation = qnormalize(qmul(base, undo));
    let feet = sub(body_t.translation, scale(up, depth));
    let flat_head = rotate(t.rotation, Vec3::new(head.x, 0.0, head.z));
    t.translation = sub(feet, flat_head);
}

/// Locomotion for a rig riding a body (before physics). Returns (vignette wanted, teleported).
fn ride(world: &mut World, e: Entity, body: Entity, a: &ActionState, dt: f32) -> (f32, bool) {
    let Some(loc) = world.get::<XrLocomotion>(e).cloned() else {
        return (0.0, false);
    };
    let ride = world.get::<XrCharacter>(e).copied().unwrap_or_default();
    let rig = world.get::<XrRig>(e).copied().unwrap_or_default();
    let mut state = world
        .get::<XrLocomotionState>(e)
        .copied()
        .unwrap_or(XrLocomotionState {
            snap_armed: true,
            ..Default::default()
        });
    let mut cs = world.get::<XrCharacterState>(e).copied().unwrap_or_default();
    let (Some(mut t), Some(mut bt)) = (
        world.get::<Transform>(e).copied(),
        world.get::<Transform>(body).copied(),
    ) else {
        return (0.0, false);
    };
    let up = body_up(world, body, &bt);
    let depth = feet_depth(world, body);
    if !cs.placed {
        place_on_body(&mut t, a, &rig, &bt, up, depth);
        cs.placed = true;
    }
    let mut vignette: f32 = 0.0;

    // Turning about the body's up through the head.
    if !loc.turn.is_empty() {
        let v = a.value(&loc.turn);
        let angle = match loc.turn_mode {
            TurnMode::Snap => {
                if state.snap_armed && v.abs() > 0.7 {
                    state.snap_armed = false;
                    let sign = if v > 0.0 { -1.0 } else { 1.0 };
                    sign * loc.snap_degrees.to_radians()
                } else {
                    if v.abs() < 0.3 {
                        state.snap_armed = true;
                    }
                    0.0
                }
            }
            TurnMode::Smooth => {
                vignette = vignette.max(v.abs());
                -v * loc.smooth_degrees_per_second.to_radians() * dt
            }
            TurnMode::Off => 0.0,
        };
        if angle != 0.0 {
            let q = axis_angle(up, angle);
            let pivot = head_world(&t, a, &rig);
            t.rotation = qnormalize(qmul(q, t.rotation));
            t.translation = add(pivot, rotate(q, sub(t.translation, pivot)));
            t.translation = sub(t.translation, sub(head_world(&t, a, &rig), pivot));
        }
    }

    // Stick movement along the body's ground.
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
    let mut stick = Vec3::default();
    if mx != 0.0 || my != 0.0 {
        let dir = pose::read(a, loc.direction);
        let (_, r) = compose(&t, Vec3::default(), dir.rotation);
        let mut f = normalize(flatten(rotate(r, Vec3::new(0.0, 0.0, -1.0)), up));
        if length(f) < 0.5 {
            f = normalize(flatten(rotate(r, Vec3::new(0.0, 1.0, 0.0)), up));
        }
        let right = cross(f, up);
        let mut m = add(scale(right, mx), scale(f, my));
        let len = length(m);
        if len > 1.0 {
            m = scale(m, 1.0 / len);
        }
        stick = scale(m, loc.speed);
        vignette = vignette.max(len.min(1.0));
    }

    // Room-scale: the body steps under the head.
    let head = head_world(&t, a, &rig);
    let head_step = if ride.follow_head {
        flatten(sub(head, bt.translation), up)
    } else {
        Vec3::default()
    };

    // Teleport: aim while held, move the body on release.
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
            let p = state.teleport_point;
            let new_centre = add(p, scale(up, depth + 0.02));
            let shift = sub(new_centre, bt.translation);
            bt.translation = new_centre;
            t.translation = add(t.translation, shift);
            state.teleport_valid = false;
            jumped = true;
        } else {
            state.teleport_valid = false;
        }
    }

    // The body faces where the head looks and walks where the stick and the room say.
    let head_rot = compose(&t, Vec3::default(), pose::read(a, XrSource::Head).rotation).1;
    if let Some(r) = upright(up, rotate(head_rot, Vec3::new(0.0, 0.0, -1.0))) {
        bt.rotation = r;
    }
    let step_head = if jumped { Vec3::default() } else { head_step };
    let velocity = add(stick, scale(step_head, 1.0 / dt));
    cs.body_before = bt.translation;
    cs.up_before = up;
    cs.stick_step = scale(stick, dt);
    cs.head_step = step_head;

    if let Some(mut tr) = world.get_mut::<Transform>(e) {
        *tr = t;
    }
    if let Some(mut tr) = world.get_mut::<Transform>(body) {
        *tr = bt;
    }
    let intent = MoveIntent {
        velocity,
        jump: world.get::<MoveIntent>(body).is_some_and(|i| i.jump),
    };
    world.entity_mut(body).insert(intent);
    world.entity_mut(e).insert((state, cs));
    (vignette * loc.vignette, jumped)
}

/// After physics: moves the rig by the body's walk (not by the room-scale step, which only
/// brought the body under the head), stands it on the body's new up and feet. Returns whether
/// the rig moved.
fn follow(world: &mut World, e: Entity, body: Entity, a: &ActionState) -> bool {
    let Some(cs) = world.get::<XrCharacterState>(e).copied() else {
        return false;
    };
    if !cs.placed {
        return false;
    }
    let rig = world.get::<XrRig>(e).copied().unwrap_or_default();
    let (Some(mut t), Some(bt)) = (
        world.get::<Transform>(e).copied(),
        world.get::<Transform>(body).copied(),
    ) else {
        return false;
    };
    let before = t;
    let up = body_up(world, body, &bt);
    let depth = feet_depth(world, body);
    // Share out what physics allowed between the stick and the room-scale step: a blocked
    // step leaves the head where it is.
    let moved = sub(bt.translation, cs.body_before);
    let along = flatten(moved, cs.up_before);
    let asked = add(cs.stick_step, cs.head_step);
    let asked_len = length(asked);
    let share = if asked_len > 1e-6 {
        (length(along) / asked_len).min(1.0)
    } else {
        0.0
    };
    t.translation = add(t.translation, sub(along, scale(cs.head_step, share)));
    // Stand upright on the body's up, turning about the head so the view does not swing.
    let rig_up = rotate(t.rotation, Vec3::new(0.0, 1.0, 0.0));
    if dot(rig_up, up) < 0.999_999 {
        let pivot = head_world(&t, a, &rig);
        let q = arc(rig_up, up);
        t.rotation = qnormalize(qmul(q, t.rotation));
        t.translation = add(pivot, rotate(q, sub(t.translation, pivot)));
    }
    // The floor at the body's feet.
    let feet = sub(bt.translation, scale(up, depth));
    t.translation = sub(t.translation, scale(up, dot(sub(t.translation, feet), up)));
    if t == before {
        return false;
    }
    if let Some(mut tr) = world.get_mut::<Transform>(e) {
        *tr = t;
    }
    true
}

/// Objects held without a dynamic body follow their hand again after the hands moved.
fn carry_held(world: &mut World) {
    let hands: BTreeMap<u64, Entity> = ids_of::<XrHand>(world).into_iter().collect();
    for (_, ge) in ids_of::<Held>(world) {
        let Some(&h) = world.get::<Held>(ge) else {
            continue;
        };
        if is_dynamic(world, ge) {
            continue;
        }
        let Some(ht) = hands
            .get(&h.hand.0)
            .and_then(|he| world.get::<Transform>(*he))
            .copied()
        else {
            continue;
        };
        let (pos, rot) = compose(&ht, h.offset_translation, h.offset_rotation);
        if let Some(mut t) = world.get_mut::<Transform>(ge) {
            t.translation = pos;
            t.rotation = rot;
        }
    }
}
