//! Navigation for round, changing worlds (decisions 0231, 0232): a replacement for a baked
//! navigation mesh that works on spheres, on terrain made at run time, and while the world
//! changes, for thousands of agents.
//!
//! - [`NavSurface`] on a world's entity (centre and turn = its `Transform`) builds a
//!   cube-sphere cell grid ([`grid::Grid`]). At load each cell probes the ground with the
//!   physics ray cast (when the scene has physics) for its height and slope; a game with its
//!   own terrain function calls [`bake_with`] instead, and [`rebake`] after it edits terrain.
//! - [`NavObstacle`] blocks the cells around it. Moving, adding or removing one updates only
//!   the cells it covers, and only agents whose route crosses a changed cell plan again.
//! - [`NavAgent`] walks to `goal`. Agents with the same goal cell share one search; route
//!   searches and steering run on the job pool; each agent avoids the others. `Direct` agents
//!   are moved along the ground by navigation (cheapest, for crowds); `Intent` agents write a
//!   `MoveIntent` for a character controller (`PlanetCharacter`) or a game system.
//!
//! The grid and routes are plain integers and `+ - * / sqrt` floats, searched in a fixed
//! order, so a run is the same on every platform and thread count. Routes live in the module
//! state (saved in snapshots); [`NavAgentState`] is in the scene and the state hash.

pub mod grid;
pub mod search;
pub mod steer;

use bevy_ecs::prelude::*;
use bevy_reflect::Reflect;
use grid::Grid;
use marionette_core::builtin::{DEFAULT_DT, FixedStep, MoveIntent, SceneRef};
use marionette_core::module::{RayCaster, RayFilter, Stage};
use marionette_core::scene::SceneId;
use marionette_core::spatial::{Transform, Vec3};
use marionette_core::vmath::*;
use marionette_core::{Diagnostic, Errors, Module, Registry, Result};
use marionette_jobs::Jobs;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Stable error codes.
pub mod codes {
    pub const SNAPSHOT: &str = "nav.snapshot";
    pub const NO_SURFACE: &str = "nav.no_surface";
}

/// A walkable round world. Put it on the world's entity; its `Transform` translation is the
/// centre and its rotation turns the grid with the world.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq)]
pub struct NavSurface {
    /// Ground radius in metres where there is no terrain to probe.
    pub radius: f32,
    /// Cells per cube-face edge (6 × resolution² cells; 128 gives cells of about
    /// radius / 80 metres). 4 to 1024.
    pub resolution: u32,
    /// Steepest walkable ground as rise over run (1 = 45°).
    pub max_grade: f32,
    /// Ground is probed from `radius + probe` down to `radius - probe` (0 = no probing:
    /// a smooth sphere at `radius`).
    pub probe: f32,
    /// Ground below this distance from the centre is water and not walkable (0 = none).
    pub sea_level: f32,
    /// Extra metres added around every obstacle (agent body clearance).
    pub clearance: f32,
}

/// Blocks the surface cells within `radius` (plus the surface's clearance) of this entity's
/// `Transform` translation, while the entity exists.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq)]
pub struct NavObstacle {
    pub surface: SceneRef,
    pub radius: f32,
}

#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavDrive {
    /// Navigation moves the `Transform` along the ground (no physics body needed).
    Direct,
    /// Navigation writes a `MoveIntent`; something else moves the entity.
    Intent,
}

/// An agent that walks to `goal` over a `NavSurface`.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq)]
pub struct NavAgent {
    pub surface: SceneRef,
    /// Destination in world space. Changing it plans a new route.
    pub goal: Vec3,
    /// False stands still (and drops the route).
    pub active: bool,
    /// Top speed in m/s.
    pub speed: f32,
    /// m/s².
    pub acceleration: f32,
    /// Body radius for avoidance, in metres.
    pub radius: f32,
    /// `Direct` only: height of the entity's origin above the ground.
    pub height: f32,
    /// Counts as arrived within this many metres of the goal.
    pub arrive_distance: f32,
    pub drive: NavDrive,
}

#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NavStatus {
    #[default]
    Idle,
    /// Waiting for a route search (searches per tick are capped).
    Waiting,
    Moving,
    /// The goal cannot be reached; moving to the closest reachable place.
    Partial,
    Arrived,
    /// No route and no closer place (for example, standing inside a blocked area).
    Unreachable,
}

/// Navigation's view of an agent. Written by navigation.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq, Default)]
pub struct NavAgentState {
    pub status: NavStatus,
    /// World-space velocity in m/s.
    pub velocity: Vec3,
    /// Route corners left before the goal.
    pub corners: u32,
}

/// Navigation tunables (optional resource; these defaults apply without it).
#[derive(Resource, Reflect, Debug, Clone, Copy, PartialEq)]
pub struct NavSettings {
    /// Route searches started per tick at most (agents beyond wait in order).
    pub searches_per_tick: u32,
    /// Cells one search may expand before it settles for the closest place.
    pub search_budget: u32,
    /// Agents closer than this (metres) are considered for avoidance.
    pub neighbour_distance: f32,
    /// Seconds ahead that avoidance looks.
    pub horizon: f32,
}

impl Default for NavSettings {
    fn default() -> Self {
        Self {
            searches_per_tick: 256,
            search_budget: 200_000,
            neighbour_distance: 3.0,
            horizon: 1.5,
        }
    }
}

pub struct NavModule;

/// One surface's grid and placement.
struct Surface {
    grid: Grid,
    spec: NavSurface,
    pos: [f32; 3],
    rot: [f32; 4],
    /// Cells changed this tick (any change makes agents check their routes).
    dirty: bool,
}

impl Surface {
    fn to_local(&self, p: [f32; 3]) -> [f32; 3] {
        qrot(qconj(self.rot), sub(p, self.pos))
    }
    fn to_world(&self, p: [f32; 3]) -> [f32; 3] {
        add(self.pos, qrot(self.rot, p))
    }
    /// Local point on the ground (plus `lift`) under local direction `d`.
    fn on_ground(&self, d: [f32; 3], lift: f32) -> [f32; 3] {
        let u = norm(d).unwrap_or([0.0, 1.0, 0.0]);
        scale(u, self.grid.ground(u) + lift)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
struct AgentRt {
    surface: u64,
    goal_bits: [u32; 3],
    /// Route corners (cells), the goal last.
    route: Vec<u32>,
    next: u32,
    goal_local: [f32; 3],
    stuck: u16,
    planned: bool,
    partial: bool,
    arrived: bool,
    /// The goal's own cell is open (otherwise the route ends at the nearest open cell).
    exact: bool,
    /// Blocked off the cell-centre line: walk back to this cell's centre first (from there
    /// the next corner is in sight).
    recentre: Option<u32>,
    /// In the search queue.
    queued: bool,
}

#[derive(Clone, Debug)]
struct ObstacleRt {
    surface: u64,
    key: [u32; 5],
    cells: Vec<u32>,
}

/// Saved per surface: the baked weights and heights (they may have been edited).
#[derive(Serialize, Deserialize)]
struct SavedSurface {
    base: Vec<u8>,
    height: Vec<f32>,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    agents: BTreeMap<u64, AgentRt>,
    queue: VecDeque<u64>,
    surfaces: BTreeMap<u64, SavedSurface>,
}

/// Navigation's runtime: grids, obstacle stamps, routes. Not a scene resource.
#[derive(Resource, Default)]
pub struct NavWorld {
    surfaces: BTreeMap<u64, Surface>,
    /// Agent ids in order, and their runtime at the same index.
    ids: Vec<u64>,
    rts: Vec<AgentRt>,
    obstacles: BTreeMap<u64, ObstacleRt>,
    /// Agents waiting for a search, oldest first.
    queue: VecDeque<u64>,
    /// After a restore, obstacles are stamped again without counting as a change.
    quiet_restamp: bool,
}

impl NavWorld {
    /// The grid of a surface entity, if built.
    pub fn grid(&self, surface: u64) -> Option<&Grid> {
        self.surfaces.get(&surface).map(|s| &s.grid)
    }
}

impl Module for NavModule {
    fn name(&self) -> &'static str {
        "nav"
    }

    fn register(&self, r: &mut Registry) {
        r.register_resource::<NavSettings>();
        r.register_component::<NavSurface>();
        r.register_component::<NavObstacle>();
        r.register_component::<NavAgent>();
        r.register_component::<NavAgentState>();
    }

    fn add_systems(&self, schedule: &mut Schedule) {
        schedule.add_systems(nav_tick.in_set(Stage::Logic));
    }

    fn init(&self, world: &mut World) -> Result<()> {
        world.remove_resource::<NavWorld>();
        let any = world
            .try_query::<&NavSurface>()
            .is_some_and(|mut q| q.iter(world).next().is_some());
        if !any {
            return Ok(());
        }
        if !world.contains_resource::<Jobs>() {
            world.insert_resource(Jobs::shared());
        }
        let mut nav = NavWorld::default();
        sync_surfaces(world, &mut nav);
        world.insert_resource(nav);
        Ok(())
    }

    fn save(&self, world: &World) -> Option<Vec<u8>> {
        let nav = world.get_resource::<NavWorld>()?;
        let saved = Saved {
            agents: nav.ids.iter().copied().zip(nav.rts.iter().cloned()).collect(),
            queue: nav.queue.clone(),
            surfaces: nav
                .surfaces
                .iter()
                .map(|(id, s)| {
                    (
                        *id,
                        SavedSurface {
                            base: s.grid.base.clone(),
                            height: s.grid.height.clone(),
                        },
                    )
                })
                .collect(),
        };
        Some(postcard::to_allocvec(&saved).expect("nav state serializes"))
    }

    fn load(&self, world: &mut World, bytes: &[u8]) -> Result<()> {
        let saved: Saved = postcard::from_bytes(bytes).map_err(|e| {
            Errors::from(Diagnostic::error(
                codes::SNAPSHOT,
                format!("cannot restore navigation state: {e}"),
            ))
        })?;
        let mut nav = world.remove_resource::<NavWorld>().unwrap_or_default();
        for (id, s) in saved.surfaces {
            if let Some(surf) = nav.surfaces.get_mut(&id)
                && surf.grid.base.len() == s.base.len()
            {
                surf.grid.base = s.base;
                surf.grid.height = s.height;
                surf.grid.blocks.fill(0);
            }
        }
        // A snapshot is taken between ticks, when nothing is marked changed.
        for surf in nav.surfaces.values_mut() {
            surf.dirty = false;
        }
        nav.obstacles.clear();
        nav.ids = saved.agents.keys().copied().collect();
        nav.rts = saved.agents.into_values().collect();
        nav.queue = saved.queue;
        nav.quiet_restamp = true;
        world.insert_resource(nav);
        Ok(())
    }
}

/// Builds grids for new surfaces (probing the ground), drops removed ones, and follows
/// moving ones.
fn sync_surfaces(world: &mut World, nav: &mut NavWorld) {
    let mut found: Vec<(u64, NavSurface, Transform)> = world
        .query::<(&SceneId, &NavSurface, Option<&Transform>)>()
        .iter(world)
        .map(|(id, s, t)| (id.0, *s, t.copied().unwrap_or_default()))
        .collect();
    found.sort_by_key(|f| f.0);
    let ids: BTreeSet<u64> = found.iter().map(|f| f.0).collect();
    nav.surfaces.retain(|id, _| ids.contains(id));
    for (id, spec, t) in found {
        let pos = [t.translation.x, t.translation.y, t.translation.z];
        let rot = qnorm([t.rotation.x, t.rotation.y, t.rotation.z, t.rotation.w]);
        if let Some(s) = nav.surfaces.get_mut(&id)
            && s.spec == spec
        {
            s.pos = pos;
            s.rot = rot;
            continue;
        }
        let mut s = Surface {
            grid: Grid::new(spec.resolution, spec.radius),
            spec,
            pos,
            rot,
            dirty: true,
        };
        if spec.probe > 0.0 {
            let all: Vec<u32> = (0..s.grid.cells() as u32).collect();
            probe_cells(world, &mut s, &all);
        } else if spec.sea_level > spec.radius {
            s.grid.base.fill(0);
        }
        // Obstacles of a rebuilt surface are stamped again.
        nav.obstacles.retain(|_, o| o.surface != id);
        nav.surfaces.insert(id, s);
    }
}

/// Probes the ground of `cells` with the physics ray cast (in parallel, results in order).
fn probe_cells(world: &World, s: &mut Surface, cells: &[u32]) {
    let caster = world.get_resource::<RayCaster>().map(|r| r.0);
    let jobs = world.get_resource::<Jobs>().cloned().unwrap_or_default();
    let spec = s.spec;
    let grid = &s.grid;
    let (pos, rot) = (s.pos, s.rot);
    let results: Vec<Vec<(f32, u8)>> = jobs.map_chunks_heavy(cells.len(), 512, |range| {
        cells[range]
            .iter()
            .map(|&c| {
                let d = grid.centre[c as usize];
                let world_up = qrot(rot, d);
                let origin = add(pos, scale(world_up, spec.radius + spec.probe));
                let hit = caster.and_then(|cast| {
                    cast(
                        world,
                        origin,
                        scale(world_up, -1.0),
                        2.0 * spec.probe,
                        RayFilter {
                            exclude: None,
                            skip_sensors: true,
                        },
                    )
                });
                match hit {
                    None => (spec.radius, walk_weight(spec, spec.radius, 1.0)),
                    Some(h) if h.distance <= 0.0 => (spec.radius + spec.probe, 0),
                    Some(h) => {
                        let height = len(sub(h.point, pos));
                        let cos = dot(h.normal, world_up);
                        (height, walk_weight(spec, height, cos))
                    }
                }
            })
            .collect()
    });
    for (&c, (h, w)) in cells.iter().zip(results.into_iter().flatten()) {
        s.grid.height[c as usize] = h;
        s.grid.base[c as usize] = w;
    }
    s.dirty = true;
}

/// Weight of ground at `height` whose normal makes cosine `cos` with up (no trigonometry:
/// grade² = (1 - cos²) / cos²).
fn walk_weight(spec: NavSurface, height: f32, cos: f32) -> u8 {
    if spec.sea_level > 0.0 && height < spec.sea_level {
        return 0;
    }
    if cos <= 0.0 {
        return 0;
    }
    let g2 = spec.max_grade * spec.max_grade;
    if 1.0 - cos * cos > g2 * cos * cos { 0 } else { 1 }
}

/// Probes the ground again around a world point (after terrain changed): cells within
/// `radius` metres. Agents whose routes cross those cells plan again.
pub fn rebake(world: &mut World, surface: u64, centre: [f32; 3], radius: f32) -> Result<()> {
    let Some(mut nav) = world.remove_resource::<NavWorld>() else {
        return Err(no_surface(surface));
    };
    let out = match nav.surfaces.get_mut(&surface) {
        Some(s) => {
            let cells = s.grid.cells_within(s.to_local(centre), radius);
            probe_cells(world, s, &cells);
            Ok(())
        }
        None => Err(no_surface(surface)),
    };
    world.insert_resource(nav);
    out
}

/// Sets every cell from a game's own terrain function, given the cell's local unit
/// direction: `(ground distance from centre, weight)` with weight 0 = blocked, 1 = normal,
/// up to 255 = dearer (paths avoid it). For terrain that is a function, not colliders.
pub fn bake_with(world: &mut World, surface: u64, f: impl Fn([f32; 3]) -> (f32, u8) + Sync) -> Result<()> {
    let Some(mut nav) = world.remove_resource::<NavWorld>() else {
        return Err(no_surface(surface));
    };
    let jobs = world.get_resource::<Jobs>().cloned().unwrap_or_default();
    let out = match nav.surfaces.get_mut(&surface) {
        Some(s) => {
            let grid = &s.grid;
            let vals: Vec<Vec<(f32, u8)>> =
                jobs.map_chunks_heavy(grid.cells(), 4096, |r| r.map(|c| f(grid.centre[c])).collect());
            for (c, (h, w)) in vals.into_iter().flatten().enumerate() {
                s.grid.height[c] = h;
                s.grid.base[c] = w;
            }
            s.dirty = true;
            Ok(())
        }
        None => Err(no_surface(surface)),
    };
    world.insert_resource(nav);
    out
}

/// A route between two world points on a surface (corner points in world space, on the
/// ground), and whether it reaches `to`. For tools and scripts; agents plan by themselves.
pub fn find_path(world: &World, surface: u64, from: [f32; 3], to: [f32; 3]) -> Option<(Vec<[f32; 3]>, bool)> {
    let nav = world.get_resource::<NavWorld>()?;
    let s = nav.surfaces.get(&surface)?;
    let settings = world.get_resource::<NavSettings>().copied().unwrap_or_default();
    let a = s.grid.cell_of(s.to_local(from));
    let goal_local = s.to_local(to);
    let b = search::nearest_open(&s.grid, s.grid.cell_of(goal_local), 256)?;
    let r = search::astar(&s.grid, a, b, settings.search_budget);
    let cells = search::smooth(&s.grid, &r.cells);
    let mut pts: Vec<[f32; 3]> = cells
        .iter()
        .map(|c| s.to_world(s.on_ground(s.grid.centre[*c as usize], 0.0)))
        .collect();
    if r.complete
        && let Some(last) = pts.last_mut()
    {
        *last = s.to_world(s.on_ground(goal_local, 0.0));
    }
    Some((pts, r.complete))
}

fn no_surface(id: u64) -> Errors {
    Diagnostic::error(codes::NO_SURFACE, format!("entity {id} has no built NavSurface"))
        .entity(id)
        .fix("add a NavSurface component to the world's entity")
        .into()
}

/// Updates obstacle stamps; returns nothing, marks surfaces dirty.
fn sync_obstacles(world: &mut World, nav: &mut NavWorld) {
    let mut found: Vec<(u64, NavObstacle, [f32; 3])> = world
        .query::<(&SceneId, &NavObstacle, Option<&Transform>)>()
        .iter(world)
        .map(|(id, o, t)| {
            let t = t.copied().unwrap_or_default().translation;
            (id.0, *o, [t.x, t.y, t.z])
        })
        .collect();
    found.sort_by_key(|f| f.0);
    let quiet = std::mem::take(&mut nav.quiet_restamp);
    let ids: BTreeSet<u64> = found.iter().map(|f| f.0).collect();
    let gone: Vec<u64> = nav
        .obstacles
        .keys()
        .filter(|k| !ids.contains(k))
        .copied()
        .collect();
    for id in gone {
        let o = nav.obstacles.remove(&id).expect("listed");
        unstamp(nav, &o);
    }
    for (id, ob, p) in found {
        let key = [
            p[0].to_bits(),
            p[1].to_bits(),
            p[2].to_bits(),
            ob.radius.to_bits(),
            0,
        ];
        if let Some(old) = nav.obstacles.get(&id) {
            if old.key == key && old.surface == ob.surface.0 {
                continue;
            }
            let old = nav.obstacles.remove(&id).expect("present");
            unstamp(nav, &old);
        }
        let Some(s) = nav.surfaces.get_mut(&ob.surface.0) else {
            continue;
        };
        let cells = s.grid.cells_within(s.to_local(p), ob.radius + s.spec.clearance);
        for &c in &cells {
            s.grid.blocks[c as usize] = s.grid.blocks[c as usize].saturating_add(1);
        }
        if !quiet {
            s.dirty = true;
        }
        nav.obstacles.insert(
            id,
            ObstacleRt {
                surface: ob.surface.0,
                key,
                cells,
            },
        );
    }
}

fn unstamp(nav: &mut NavWorld, o: &ObstacleRt) {
    if let Some(s) = nav.surfaces.get_mut(&o.surface) {
        for &c in &o.cells {
            s.grid.blocks[c as usize] = s.grid.blocks[c as usize].saturating_sub(1);
        }
        s.dirty = true;
    }
}

/// An agent gathered for this tick.
#[derive(Clone, Copy)]
struct AgentIn {
    id: u64,
    entity: Entity,
    agent: NavAgent,
    pos: [f32; 3],
    vel: [f32; 3],
}

/// A search job: one goal cell on one surface, the agents (indices) heading there.
struct Job {
    surface: u64,
    goal: u32,
    starts: Vec<u32>,
    agents: Vec<usize>,
}

/// Per-agent result of the target phase.
#[derive(Clone, Copy)]
struct Aim {
    status: NavStatus,
    target: Option<[f32; 3]>,
    last: bool,
    next: u32,
    recentre: Option<u32>,
}

/// Per-agent result of the move phase.
#[derive(Clone, Copy)]
struct Moved {
    /// World-space velocity.
    vel: [f32; 3],
    /// `Direct` agents: new world position and rotation.
    place: Option<([f32; 3], Option<[f32; 4]>)>,
    /// Blocked: walk back to this cell's centre.
    blocked: Option<u32>,
}

/// Lines `nav.rts` up with this tick's sorted agent ids, keeping known agents' runtime.
fn align(nav: &mut NavWorld, ids: &[u64]) {
    if nav.ids == ids {
        return;
    }
    let mut old: BTreeMap<u64, AgentRt> = std::mem::take(&mut nav.ids)
        .into_iter()
        .zip(std::mem::take(&mut nav.rts))
        .collect();
    nav.rts = ids.iter().map(|id| old.remove(id).unwrap_or_default()).collect();
    nav.ids = ids.to_vec();
    let live: BTreeSet<u64> = ids.iter().copied().collect();
    nav.queue.retain(|id| live.contains(id));
}

/// The navigation step (Stage::Logic, before physics).
fn nav_tick(world: &mut World) {
    let Some(mut nav) = world.remove_resource::<NavWorld>() else {
        return;
    };
    let settings = world.get_resource::<NavSettings>().copied().unwrap_or_default();
    let dt = world.get_resource::<FixedStep>().map_or(DEFAULT_DT, |s| s.dt);
    let jobs = world.get_resource::<Jobs>().cloned().unwrap_or_default();
    sync_surfaces(world, &mut nav);
    sync_obstacles(world, &mut nav);

    // Gather agents in id order.
    let mut agents: Vec<AgentIn> = world
        .query::<(
            Entity,
            &SceneId,
            &NavAgent,
            Option<&Transform>,
            Option<&NavAgentState>,
        )>()
        .iter(world)
        .map(|(e, id, a, t, st)| {
            let t = t.copied().unwrap_or_default().translation;
            let v = st.copied().unwrap_or_default().velocity;
            AgentIn {
                id: id.0,
                entity: e,
                agent: *a,
                pos: [t.x, t.y, t.z],
                vel: [v.x, v.y, v.z],
            }
        })
        .collect();
    agents.sort_unstable_by_key(|a| a.id);
    let ids: Vec<u64> = agents.iter().map(|a| a.id).collect();
    align(&mut nav, &ids);
    let index: BTreeMap<u64, usize> = if nav.queue.is_empty() {
        BTreeMap::new()
    } else {
        ids.iter().enumerate().map(|(i, id)| (*id, i)).collect()
    };

    // Who needs a route.
    {
        let NavWorld {
            surfaces, rts, queue, ..
        } = &mut nav;
        for (i, a) in agents.iter().enumerate() {
            let rt = &mut rts[i];
            let Some(s) = surfaces.get(&a.agent.surface.0) else {
                continue;
            };
            if !a.agent.active {
                if rt.planned {
                    let queued = rt.queued;
                    *rt = AgentRt {
                        queued,
                        ..AgentRt::default()
                    };
                }
                continue;
            }
            let gb = [
                a.agent.goal.x.to_bits(),
                a.agent.goal.y.to_bits(),
                a.agent.goal.z.to_bits(),
            ];
            let fresh = !rt.planned || rt.goal_bits != gb || rt.surface != a.agent.surface.0;
            let mut replan = fresh || rt.stuck >= STUCK_TICKS;
            if !replan && s.dirty && !rt.arrived && !rt.queued {
                // The world changed: give unreachable goals another try, and check that the
                // rest of the route is still open.
                if rt.partial {
                    replan = true;
                } else {
                    let mut prev = s.grid.cell_of(s.to_local(a.pos));
                    for &c in rt.route.iter().skip(rt.next as usize) {
                        if !s.grid.line_open(prev, c) {
                            replan = true;
                            break;
                        }
                        prev = c;
                    }
                }
            }
            if replan {
                rt.goal_bits = gb;
                rt.surface = a.agent.surface.0;
                rt.goal_local = s.to_local([a.agent.goal.x, a.agent.goal.y, a.agent.goal.z]);
                rt.planned = true;
                rt.arrived = false;
                rt.partial = false;
                rt.recentre = None;
                rt.stuck = 0;
                rt.route.clear();
                rt.next = 0;
                if !rt.queued {
                    rt.queued = true;
                    queue.push_back(a.id);
                }
            }
        }
    }

    // Take up to the cap from the queue, group by (surface, goal cell).
    let index = if index.is_empty() && !nav.queue.is_empty() {
        ids.iter().enumerate().map(|(i, id)| (*id, i)).collect()
    } else {
        index
    };
    let take = (settings.searches_per_tick as usize).min(nav.queue.len());
    let mut groups: BTreeMap<(u64, u32), Job> = BTreeMap::new();
    let taken: Vec<u64> = nav.queue.drain(..take).collect();
    for id in taken {
        let i = index[&id];
        let a = &agents[i];
        let rt = &mut nav.rts[i];
        rt.queued = false;
        let Some(s) = nav.surfaces.get(&rt.surface) else {
            continue;
        };
        let start = s.grid.cell_of(s.to_local(a.pos));
        let goal_cell = s.grid.cell_of(rt.goal_local);
        let Some(goal) = search::nearest_open(&s.grid, goal_cell, 256) else {
            rt.partial = true;
            continue;
        };
        rt.exact = goal == goal_cell;
        let job = groups.entry((rt.surface, goal)).or_insert_with(|| Job {
            surface: rt.surface,
            goal,
            starts: Vec::new(),
            agents: Vec::new(),
        });
        job.starts.push(start);
        job.agents.push(i);
    }
    let job_list: Vec<Job> = groups.into_values().collect();
    let budget = settings.search_budget;
    let results: Vec<Vec<Vec<(usize, search::Route)>>> = {
        let surfaces = &nav.surfaces;
        jobs.map_chunks_heavy(job_list.len(), 1, |range| {
            range
                .map(|j| {
                    let job = &job_list[j];
                    let g = &surfaces[&job.surface].grid;
                    let shared = if job.starts.len() > 1 {
                        search::shared(g, job.goal, &job.starts, budget)
                    } else {
                        vec![None]
                    };
                    job.agents
                        .iter()
                        .zip(&job.starts)
                        .zip(shared)
                        .map(|((&i, &st), r)| {
                            let r = r.unwrap_or_else(|| search::astar(g, st, job.goal, budget));
                            let cells = search::smooth(g, &r.cells);
                            (
                                i,
                                search::Route {
                                    cells,
                                    complete: r.complete,
                                },
                            )
                        })
                        .collect()
                })
                .collect()
        })
    };
    for (i, route) in results.into_iter().flatten().flatten() {
        let rt = &mut nav.rts[i];
        rt.partial = !route.complete;
        rt.route = route.cells;
        // Skip the start cell: steer to the next corner.
        rt.next = if rt.route.len() > 1 { 1 } else { 0 };
    }

    // Aim: status and the point each agent heads for (parallel, surface-local).
    let aims: Vec<Aim> = {
        let (surfaces, rts) = (&nav.surfaces, &nav.rts);
        let agents = &agents;
        jobs.map_chunks_heavy(agents.len(), 512, |r| {
            r.map(|i| aim(&agents[i], &rts[i], surfaces.get(&agents[i].agent.surface.0)))
                .collect::<Vec<_>>()
        })
        .into_iter()
        .flatten()
        .collect()
    };
    for (rt, a) in nav.rts.iter_mut().zip(&aims) {
        rt.next = a.next;
        rt.recentre = a.recentre;
        if matches!(a.status, NavStatus::Arrived | NavStatus::Unreachable) {
            if !rt.route.is_empty() {
                rt.route.clear();
            }
            rt.arrived = a.status == NavStatus::Arrived;
        }
    }

    // Steer and move (parallel; each agent reads everyone, writes only its own result).
    let bodies: Vec<steer::Body> = agents
        .iter()
        .map(|a| match nav.surfaces.get(&a.agent.surface.0) {
            Some(s) => steer::Body {
                pos: s.to_local(a.pos),
                vel: qrot(qconj(s.rot), a.vel),
                radius: a.agent.radius,
                surface: a.agent.surface.0,
            },
            None => steer::Body {
                pos: a.pos,
                vel: [0.0; 3],
                radius: a.agent.radius,
                surface: u64::MAX,
            },
        })
        .collect();
    let wants: Vec<steer::Want> = agents
        .iter()
        .zip(&aims)
        .map(|(a, aim)| steer::Want {
            target: aim.target,
            last: aim.last,
            speed: a.agent.speed,
            accel: a.agent.acceleration,
        })
        .collect();
    let params = steer::Params {
        neighbour_distance: settings.neighbour_distance,
        horizon: settings.horizon,
        max_neighbours: 16,
        dt,
    };
    let buckets = steer::Buckets::new(&bodies, settings.neighbour_distance);
    let moved: Vec<Moved> = {
        let surfaces = &nav.surfaces;
        let (agents, bodies, wants) = (&agents, &bodies, &wants);
        jobs.map_chunks_heavy(agents.len(), 256, |r| {
            r.map(|i| {
                let a = &agents[i];
                match surfaces.get(&a.agent.surface.0) {
                    Some(s) if a.agent.active && wants[i].target.is_some() => {
                        let v = steer::velocity(i, bodies, wants, &buckets, &params);
                        move_agent(s, a, bodies[i].pos, v, dt)
                    }
                    Some(s) => move_agent(s, a, bodies[i].pos, [0.0; 3], dt),
                    None => Moved {
                        vel: [0.0; 3],
                        place: None,
                        blocked: None,
                    },
                }
            })
            .collect::<Vec<_>>()
        })
        .into_iter()
        .flatten()
        .collect()
    };

    // Write back (serial, id order).
    for (i, a) in agents.iter().enumerate() {
        if !nav.surfaces.contains_key(&a.agent.surface.0) {
            continue;
        }
        let m = moved[i];
        let rt = &mut nav.rts[i];
        if let Some(c) = m.blocked {
            rt.recentre = Some(c);
        }
        let speed = len(m.vel);
        if wants[i].target.is_some() && speed < 0.05 * a.agent.speed {
            rt.stuck = rt.stuck.saturating_add(1);
        } else {
            rt.stuck = 0;
        }
        match (a.agent.drive, m.place) {
            (NavDrive::Direct, Some((p, rot))) => {
                if let Some(mut t) = world.get_mut::<Transform>(a.entity) {
                    t.translation = Vec3::new(p[0], p[1], p[2]);
                    if let Some(q) = rot {
                        t.rotation.x = q[0];
                        t.rotation.y = q[1];
                        t.rotation.z = q[2];
                        t.rotation.w = q[3];
                    }
                }
            }
            _ => {
                let velocity = Vec3::new(m.vel[0], m.vel[1], m.vel[2]);
                match world.get_mut::<MoveIntent>(a.entity) {
                    Some(mut mi) => {
                        if mi.velocity != velocity {
                            mi.velocity = velocity;
                        }
                    }
                    None => {
                        world.entity_mut(a.entity).insert(MoveIntent {
                            velocity,
                            jump: false,
                        });
                    }
                }
            }
        }
        let state = NavAgentState {
            status: aims[i].status,
            velocity: Vec3::new(m.vel[0], m.vel[1], m.vel[2]),
            corners: rt.route.len().saturating_sub(rt.next as usize) as u32,
        };
        match world.get_mut::<NavAgentState>(a.entity) {
            Some(mut st) => {
                if *st != state {
                    *st = state;
                }
            }
            None => {
                world.entity_mut(a.entity).insert(state);
            }
        }
    }
    for s in nav.surfaces.values_mut() {
        s.dirty = false;
    }
    world.insert_resource(nav);
}

/// Status and target point of one agent.
fn aim(a: &AgentIn, rt: &AgentRt, s: Option<&Surface>) -> Aim {
    let mut out = Aim {
        status: NavStatus::Idle,
        target: None,
        last: false,
        next: rt.next,
        recentre: rt.recentre,
    };
    let Some(s) = s else {
        return out;
    };
    out.status = if !a.agent.active {
        NavStatus::Idle
    } else if rt.queued {
        NavStatus::Waiting
    } else if rt.arrived {
        NavStatus::Arrived
    } else if rt.route.is_empty() {
        if rt.partial {
            NavStatus::Unreachable
        } else {
            NavStatus::Idle
        }
    } else if rt.partial {
        NavStatus::Partial
    } else {
        NavStatus::Moving
    };
    if !matches!(out.status, NavStatus::Moving | NavStatus::Partial) {
        return out;
    }
    let pos = s.to_local(a.pos);
    let up = norm(pos).unwrap_or([0.0, 1.0, 0.0]);
    let flat = |point: [f32; 3]| {
        let gap = sub(point, pos);
        len(sub(gap, scale(up, dot(gap, up))))
    };
    let reach = a.agent.radius.max(s.grid.min_step * s.spec.radius * 0.5);
    if let Some(c) = out.recentre {
        let point = s.on_ground(s.grid.centre[c as usize], a.agent.height);
        if flat(point) <= reach * 0.3 {
            out.recentre = None;
        } else {
            out.target = Some(point);
            return out;
        }
    }
    loop {
        let n = out.next as usize;
        let is_last = n + 1 >= rt.route.len();
        let point = if is_last && !rt.partial && rt.exact {
            rt.goal_local
        } else {
            s.grid.centre[rt.route[n.min(rt.route.len() - 1)] as usize]
        };
        let point = s.on_ground(point, a.agent.height);
        let d = flat(point);
        if is_last {
            if d <= a.agent.arrive_distance.max(1e-3) {
                out.status = if rt.partial {
                    NavStatus::Unreachable
                } else {
                    NavStatus::Arrived
                };
            } else {
                out.target = Some(point);
                out.last = true;
            }
            return out;
        }
        if d <= reach {
            out.next += 1;
            continue;
        }
        out.target = Some(point);
        return out;
    }
}

/// Moves one agent by velocity `v` (surface-local) for `dt`: `Direct` agents slide along
/// blocked cells and stay on the ground; `Intent` agents only report the velocity.
fn move_agent(s: &Surface, a: &AgentIn, pos: [f32; 3], mut v: [f32; 3], dt: f32) -> Moved {
    if a.agent.drive != NavDrive::Direct {
        return Moved {
            vel: qrot(s.rot, v),
            place: None,
            blocked: None,
        };
    }
    let here = s.grid.cell_of(pos);
    let blocked = |p: [f32; 3]| {
        let c = s.grid.cell_of(p);
        (c != here && s.grid.weight(c) == 0).then_some(c)
    };
    let mut np = add(pos, scale(v, dt));
    let mut stopped = None;
    if let Some(c) = blocked(np) {
        // Slide along the blocked cell: drop the part of the velocity into it.
        let up = norm(pos).unwrap_or([0.0, 1.0, 0.0]);
        let into = sub(scale(s.grid.centre[c as usize], len(pos)), pos);
        let into = sub(into, scale(up, dot(into, up)));
        if let Some(n) = norm(into) {
            let k = dot(v, n);
            if k > 0.0 {
                v = sub(v, scale(n, k));
            }
        }
        np = add(pos, scale(v, dt));
        if blocked(np).is_some() {
            np = pos;
            v = [0.0; 3];
            stopped = Some(here);
        }
    }
    let np = s.on_ground(np, a.agent.height);
    let up = norm(np).unwrap_or([0.0, 1.0, 0.0]);
    let rot = if len(v) > 1e-3 {
        basis(up, v).map(|q| qmul(s.rot, q))
    } else {
        None
    };
    Moved {
        vel: qrot(s.rot, v),
        place: Some((s.to_world(np), rot)),
        blocked: stopped,
    }
}

/// Ticks of no progress before an agent plans again.
const STUCK_TICKS: u16 = 45;
