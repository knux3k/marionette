//! The physics backend, written once and expanded for Rapier 3D (`d3`) and 2D (`d2`). Each
//! expansion site provides a `conv` module with the few functions that differ by dimension.

macro_rules! backend {
    () => {
        use crate::components::{
            AngularVelocity, BodyKind, CharacterController, CharacterInput, CharacterState,
            Collider as ColliderComp, RigidBody as BodyComp,
        };
        use crate::planet::PlanetCharacter;
        use bevy_ecs::entity::Entity;
        use bevy_ecs::world::World;
        use marionette_core::actions::ActionState;
        use marionette_core::builtin::MoveIntent;
        use marionette_core::builtin::{Position, Velocity};
        use marionette_core::module::EventLog;
        use marionette_core::spatial::Transform;
        use marionette_core::{Diagnostic, Errors};
        use serde::{Deserialize, Serialize};
        use std::collections::BTreeMap;
        use std::path::Path;

        #[derive(Serialize, Deserialize, Clone, Copy, Debug)]
        pub struct Handles {
            pub body: RigidBodyHandle,
            pub collider: ColliderHandle,
            pub kind: u8, // 0 dynamic, 1 fixed, 2 kinematic, 3 character
        }

        /// Everything physics knows. Serialized whole for snapshots.
        #[derive(Serialize, Deserialize)]
        pub struct State {
            pub world: PhysicsWorld,
            /// Scene id -> Rapier handles, in id order.
            pub entities: BTreeMap<u64, Handles>,
        }

        fn err(code: &'static str, id: u64, msg: String, fix: &str) -> Errors {
            Diagnostic::error(code, msg)
                .entity(id)
                .fix(fix.to_string())
                .into()
        }

        /// Builds the physics world from the scene's components (bodies in id order).
        pub fn build(
            world: &mut World,
            ids: &BTreeMap<u64, Entity>,
            scene_dir: &Path,
        ) -> Result<State, Errors> {
            let mut state = State {
                world: PhysicsWorld::default(),
                entities: BTreeMap::new(),
            };
            for (&id, &e) in ids {
                add(&mut state, world, id, e, scene_dir)?;
            }
            Ok(state)
        }

        /// Follows entities spawned or despawned since the last tick (scripts do this):
        /// bodies of entities that are gone are removed, new entities with a `Collider` get
        /// a body, both in id order. Returns the diagnostics of new entities that cannot get
        /// a body; `failed` keeps them from being retried every tick.
        pub fn sync(
            state: &mut State,
            world: &World,
            ids: &BTreeMap<u64, Entity>,
            scene_dir: &Path,
            failed: &mut std::collections::BTreeSet<u64>,
        ) -> Vec<Errors> {
            let gone: Vec<u64> = state
                .entities
                .keys()
                .filter(|id| !ids.contains_key(id))
                .copied()
                .collect();
            for id in gone {
                if let Some(h) = state.entities.remove(&id) {
                    state.world.remove_body(h.body);
                }
            }
            failed.retain(|id| ids.contains_key(id));
            let mut errors = Vec::new();
            for (&id, &e) in ids {
                if state.entities.contains_key(&id) || failed.contains(&id) {
                    continue;
                }
                if world.get::<ColliderComp>(e).is_none() && world.get::<BodyComp>(e).is_none() {
                    continue;
                }
                if let Err(err) = add(state, world, id, e, scene_dir) {
                    failed.insert(id);
                    errors.push(err);
                }
            }
            errors
        }

        /// Adds the body and collider of one entity, if it has a `Collider`.
        fn add(state: &mut State, world: &World, id: u64, e: Entity, scene_dir: &Path) -> Result<(), Errors> {
            {
                let Some(collider) = world.get::<ColliderComp>(e).cloned() else {
                    if world.get::<BodyComp>(e).is_some() {
                        return Err(err(
                            crate::codes::MISSING,
                            id,
                            format!("entity {id} has a RigidBody but no Collider"),
                            "add a Collider component",
                        ));
                    }
                    return Ok(());
                };
                if world.get::<Position>(e).is_some() {
                    return Err(err(
                        crate::codes::POSITION,
                        id,
                        format!("entity {id} has both a physics body and Position"),
                        "remove Position: physics moves the entity's Transform",
                    ));
                }
                let t = world.get::<Transform>(e).copied().unwrap_or_default();
                let tr = [t.translation.x, t.translation.y, t.translation.z];
                let rot = [t.rotation.x, t.rotation.y, t.rotation.z, t.rotation.w];
                let scale = [t.scale.x, t.scale.y, t.scale.z];
                let comp = world.get::<BodyComp>(e).copied();
                let character = world.get::<CharacterController>(e).is_some();
                let kind = match (character, comp.map(|b| b.kind)) {
                    (true, _) => 3,
                    (false, Some(BodyKind::Dynamic)) => 0,
                    (false, Some(BodyKind::Kinematic)) => 2,
                    (false, Some(BodyKind::Fixed)) | (false, None) => 1,
                };
                let mut b = match kind {
                    0 => RigidBodyBuilder::dynamic(),
                    1 => RigidBodyBuilder::fixed(),
                    _ => RigidBodyBuilder::kinematic_position_based(),
                }
                .pose(conv::pose(tr, rot))
                .user_data(id as u128);
                if let Some(c) = comp {
                    b = b
                        .linear_damping(c.linear_damping)
                        .angular_damping(c.angular_damping)
                        .gravity_scale(c.gravity_scale);
                    if c.lock_rotations || character {
                        b = b.lock_rotations();
                    }
                }
                if let Some(v) = world.get::<Velocity>(e) {
                    b = b.linvel(conv::vector([v.x, v.y, v.z]));
                }
                if let Some(w) = world.get::<AngularVelocity>(e) {
                    b = b.angvel(conv::angular([w.x, w.y, w.z]));
                }
                let shape = conv::shape(&collider.shape, scale, scene_dir).map_err(|m| {
                    Errors::from(
                        Diagnostic::error(crate::codes::SHAPE, m)
                            .entity(id)
                            .component("Collider"),
                    )
                })?;
                let mut c = shape
                    .friction(collider.friction)
                    .restitution(collider.restitution)
                    .density(collider.density)
                    .sensor(collider.sensor)
                    .active_events(ActiveEvents::COLLISION_EVENTS)
                    .user_data(id as u128);
                // Sensors report every body type, so characters (kinematic) and fixed or
                // kinematic pickups trigger them; solid pairs keep Rapier's default.
                if collider.sensor {
                    c = c.active_collision_types(ActiveCollisionTypes::all());
                }
                let (body, collider) = state.world.insert(b, c);
                state.entities.insert(
                    id,
                    Handles {
                        body,
                        collider,
                        kind,
                    },
                );
            }
            Ok(())
        }

        /// One fixed tick: kinematic targets, character controllers, the step, events, write-back.
        pub fn tick(
            state: &mut State,
            world: &mut World,
            ids: &BTreeMap<u64, Entity>,
            actions: &ActionState,
            dt: f32,
            gravity: [f32; 3],
            tick: u64,
        ) {
            let pw = &mut state.world;
            pw.integration_parameters.dt = dt;
            pw.gravity = conv::vector(gravity);
            let mut log: Vec<(&'static str, u64, Option<u64>)> = Vec::new();
            // Point gravity (decision 0230): empty unless the scene has a GravityWell.
            let wells = if conv::DIM == 3 {
                crate::planet::wells(world)
            } else {
                Vec::new()
            };

            for (&id, h) in &state.entities {
                let Some(&e) = ids.get(&id) else { continue };
                // A script that moved a dynamic body or a character (its Transform no longer
                // matches what physics wrote back) teleports it. Untouched bodies compare
                // equal bit for bit, so they are left alone.
                if h.kind == 0 || h.kind == 3 {
                    let t = world.get::<Transform>(e).copied().unwrap_or_default();
                    let want = conv::vector([t.translation.x, t.translation.y, t.translation.z]);
                    let body = &mut pw.bodies[h.body];
                    if body.translation() != want {
                        body.set_translation(want, true);
                        if h.kind == 3 {
                            body.set_next_kinematic_translation(want);
                        }
                    }
                }
                match h.kind {
                    2 => {
                        let t = world.get::<Transform>(e).copied().unwrap_or_default();
                        let p = conv::pose(
                            [t.translation.x, t.translation.y, t.translation.z],
                            [t.rotation.x, t.rotation.y, t.rotation.z, t.rotation.w],
                        );
                        pw.bodies[h.body].set_next_kinematic_position(p);
                    }
                    3 => {
                        let Some(cc) = world.get::<CharacterController>(e).cloned() else {
                            continue;
                        };
                        let mut cs = world.get::<CharacterState>(e).copied().unwrap_or_default();
                        let mut planet_state: Option<crate::planet::PlanetState> = None;
                        let mut vy = cs.vertical_velocity - cc.gravity * dt;
                        let scripted = world.get::<CharacterInput>(e).copied();
                        let (move_x, move_forward, jump) = match scripted {
                            Some(i) => (
                                i.move_x.clamp(-1.0, 1.0),
                                i.move_forward.clamp(-1.0, 1.0),
                                i.jump,
                            ),
                            None => (
                                actions.value(&cc.move_x),
                                actions.value(&cc.move_forward),
                                actions.just_pressed(&cc.jump),
                            ),
                        };
                        if scripted.is_some_and(|i| i.jump)
                            && let Some(mut i) = world.get_mut::<CharacterInput>(e)
                        {
                            i.jump = false;
                        }
                        let planet = if conv::DIM == 3 {
                            world.get::<PlanetCharacter>(e).cloned()
                        } else {
                            None
                        };
                        let t = world.get::<Transform>(e).copied().unwrap_or_default();
                        let q = [t.rotation.x, t.rotation.y, t.rotation.z, t.rotation.w];
                        let mut controller = KinematicCharacterController::default();
                        let pose = *pw.bodies[h.body].position();
                        let desired = if let Some(pc) = &planet {
                            // Round worlds: up follows gravity (decision 0230).
                            let intent = world.get::<MoveIntent>(e).copied();
                            if intent.is_some_and(|i| i.jump)
                                && let Some(mut i) = world.get_mut::<MoveIntent>(e)
                            {
                                i.jump = false;
                            }
                            let tr = conv::to3(pose.translation);
                            let step = crate::planet::Step {
                                rotation: q,
                                position: [tr[0], tr[1], tr[2]],
                                walk: [move_x, move_forward],
                                intent: intent.map(|i| [i.velocity.x, i.velocity.y, i.velocity.z]),
                                jump: jump || intent.is_some_and(|i| i.jump),
                                thrust_held: !pc.thrust_action.is_empty()
                                    && actions.value(&pc.thrust_action) > 0.5,
                                vertical_velocity: cs.vertical_velocity,
                                grounded: cs.grounded,
                                speed: cc.speed,
                                jump_speed: cc.jump_speed,
                                fallback_gravity: cc.gravity,
                                dt,
                            };
                            let plan = crate::planet::plan(pc, &wells, &step);
                            if plan.jumped {
                                log.push(("jump", id, None));
                            }
                            vy = plan.vertical_velocity;
                            controller.up = conv::vector(plan.up).normalize_or_zero();
                            let p = conv::pose(tr, plan.rotation);
                            pw.bodies[h.body].set_next_kinematic_rotation(p.rotation);
                            planet_state = Some(plan.state);
                            conv::vector(plan.translation)
                        } else {
                            if cs.grounded && jump {
                                vy = cc.jump_speed;
                                log.push(("jump", id, None));
                            }
                            conv::turn(&mut pw.bodies[h.body], q);
                            conv::face(
                                q,
                                conv::walk(
                                    move_x * cc.speed * dt,
                                    move_forward * cc.speed * dt,
                                    vy * dt,
                                ),
                            )
                        };
                        // The controller casts from the rotated pose so the collider is
                        // upright on round worlds.
                        let pose = match &planet_state {
                            Some(_) => {
                                let r = pw.bodies[h.body].next_position().rotation;
                                let mut p = pose;
                                p.rotation = r;
                                p
                            }
                            None => pose,
                        };
                        let shape = pw.colliders[h.collider].shared_shape().clone();
                        let mut collisions = Vec::new();
                        let movement = {
                            let filter = QueryFilter::default()
                                .exclude_rigid_body(h.body)
                                .exclude_sensors();
                            let queries = pw.broad_phase.as_query_pipeline(
                                pw.narrow_phase.query_dispatcher(),
                                &pw.bodies,
                                &pw.colliders,
                                filter,
                            );
                            controller.move_shape(dt, &queries, shape.as_ref(), &pose, desired, |c| {
                                collisions.push(c)
                            })
                        };
                        cs.grounded = movement.grounded;
                        if movement.grounded && vy < 0.0 {
                            vy = 0.0;
                        }
                        cs.vertical_velocity = vy;
                        let target = pose.translation + movement.translation;
                        pw.bodies[h.body].set_next_kinematic_translation(target);
                        {
                            let filter = QueryFilter::default()
                                .exclude_rigid_body(h.body)
                                .exclude_sensors();
                            let mut queries = pw.broad_phase.as_query_pipeline_mut(
                                pw.narrow_phase.query_dispatcher(),
                                &mut pw.bodies,
                                &mut pw.colliders,
                                filter,
                            );
                            controller.solve_character_collision_impulses(
                                dt,
                                &mut queries,
                                shape.as_ref(),
                                cc.mass,
                                &collisions,
                            );
                        }
                        if let Some(ps) = planet_state {
                            if let Some(mut s) = world.get_mut::<crate::planet::PlanetState>(e) {
                                *s = ps;
                            } else {
                                world.entity_mut(e).insert(ps);
                            }
                        }
                        if let Some(mut s) = world.get_mut::<CharacterState>(e) {
                            *s = cs;
                        } else {
                            world.entity_mut(e).insert(cs);
                        }
                    }
                    _ => {}
                }
            }

            if !wells.is_empty() {
                for h in state.entities.values() {
                    if h.kind != 0 {
                        continue;
                    }
                    let body = &mut pw.bodies[h.body];
                    let p = conv::to3(body.translation());
                    let v = conv::to3(body.linvel());
                    let f = crate::planet::body_force(&wells, p, v, body.mass(), body.gravity_scale());
                    body.reset_forces(false);
                    body.add_force(conv::vector(f), false);
                }
            }

            let (cs, cr) = std::sync::mpsc::channel();
            let (fs, _fr) = std::sync::mpsc::channel();
            let (ts, _tr) = std::sync::mpsc::channel();
            let collector = ChannelEventCollector::new(cs, fs, ts);
            pw.step_with_events(&(), &collector);
            while let Ok(ev) = cr.try_recv() {
                let a = pw.colliders.get(ev.collider1()).map(|c| c.user_data as u64);
                let b = pw.colliders.get(ev.collider2()).map(|c| c.user_data as u64);
                if let (Some(a), Some(b)) = (a, b) {
                    let kind = if ev.started() {
                        "collision_started"
                    } else {
                        "collision_stopped"
                    };
                    log.push((kind, a.min(b), Some(a.max(b))));
                }
            }

            for (&id, h) in &state.entities {
                if h.kind == 1 {
                    continue;
                }
                let Some(&e) = ids.get(&id) else { continue };
                let body = &pw.bodies[h.body];
                if let Some(mut t) = world.get_mut::<Transform>(e) {
                    let (tr, rot) = conv::from_pose(body.position());
                    t.translation.x = tr[0];
                    t.translation.y = tr[1];
                    if conv::DIM == 3 {
                        t.translation.z = tr[2];
                    }
                    t.rotation.x = rot[0];
                    t.rotation.y = rot[1];
                    t.rotation.z = rot[2];
                    t.rotation.w = rot[3];
                }
                if let Some(mut v) = world.get_mut::<Velocity>(e) {
                    let l = conv::to3(body.linvel());
                    v.x = l[0];
                    v.y = l[1];
                    v.z = l[2];
                }
                if let Some(mut w) = world.get_mut::<AngularVelocity>(e) {
                    let a = conv::angular_to3(body);
                    w.x = a[0];
                    w.y = a[1];
                    w.z = a[2];
                }
            }

            if let Some(mut events) = world.get_resource_mut::<EventLog>() {
                for (kind, a, b) in log {
                    events.push(tick, kind, Some(a), b, serde_json::Value::Null);
                }
            }
        }

        /// First collider hit by a ray: (scene id, distance, normal).
        pub fn raycast(
            state: &State,
            origin: [f32; 3],
            dir: [f32; 3],
            max: f32,
            filter: marionette_core::module::RayFilter,
        ) -> Option<(u64, f32, [f32; 3])> {
            let ray = Ray::new(conv::vector(origin), conv::vector(dir).normalize_or_zero());
            let mut q = QueryFilter::default();
            if filter.skip_sensors {
                q = q.exclude_sensors();
            }
            let skip = |_: ColliderHandle, c: &Collider| filter.exclude != Some(c.user_data as u64);
            if filter.exclude.is_some() {
                q = q.predicate(&skip);
            }
            let (handle, hit) = state.world.cast_ray_and_get_normal(&ray, max, true, q)?;
            let id = state.world.colliders.get(handle)?.user_data as u64;
            Some((id, hit.time_of_impact, conv::to3(hit.normal)))
        }
    };
}

pub(crate) use backend;
