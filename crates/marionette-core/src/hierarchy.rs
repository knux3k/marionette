//! Parent and child placement (decision 0272).
//!
//! `Transform` is always where an entity is in the world, so the renderer, physics, audio and
//! every other reader stay as they are. An entity with [`ChildOf`] is placed relative to
//! another entity: the engine writes its `Transform` from the parent's `Transform` and the
//! local placement in `ChildOf`, when a scene is loaded and twice per tick (before physics
//! and after it). To move a child, change its `ChildOf`; to move it with its parent, move the
//! parent.
//!
//! Only `+ - *` and `/` are used (determinism rules in ARCHITECTURE.md), and children are
//! placed parents first, then by id.

use crate::builtin::{Position, SceneRef};
use crate::error::{Diagnostic, Errors, Result, codes};
use crate::scene::SceneId;
use crate::spatial::{Quat, Transform, Vec3};
use crate::vmath::{qconj, qmul, qrot};
use bevy_ecs::prelude::*;
use bevy_reflect::Reflect;
use std::collections::{BTreeMap, HashMap};

/// Places the entity relative to `parent`: the engine sets this entity's `Transform` to the
/// parent's `Transform` (its `Position`, if it has one, as the translation) combined with
/// `translation`, `rotation` and `scale` here, at load and every tick. Change these fields to
/// move the entity; a `Transform` written directly is overwritten. Scales multiply per axis,
/// so a turned child of a parent with unequal scales is not sheared. Not for dynamic bodies
/// or characters, which physics moves. Use `entity_attach` to attach an entity where it is.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq)]
pub struct ChildOf {
    pub parent: SceneRef,
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

fn q(r: Quat) -> [f32; 4] {
    [r.x, r.y, r.z, r.w]
}

/// The world placement of a child with local placement `c` under a parent at `p`.
pub fn compose(p: &Transform, c: &ChildOf) -> Transform {
    let pr = q(p.rotation);
    let t = qrot(
        pr,
        [
            p.scale.x * c.translation.x,
            p.scale.y * c.translation.y,
            p.scale.z * c.translation.z,
        ],
    );
    let r = qmul(pr, q(c.rotation));
    Transform {
        translation: Vec3::new(
            p.translation.x + t[0],
            p.translation.y + t[1],
            p.translation.z + t[2],
        ),
        rotation: Quat {
            x: r[0],
            y: r[1],
            z: r[2],
            w: r[3],
        },
        scale: Vec3::new(
            p.scale.x * c.scale.x,
            p.scale.y * c.scale.y,
            p.scale.z * c.scale.z,
        ),
    }
}

/// The local placement that puts a child at `world` under a parent at `p` (the inverse of
/// [`compose`]). `None` when a scale of the parent is zero.
pub fn local(p: &Transform, world: &Transform, parent: SceneRef) -> Option<ChildOf> {
    if p.scale.x == 0.0 || p.scale.y == 0.0 || p.scale.z == 0.0 {
        return None;
    }
    let inv = qconj(q(p.rotation));
    let d = qrot(
        inv,
        [
            world.translation.x - p.translation.x,
            world.translation.y - p.translation.y,
            world.translation.z - p.translation.z,
        ],
    );
    let r = qmul(inv, q(world.rotation));
    Some(ChildOf {
        parent,
        translation: Vec3::new(d[0] / p.scale.x, d[1] / p.scale.y, d[2] / p.scale.z),
        rotation: Quat {
            x: r[0],
            y: r[1],
            z: r[2],
            w: r[3],
        },
        scale: Vec3::new(
            world.scale.x / p.scale.x,
            world.scale.y / p.scale.y,
            world.scale.z / p.scale.z,
        ),
    })
}

/// Where a parent is: its `Transform`, with `Position` as the translation if it has one (the
/// rule the renderer uses).
fn placement(world: &World, e: Entity) -> Transform {
    let mut t = world.get::<Transform>(e).copied().unwrap_or_default();
    if let Some(p) = world.get::<Position>(e) {
        t.translation = Vec3::new(p.x, p.y, p.z);
    }
    t
}

/// One child in placing order.
#[derive(Clone, Copy)]
struct Link {
    child: Entity,
    parent: Entity,
    parent_id: u64,
}

/// The children in placing order (parents first, then by id), kept between ticks while the
/// links stay the same.
#[derive(Default)]
pub struct Order {
    links: Vec<Link>,
}

type Children<'w> = QueryState<(Entity, &'w SceneId, &'w ChildOf)>;

impl Order {
    /// True while every kept link still holds and no child was added or removed.
    fn holds(&self, world: &World, count: usize) -> bool {
        count == self.links.len()
            && self.links.iter().all(|l| {
                world
                    .get::<ChildOf>(l.child)
                    .is_some_and(|c| c.parent.0 == l.parent_id)
                    && world
                        .get::<SceneId>(l.parent)
                        .is_some_and(|id| id.0 == l.parent_id)
            })
    }

    /// Finds the order again. Children whose chain of parents does not end at an entity
    /// without `ChildOf` (a missing parent, or a cycle) are left out and returned by id.
    fn rebuild(&mut self, world: &mut World, children: &mut Children) -> Vec<u64> {
        let kids: BTreeMap<u64, (Entity, u64)> = children
            .iter(world)
            .map(|(e, id, c)| (id.0, (e, c.parent.0)))
            .collect();
        let mut all = world.query::<(Entity, &SceneId)>();
        let ids: HashMap<u64, Entity> = all.iter(world).map(|(e, id)| (id.0, e)).collect();
        let mut placed: Vec<(usize, u64, Link)> = Vec::with_capacity(kids.len());
        let mut left_out = Vec::new();
        for (&id, &(child, parent_id)) in &kids {
            let (mut depth, mut at) = (0, parent_id);
            let root = loop {
                match kids.get(&at) {
                    Some(&(_, next)) if depth < kids.len() => {
                        depth += 1;
                        at = next;
                    }
                    Some(_) => break false, // a cycle
                    None => break ids.contains_key(&at),
                }
            };
            match (root, ids.get(&parent_id)) {
                (true, Some(&parent)) => placed.push((
                    depth,
                    id,
                    Link {
                        child,
                        parent,
                        parent_id,
                    },
                )),
                _ => left_out.push(id),
            }
        }
        placed.sort_by_key(|&(depth, id, _)| (depth, id));
        self.links = placed.into_iter().map(|(_, _, l)| l).collect();
        left_out
    }
}

fn place_with(world: &mut World, children: &mut Children, order: &mut Order) -> Vec<u64> {
    let count = children.iter(world).len();
    if count == 0 {
        order.links.clear();
        return Vec::new();
    }
    let left_out = if order.holds(world, count) {
        Vec::new()
    } else {
        order.rebuild(world, children)
    };
    for l in &order.links {
        let Some(c) = world.get::<ChildOf>(l.child).copied() else {
            continue;
        };
        let t = compose(&placement(world, l.parent), &c);
        match world.get_mut::<Transform>(l.child) {
            Some(mut now) => {
                if *now != t {
                    *now = t;
                }
            }
            None => {
                world.entity_mut(l.child).insert(t);
            }
        }
    }
    left_out
}

/// The system: writes every child's `Transform`. Costs nothing in a world without `ChildOf`.
pub fn place(world: &mut World, children: &mut Children, mut order: Local<Order>) {
    place_with(world, children, &mut order);
}

/// Places every child once, right after a scene is loaded, and refuses a scene whose
/// `ChildOf` parents form a cycle.
pub fn place_loaded(world: &mut World) -> Result<()> {
    let mut children = world.query::<(Entity, &SceneId, &ChildOf)>();
    let left_out = place_with(world, &mut children, &mut Order::default());
    match left_out.first() {
        None => Ok(()),
        Some(&id) => Err(Errors::from(
            Diagnostic::error(
                codes::SCENE_PARENT_CYCLE,
                format!(
                    "entity {id} is its own ancestor: the ChildOf parents of {} form a cycle",
                    left_out.iter().map(u64::to_string).collect::<Vec<_>>().join(", ")
                ),
            )
            .entity(id)
            .component("ChildOf")
            .fix("change ChildOf.parent on one of them, or remove its ChildOf".to_string()),
        )),
    }
}
