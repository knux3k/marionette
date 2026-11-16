//! The physics backend, written once and expanded for Rapier 3D (`d3`) and 2D (`d2`). Each
//! expansion site provides a `conv` module with the few functions that differ by dimension.

macro_rules! backend {
    () => {
        use crate::BodyTracking;
        use crate::components::{
            AngularVelocity, BodyKind, CharacterController, CharacterInput, CharacterPlaced, CharacterState,
            Collider as ColliderComp, ColliderShape, CollisionLayers, ContinuousCollision, Joint,
            RigidBody as BodyComp,
        };
        use crate::game::{BodyState, Contact, GAME_BIT, GameBody, Hit, QueryOptions, Target};
        use crate::planet::PlanetCharacter;
        use bevy_ecs::entity::Entity;
        use bevy_ecs::world::World;
        use marionette_core::actions::ActionState;
        use marionette_core::builtin::MoveIntent;
        use marionette_core::builtin::{Position, Velocity};
        use marionette_core::mesh::GameMeshes;
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

        /// Everything physics knows. Serialized whole for snapshots (`extra` after the rest,
        /// so snapshots written before it existed still load).
        #[derive(Serialize, Deserialize)]
        pub struct State {
            pub world: PhysicsWorld,
            /// Scene id -> Rapier handles, in id order.
            pub entities: BTreeMap<u64, Handles>,
            #[serde(skip)]
            pub extra: Extra,
        }

        /// Joints, game bodies and forces (decisions 0310, 0311). The joints and bodies
        /// themselves live in `world`; these maps name them.
        #[derive(Serialize, Deserialize, Default)]
        pub struct Extra {
            /// Scene id of the entity with the `Joint` -> its Rapier joint.
            pub joints: BTreeMap<u64, ImpulseJointHandle>,
            /// A fixed body at the origin that joints to the world attach to.
            pub world_body: Option<RigidBodyHandle>,
            /// Game body key -> Rapier handles, in key order.
            pub game: BTreeMap<u64, Handles>,
            /// Forces were set last step: clear them before this one.
            pub forced: bool,
            /// What game bodies touched in the last step (an output, not state).
            #[serde(skip)]
            pub contacts: Vec<Contact>,
            /// Forces and impulses for the coming step, read from components just before it.
            #[serde(skip)]
            pub pushes: crate::Pushes,
        }

        /// A collider without `CollisionLayers` is on layer 1 and touches every layer
        /// (Unity's Default layer), so a query for other layers leaves it out.
        const DEFAULT_LAYERS: CollisionLayers = CollisionLayers {
            member: 1,
            collides_with: u32::MAX,
        };

        fn groups(l: CollisionLayers) -> InteractionGroups {
            InteractionGroups::new(
                Group::from_bits_truncate(l.member),
                Group::from_bits_truncate(l.collides_with),
                InteractionTestMode::And,
            )
        }

        fn err(code: &'static str, id: u64, msg: String, fix: &str) -> Errors {
            Diagnostic::error(code, msg)
                .entity(id)
                .fix(fix.to_string())
                .into()
        }

        /// Builds the physics world from the scene's components (bodies in id order). Bodies
        /// whose collider names a game mesh that does not exist yet wait for it (`sync`).
        pub fn build(
            world: &mut World,
            ids: &BTreeMap<u64, Entity>,
            scene_dir: &Path,
            track: &mut BodyTracking,
        ) -> Result<State, Errors> {
            let mut state = State {
                world: PhysicsWorld::default(),
                entities: BTreeMap::new(),
                extra: Extra::default(),
            };
            for (&id, &e) in ids {
                add(&mut state, world, id, e, scene_dir, track)?;
            }
            Ok(state)
        }

        /// Follows entities spawned or despawned since the last tick (scripts do this):
        /// bodies of entities that are gone are removed, new entities with a `Collider` get
        /// a body, both in id order, and colliders built from a game mesh that was stored
        /// again get the new shape. Returns the diagnostics of new entities that cannot get
        /// a body; `track.failed` keeps them from being retried every tick.
        pub fn sync(
            state: &mut State,
            world: &World,
            ids: &BTreeMap<u64, Entity>,
            scene_dir: &Path,
            track: &mut BodyTracking,
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
                track.meshes.remove(&id);
            }
            track.failed.retain(|id| ids.contains_key(id));
            let mut errors = Vec::new();
            for (&id, &e) in ids {
                if state.entities.contains_key(&id) || track.failed.contains(&id) {
                    continue;
                }
                if world.get::<ColliderComp>(e).is_none() && world.get::<BodyComp>(e).is_none() {
                    continue;
                }
                if let Err(err) = add(state, world, id, e, scene_dir, track) {
                    track.failed.insert(id);
                    errors.push(err);
                }
            }
            if let Some(meshes) = world.get_resource::<GameMeshes>() {
                remesh(state, world, ids, scene_dir, meshes, track, &mut errors);
            }
            errors
        }

        /// Gives colliders built from a game mesh the mesh's current shape when it was stored
        /// again since they were built. A removed mesh leaves the last shape in place.
        fn remesh(
            state: &mut State,
            world: &World,
            ids: &BTreeMap<u64, Entity>,
            scene_dir: &Path,
            meshes: &GameMeshes,
            track: &mut BodyTracking,
            errors: &mut Vec<Errors>,
        ) {
            let current = |id: &u64| {
                let e = *ids.get(id)?;
                let c = world.get::<ColliderComp>(e)?;
                Some((e, c, meshes.version(c.shape.game_mesh()?)?))
            };
            // After a snapshot restore the saved shapes are the built ones: take the versions
            // as they are now instead of rebuilding (a rebuild could reorder contact pairs).
            if track.adopt {
                track.adopt = false;
                for id in state.entities.keys() {
                    if let Some((_, _, v)) = current(id) {
                        track.meshes.insert(*id, v);
                    }
                }
                return;
            }
            let mut changed = false;
            for (id, built) in track.meshes.iter_mut() {
                let (Some((e, c, v)), Some(h)) = (current(id), state.entities.get(id)) else {
                    continue;
                };
                if v == *built {
                    continue;
                }
                *built = v;
                let t = world.get::<Transform>(e).copied().unwrap_or_default();
                match conv::shape(
                    &c.shape,
                    [t.scale.x, t.scale.y, t.scale.z],
                    scene_dir,
                    Some(meshes),
                ) {
                    Ok(b) => {
                        if let Some(col) = state.world.colliders.get_mut(h.collider) {
                            col.set_shape(b.shape);
                            changed = true;
                        }
                    }
                    Err(m) => errors.push(
                        Diagnostic::error(crate::codes::SHAPE, m)
                            .entity(*id)
                            .component("Collider")
                            .into(),
                    ),
                }
            }
            // Bodies asleep on the old surface would float over a dug hole: wake them all
            // once (mesh changes are rare, edits to terrain at most).
            if changed {
                state.world.wake_up_all(true);
            }
        }

        /// Adds the body and collider of one entity, if it has a `Collider` (and, for a game
        /// mesh shape, once the mesh exists).
        fn add(
            state: &mut State,
            world: &World,
            id: u64,
            e: Entity,
            scene_dir: &Path,
            track: &mut BodyTracking,
        ) -> Result<(), Errors> {
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
                let meshes = world.get_resource::<GameMeshes>();
                let mesh_version = match collider.shape.game_mesh() {
                    Some(name) if conv::DIM == 3 => match meshes.and_then(|m| m.version(name)) {
                        Some(v) => Some(v),
                        // A module often stores its meshes after physics has loaded: wait,
                        // and try again next tick.
                        None => return Ok(()),
                    },
                    _ => None,
                };
                let t = world.get::<Transform>(e).copied().unwrap_or_default();
                let tr = [t.translation.x, t.translation.y, t.translation.z];
                let rot = [t.rotation.x, t.rotation.y, t.rotation.z, t.rotation.w];
                let scale = [t.scale.x, t.scale.y, t.scale.z];
                let comp = world.get::<BodyComp>(e).copied();
                let character = world.get::<CharacterController>(e).is_some();
                // A child (decision 0272) is placed by its parent every tick, so physics
                // cannot also move it; a bare collider on a child follows it as a kinematic body.
                let child = world
                    .get::<marionette_core::hierarchy::ChildOf>(e)
                    .is_some();
                let kind = match (character, comp.map(|b| b.kind)) {
                    (true, _) => 3,
                    (false, Some(BodyKind::Dynamic)) => 0,
                    (false, Some(BodyKind::Kinematic)) => 2,
                    (false, None) if child => 2,
                    (false, Some(BodyKind::Fixed)) | (false, None) => 1,
                };
                if child && (kind == 0 || kind == 3) {
                    return Err(err(
                        crate::codes::CHILD,
                        id,
                        format!("entity {id} has ChildOf and a dynamic body or character controller"),
                        "remove ChildOf, or make the RigidBody Kinematic: a child is placed by its parent",
                    ));
                }
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
                if kind == 0
                    && world
                        .get::<ContinuousCollision>(e)
                        .is_some_and(|c| c.enabled)
                {
                    b = b.ccd_enabled(true);
                }
                let shape = conv::shape(&collider.shape, scale, scene_dir, meshes).map_err(|m| {
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
                c = c.collision_groups(groups(
                    world
                        .get::<CollisionLayers>(e)
                        .copied()
                        .unwrap_or(DEFAULT_LAYERS),
                ));
                let (body, collider) = state.world.insert(b, c);
                if let Some(v) = mesh_version {
                    track.meshes.insert(id, v);
                }
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
            let pushes = std::mem::take(&mut state.extra.pushes);
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

            // A script that set a dynamic body's Velocity or AngularVelocity since the last
            // write-back (a throw, a held object moved by velocity) sets the body's velocity.
            // Checked before character controllers run, so their pushes this tick stay.
            for (&id, h) in &state.entities {
                if h.kind != 0 {
                    continue;
                }
                let Some(&e) = ids.get(&id) else { continue };
                if let Some(v) = world.get::<Velocity>(e) {
                    let want = [v.x, v.y, v.z];
                    let body = &mut pw.bodies[h.body];
                    if conv::to3(body.linvel()) != want {
                        body.set_linvel(conv::vector(want), true);
                    }
                }
                if let Some(w) = world.get::<AngularVelocity>(e) {
                    let want = [w.x, w.y, w.z];
                    let body = &mut pw.bodies[h.body];
                    if conv::angular_to3(body) != want {
                        body.set_angvel(conv::angular(want), true);
                    }
                }
            }

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
                        // Placed by game code this step (decision 0312): the body goes where
                        // the Transform says, turned as it says, and is not walked.
                        if world.get::<CharacterPlaced>(e).is_some() {
                            let t = world.get::<Transform>(e).copied().unwrap_or_default();
                            let p = conv::pose(
                                [t.translation.x, t.translation.y, t.translation.z],
                                [t.rotation.x, t.rotation.y, t.rotation.z, t.rotation.w],
                            );
                            pw.bodies[h.body].set_next_kinematic_position(p);
                            continue;
                        }
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
                            // A `MoveIntent` (navigation, an XR rig riding the body) replaces
                            // the walk input with a world velocity, capped at the speed.
                            let intent = world.get::<MoveIntent>(e).copied();
                            if intent.is_some_and(|i| i.jump)
                                && let Some(mut i) = world.get_mut::<MoveIntent>(e)
                            {
                                i.jump = false;
                            }
                            if cs.grounded && (jump || intent.is_some_and(|i| i.jump)) {
                                vy = cc.jump_speed;
                                log.push(("jump", id, None));
                            }
                            conv::turn(&mut pw.bodies[h.body], q);
                            match intent {
                                Some(i) => {
                                    let x = i.velocity.x;
                                    let z = if conv::DIM == 3 { i.velocity.z } else { 0.0 };
                                    let l = (x * x + z * z).sqrt();
                                    let k = if l > cc.speed && l > 0.0 {
                                        cc.speed / l
                                    } else {
                                        1.0
                                    };
                                    conv::walk(x * k * dt, -z * k * dt, vy * dt)
                                }
                                None => conv::face(
                                    q,
                                    conv::walk(
                                        move_x * cc.speed * dt,
                                        move_forward * cc.speed * dt,
                                        vy * dt,
                                    ),
                                ),
                            }
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
                        // A character on layers moves through what its layers do not touch.
                        let layers = world.get::<CollisionLayers>(e).copied();
                        let mut collisions = Vec::new();
                        let movement = {
                            let mut filter = QueryFilter::default()
                                .exclude_rigid_body(h.body)
                                .exclude_sensors();
                            if let Some(l) = layers {
                                filter = filter.groups(groups(l));
                            }
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
                            let mut filter = QueryFilter::default()
                                .exclude_rigid_body(h.body)
                                .exclude_sensors();
                            if let Some(l) = layers {
                                filter = filter.groups(groups(l));
                            }
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

            // Point gravity and `ExternalForce`s are set afresh every step; a step after one
            // that had forces clears them, so a removed force stops at once.
            let forced = !wells.is_empty() || !pushes.forces.is_empty();
            if forced || state.extra.forced {
                let game = state.extra.game.values();
                for h in state.entities.values().chain(game) {
                    if h.kind != 0 {
                        continue;
                    }
                    let body = &mut pw.bodies[h.body];
                    body.reset_forces(false);
                    body.reset_torques(false);
                    if !wells.is_empty() {
                        let p = conv::to3(body.translation());
                        let v = conv::to3(body.linvel());
                        let f = crate::planet::body_force(&wells, p, v, body.mass(), body.gravity_scale());
                        body.add_force(conv::vector(f), false);
                    }
                }
                for (id, force, torque) in &pushes.forces {
                    let Some(h) = state.entities.get(id).filter(|h| h.kind == 0) else {
                        continue;
                    };
                    let body = &mut pw.bodies[h.body];
                    body.add_force(conv::vector(*force), true);
                    conv::add_torque(body, *torque);
                }
            }
            state.extra.forced = forced;
            for (id, linear, angular) in &pushes.impulses {
                let Some(h) = state.entities.get(id).filter(|h| h.kind == 0) else {
                    continue;
                };
                let body = &mut pw.bodies[h.body];
                body.apply_impulse(conv::vector(*linear), true);
                conv::torque_impulse(body, *angular);
            }

            let (cs, cr) = std::sync::mpsc::channel();
            let (fs, _fr) = std::sync::mpsc::channel();
            let (ts, _tr) = std::sync::mpsc::channel();
            let collector = ChannelEventCollector::new(cs, fs, ts);
            pw.step_with_events(&(), &collector);
            state.extra.contacts.clear();
            while let Ok(ev) = cr.try_recv() {
                let a = pw
                    .colliders
                    .get(ev.collider1())
                    .map(|c| crate::game::target(c.user_data));
                let b = pw
                    .colliders
                    .get(ev.collider2())
                    .map(|c| crate::game::target(c.user_data));
                // A game body's contacts go to the game (`body_contacts`), not the event log,
                // which names scene entities only.
                for (x, y) in [(a, b), (b, a)] {
                    if let (Some(Target::Body(k)), Some(other)) = (x, y) {
                        state.extra.contacts.push(Contact {
                            body: k,
                            other,
                            started: ev.started(),
                        });
                    }
                }
                if let (Some(Target::Entity(a)), Some(Target::Entity(b))) = (a, b) {
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
            // Game bodies (decision 0311) are the game's own: scripts and this ray see
            // scene entities only.
            let skip = |_: ColliderHandle, c: &Collider| {
                c.user_data & GAME_BIT == 0 && filter.exclude != Some(c.user_data as u64)
            };
            q = q.predicate(&skip);
            let (handle, hit) = state.world.cast_ray_and_get_normal(&ray, max, true, q)?;
            let id = state.world.colliders.get(handle)?.user_data as u64;
            Some((id, hit.time_of_impact, conv::to3(hit.normal)))
        }

        /// Creates the joints whose two bodies now exist, and drops joints whose component
        /// went away or changed, or whose bodies are gone (a changed joint is built again).
        /// `joints` is every `Joint` component in scene id order.
        pub fn sync_joints(state: &mut State, joints: &[(u64, Joint)], track: &mut BodyTracking) {
            if track.adopt_joints {
                // Restored from a snapshot: the saved joints are the built ones.
                track.adopt_joints = false;
                track.joints = joints
                    .iter()
                    .filter(|(id, _)| state.extra.joints.contains_key(id))
                    .copied()
                    .collect();
            }
            let stale: Vec<u64> = state
                .extra
                .joints
                .keys()
                .filter(|id| {
                    let now = joints.iter().find(|(j, _)| j == *id).map(|(_, j)| j);
                    let alive = state.entities.contains_key(id)
                        && now.is_some_and(|j| j.other == 0 || state.entities.contains_key(&j.other));
                    !alive || now != track.joints.get(id)
                })
                .copied()
                .collect();
            for id in stale {
                if let Some(h) = state.extra.joints.remove(&id) {
                    // Gone already when one of its bodies was removed.
                    state.world.remove_impulse_joint(h);
                }
                track.joints.remove(&id);
            }
            for (id, j) in joints {
                if state.extra.joints.contains_key(id) || *id == j.other {
                    continue;
                }
                let Some(a) = state.entities.get(id).map(|h| h.body) else {
                    continue;
                };
                let b = if j.other == 0 {
                    match state.extra.world_body {
                        Some(b) => b,
                        None => {
                            let b = state.world.insert_body(RigidBodyBuilder::fixed());
                            state.extra.world_body = Some(b);
                            b
                        }
                    }
                } else {
                    match state.entities.get(&j.other) {
                        Some(h) => h.body,
                        None => continue,
                    }
                };
                let anchor = [j.anchor.x, j.anchor.y, j.anchor.z];
                let other = [j.other_anchor.x, j.other_anchor.y, j.other_anchor.z];
                // The other body is Rapier's first body and this one the second, so a motor's
                // positive speed and the limits are this body's turn relative to the other
                // (Unity's connected body is the reference). `rel` keeps the bodies' current
                // relative rotation as the joint's zero.
                let rel = state.world.bodies[b].rotation().inverse() * *state.world.bodies[a].rotation();
                let joint = conv::joint(&j.kind, other, anchor, j.collide_connected, rel);
                let h = state.world.insert_impulse_joint(b, a, joint);
                state.extra.joints.insert(*id, h);
                track.joints.insert(*id, *j);
            }
        }

        /// Adds (or replaces) a game body.
        pub fn add_game(
            state: &mut State,
            key: u64,
            d: &GameBody,
            scene_dir: &Path,
            meshes: Option<&GameMeshes>,
        ) -> Result<(), String> {
            let shape = conv::shape(&d.shape, [1.0, 1.0, 1.0], scene_dir, meshes)?;
            remove_game(state, key);
            let (mut b, kind) = match d.kind {
                BodyKind::Dynamic => (RigidBodyBuilder::dynamic(), 0),
                BodyKind::Fixed => (RigidBodyBuilder::fixed(), 1),
                BodyKind::Kinematic => (RigidBodyBuilder::kinematic_position_based(), 2),
            };
            b = b
                .pose(conv::pose(d.position, d.rotation))
                .linvel(conv::vector(d.velocity))
                .angvel(conv::angular(d.angular_velocity))
                .linear_damping(d.linear_damping)
                .angular_damping(d.angular_damping)
                .gravity_scale(d.gravity_scale)
                .ccd_enabled(d.ccd && kind == 0)
                .user_data(GAME_BIT | key as u128);
            let mut c = shape
                .friction(d.friction)
                .restitution(d.restitution)
                .sensor(d.sensor)
                .active_events(ActiveEvents::COLLISION_EVENTS)
                .user_data(GAME_BIT | key as u128);
            c = if d.mass > 0.0 {
                c.mass(d.mass)
            } else {
                c.density(d.density)
            };
            c = c.collision_groups(groups(d.layers.unwrap_or(DEFAULT_LAYERS)));
            let (body, collider) = state.world.insert(b, c);
            state.extra.game.insert(
                key,
                Handles {
                    body,
                    collider,
                    kind,
                },
            );
            Ok(())
        }

        pub fn remove_game(state: &mut State, key: u64) -> bool {
            match state.extra.game.remove(&key) {
                Some(h) => state.world.remove_body(h.body).is_some(),
                None => false,
            }
        }

        pub fn game_state(state: &State, key: u64) -> Option<BodyState> {
            let h = state.extra.game.get(&key)?;
            let body = state.world.bodies.get(h.body)?;
            let (position, rotation) = conv::from_pose(body.position());
            Some(BodyState {
                position,
                rotation,
                velocity: conv::to3(body.linvel()),
                angular_velocity: conv::angular_to3(body),
                sleeping: body.is_sleeping(),
            })
        }

        pub fn game_keys(state: &State) -> Vec<u64> {
            state.extra.game.keys().copied().collect()
        }

        pub fn game_contacts(state: &State) -> Vec<Contact> {
            state.extra.contacts.clone()
        }

        fn game_body(state: &mut State, key: u64) -> Option<&mut RigidBody> {
            let h = state.extra.game.get(&key)?;
            state.world.bodies.get_mut(h.body)
        }

        pub fn set_game_velocity(
            state: &mut State,
            key: u64,
            v: Option<[f32; 3]>,
            w: Option<[f32; 3]>,
        ) -> bool {
            let Some(body) = game_body(state, key) else {
                return false;
            };
            if let Some(v) = v {
                body.set_linvel(conv::vector(v), true);
            }
            if let Some(w) = w {
                body.set_angvel(conv::angular(w), true);
            }
            true
        }

        pub fn set_game_pose(state: &mut State, key: u64, p: [f32; 3], q: [f32; 4]) -> bool {
            let Some(body) = game_body(state, key) else {
                return false;
            };
            let pose = conv::pose(p, q);
            if body.is_kinematic() {
                body.set_next_kinematic_position(pose);
            } else {
                body.set_position(pose, true);
            }
            true
        }

        pub fn set_game_gravity(state: &mut State, key: u64, scale: f32) -> bool {
            let Some(body) = game_body(state, key) else {
                return false;
            };
            if body.gravity_scale() != scale {
                body.set_gravity_scale(scale, true);
            }
            true
        }

        pub fn push_game(state: &mut State, key: u64, impulse: [f32; 3]) -> bool {
            let Some(body) = game_body(state, key) else {
                return false;
            };
            body.apply_impulse(conv::vector(impulse), true);
            true
        }

        /// The query filter of `QueryOptions`. The predicate is passed in so it outlives the
        /// filter that borrows it.
        fn query_filter<'a>(
            o: &QueryOptions,
            pred: &'a dyn Fn(ColliderHandle, &Collider) -> bool,
        ) -> QueryFilter<'a> {
            let mut q = QueryFilter {
                predicate: Some(pred),
                ..QueryFilter::default()
            };
            if o.skip_sensors {
                q = q.exclude_sensors();
            }
            if o.layers != 0 {
                q = q.groups(InteractionGroups::new(
                    Group::ALL,
                    Group::from_bits_truncate(o.layers),
                    InteractionTestMode::And,
                ));
            }
            q
        }

        fn keep(o: QueryOptions) -> impl Fn(ColliderHandle, &Collider) -> bool {
            move |_, c| {
                let t = crate::game::target(c.user_data);
                !(o.skip_bodies && matches!(t, Target::Body(_))) && o.exclude != Some(t)
            }
        }

        fn query_shape(s: &ColliderShape) -> Result<SharedShape, String> {
            match s {
                ColliderShape::ConvexHull { .. } | ColliderShape::TriMesh { .. } => {
                    Err("queries take Cuboid, RoundCuboid, Ball, Capsule or Cylinder shapes".to_string())
                }
                _ => Ok(conv::shape(s, [1.0, 1.0, 1.0], Path::new(""), None)?.shape),
            }
        }

        pub fn overlap(
            state: &State,
            shape: &ColliderShape,
            p: [f32; 3],
            q: [f32; 4],
            o: QueryOptions,
        ) -> Result<Vec<Target>, String> {
            let shape = query_shape(shape)?;
            let pred = keep(o);
            let filter = query_filter(&o, &pred);
            let mut out: Vec<Target> = state
                .world
                .intersect_shape(conv::pose(p, q), shape.as_ref(), filter)
                .map(|(_, c)| crate::game::target(c.user_data))
                .collect();
            out.sort_unstable();
            out.dedup();
            Ok(out)
        }

        pub fn shape_cast(
            state: &State,
            shape: &ColliderShape,
            p: [f32; 3],
            q: [f32; 4],
            dir: [f32; 3],
            max: f32,
            o: QueryOptions,
        ) -> Result<Option<Hit>, String> {
            let shape = query_shape(shape)?;
            let pred = keep(o);
            let filter = query_filter(&o, &pred);
            let d = conv::vector(dir).normalize_or_zero();
            let opts = conv::ShapeCastOptions {
                max_time_of_impact: max,
                target_distance: 0.0,
                stop_at_penetration: true,
                compute_impact_geometry_on_penetration: true,
            };
            let hit = state
                .world
                .cast_shape(&conv::pose(p, q), d, shape.as_ref(), opts, filter);
            Ok(hit.and_then(|(handle, h)| {
                let c = state.world.colliders.get(handle)?;
                // witness1 and normal1 are on the hit collider, in world space.
                Some(Hit {
                    target: crate::game::target(c.user_data),
                    distance: h.time_of_impact,
                    point: conv::to3(h.witness1),
                    normal: conv::to3(h.normal1),
                })
            }))
        }

        /// Hits along a ray, nearest first: all of them, or only the nearest.
        pub fn ray_hits(
            state: &State,
            origin: [f32; 3],
            dir: [f32; 3],
            max: f32,
            o: QueryOptions,
            all: bool,
        ) -> Vec<Hit> {
            let d = conv::vector(dir).normalize_or_zero();
            let ray = Ray::new(conv::vector(origin), d);
            let pred = keep(o);
            let filter = query_filter(&o, &pred);
            let hit = |c: &Collider, toi: f32, n: Vector| Hit {
                target: crate::game::target(c.user_data),
                distance: toi,
                point: conv::to3(ray.point_at(toi)),
                normal: conv::to3(n),
            };
            if !all {
                return state
                    .world
                    .cast_ray_and_get_normal(&ray, max, true, filter)
                    .and_then(|(h, i)| {
                        Some(hit(
                            state.world.colliders.get(h)?,
                            i.time_of_impact,
                            i.normal,
                        ))
                    })
                    .into_iter()
                    .collect();
            }
            let mut out: Vec<Hit> = state
                .world
                .intersect_ray(ray, max, true, filter)
                .map(|(_, c, i)| hit(c, i.time_of_impact, i.normal))
                .collect();
            out.sort_by(|a, b| {
                a.distance
                    .total_cmp(&b.distance)
                    .then(a.target.cmp(&b.target))
            });
            out
        }
    };
}

pub(crate) use backend;
