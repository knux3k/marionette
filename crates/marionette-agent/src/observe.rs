//! Observation: scene tree, entity queries, single entities, screenshots, logs, statistics.
//! Works on a live session (`session`) or on a scene file simulated for `ticks` with `seed`
//! (`scene`), which is how the stateless CLI observes. Never changes simulation state.

use crate::session::{LogEntry, Session};
use crate::{Agent, Op, codes, op, page, sha256_hex};
use marionette_core::scene::Scene;
use marionette_core::{Diagnostic, Errors, Sim};
use marionette_render::{Offscreen, assets::encode_png, extract};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::Path;
use std::time::Instant;

pub(crate) fn ops() -> Vec<Op> {
    vec![
        op(
            "observe_tree",
            "List entities (id, name, component names), paged.",
            false,
            tree,
        ),
        op(
            "observe_query",
            "Find entities that have all `with` components and match every `where` predicate \
             (path like `Position.x`, op eq|ne|lt|le|gt|ge|exists). `select` picks component values to return.",
            false,
            query,
        ),
        op("observe_entity", "All components of one entity.", false, entity),
        op(
            "observe_screenshot",
            "Render the world from a camera with no window and save a PNG; returns its path (not the pixels).",
            false,
            screenshot,
        ),
        op(
            "observe_logs",
            "Session log entries since `cursor`; returns `next_cursor`.",
            false,
            logs,
        ),
        op(
            "observe_stats",
            "Tick, entity count and step / render timings of a session.",
            false,
            stats,
        ),
        op(
            "observe_raycast",
            "Cast a ray through the physics scene; returns the first entity hit, distance, point and normal.",
            false,
            raycast,
        ),
        op(
            "observe_events",
            "Engine events so far (sound, jump, collision_started, collision_stopped), filtered and paged. \
             Sounds carry gain and pan: this is the headless audio log.",
            false,
            events,
        ),
    ]
}

/// Which world to look at: a live `session`, or `scene` loaded and run `ticks` with `seed`.
#[derive(Deserialize, JsonSchema, Clone, Default)]
pub struct WorldTarget {
    pub session: Option<String>,
    pub scene: Option<String>,
    #[serde(default)]
    pub seed: u64,
    #[serde(default)]
    pub ticks: u64,
}

/// Runs `f` on the target world. Stateless targets are loaded, stepped and discarded.
pub(crate) fn with_world<T>(
    agent: &mut Agent,
    target: &WorldTarget,
    f: impl FnOnce(&mut Agent, &mut Sim, &str) -> Result<T, Errors>,
) -> Result<T, Errors> {
    match (&target.session, &target.scene) {
        (Some(id), None) => {
            let mut session = take_session(agent, id)?;
            let scene = session.scene.clone();
            let result = f(agent, &mut session.sim, &scene);
            agent.sessions.insert(id.clone(), session);
            result
        }
        (None, Some(scene)) => {
            let mut sim = crate::project::load_sim(agent, scene, target.seed)?;
            sim.run(target.ticks);
            f(agent, &mut sim, scene)
        }
        _ => Err(Diagnostic::error(
            codes::TARGET,
            "observe needs exactly one of `session` and `scene`",
        )
        .fix("pass `session`, or `scene` with optional `seed` and `ticks`")
        .into()),
    }
}

fn take_session(agent: &mut Agent, id: &str) -> Result<Session, Errors> {
    crate::session::get_mut(agent, id)?;
    Ok(agent.sessions.remove(id).expect("checked"))
}

#[derive(Deserialize, JsonSchema)]
pub struct TreeInput {
    #[serde(flatten)]
    pub target: WorldTarget,
    #[serde(default)]
    pub offset: usize,
    /// Page size (default 50, max 500).
    pub limit: Option<usize>,
}

#[derive(Serialize, JsonSchema, Clone)]
pub struct TreeEntry {
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub components: Vec<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct TreeOutput {
    pub tick: u64,
    pub total: usize,
    pub entities: Vec<TreeEntry>,
    /// Offset of the next page, if there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_offset: Option<usize>,
}

fn tree(agent: &mut Agent, input: TreeInput) -> Result<TreeOutput, Errors> {
    with_world(agent, &input.target, |_, sim, _| {
        let scene = sim.snapshot();
        let all: Vec<TreeEntry> = scene
            .entities
            .iter()
            .map(|e| TreeEntry {
                id: e.id,
                name: e.name.clone(),
                components: e.components.keys().cloned().collect(),
            })
            .collect();
        let (entities, next_offset) = page(&all, input.offset, input.limit);
        Ok(TreeOutput {
            tick: sim.tick(),
            total: all.len(),
            entities,
            next_offset,
        })
    })
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum CompareOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Exists,
}

/// A test on one value inside a component, e.g. `Position.x > 0`.
#[derive(Deserialize, JsonSchema, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Predicate {
    /// Component name, then fields, separated by dots: `Position.x`,
    /// `Camera.projection.Perspective.fov_y_degrees`.
    pub path: String,
    pub op: CompareOp,
    /// Value to compare with (not needed for `exists`).
    #[serde(default)]
    pub value: Value,
}

/// Looks a dotted path up in an entity's components.
pub fn lookup<'a>(
    components: &'a std::collections::BTreeMap<String, Value>,
    path: &str,
) -> Option<&'a Value> {
    let mut parts = path.split('.');
    let mut v = components.get(parts.next()?)?;
    for p in parts {
        v = match v {
            Value::Object(m) => m.get(p)?,
            Value::Array(a) => a.get(p.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(v)
}

/// Evaluates a predicate. Numbers compare as f64; other values only with eq/ne.
pub fn holds(actual: Option<&Value>, op: CompareOp, expected: &Value) -> bool {
    let Some(a) = actual else { return false };
    if op == CompareOp::Exists {
        return true;
    }
    match (a.as_f64(), expected.as_f64()) {
        (Some(x), Some(y)) => match op {
            CompareOp::Eq => x == y,
            CompareOp::Ne => x != y,
            CompareOp::Lt => x < y,
            CompareOp::Le => x <= y,
            CompareOp::Gt => x > y,
            CompareOp::Ge => x >= y,
            CompareOp::Exists => true,
        },
        _ => match op {
            CompareOp::Eq => a == expected,
            CompareOp::Ne => a != expected,
            _ => false,
        },
    }
}

/// Ids (sorted) of entities with all `with` components that satisfy every predicate.
pub fn matching(scene: &Scene, with: &[String], predicates: &[Predicate]) -> Vec<u64> {
    scene
        .entities
        .iter()
        .filter(|e| with.iter().all(|c| e.components.contains_key(c)))
        .filter(|e| {
            predicates
                .iter()
                .all(|p| holds(lookup(&e.components, &p.path), p.op, &p.value))
        })
        .map(|e| e.id)
        .collect()
}

#[derive(Deserialize, JsonSchema)]
pub struct QueryInput {
    #[serde(flatten)]
    pub target: WorldTarget,
    /// Component names every result must have.
    #[serde(default)]
    pub with: Vec<String>,
    #[serde(default, rename = "where")]
    pub predicates: Vec<Predicate>,
    /// Component names whose values to include in each result (default: none).
    #[serde(default)]
    pub select: Vec<String>,
    #[serde(default)]
    pub offset: usize,
    pub limit: Option<usize>,
}

#[derive(Serialize, JsonSchema, Clone)]
pub struct QueryItem {
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Map::is_empty")]
    pub values: Map<String, Value>,
}

#[derive(Serialize, JsonSchema)]
pub struct QueryOutput {
    pub tick: u64,
    pub total: usize,
    pub items: Vec<QueryItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_offset: Option<usize>,
}

fn query(agent: &mut Agent, input: QueryInput) -> Result<QueryOutput, Errors> {
    for name in input.with.iter().chain(&input.select) {
        if agent.registry().get(name).is_none() {
            return Err(
                Diagnostic::error(codes::NOT_FOUND, format!("unknown component `{name}`"))
                    .component(name.clone())
                    .fix("list component names with project_schema")
                    .into(),
            );
        }
    }
    with_world(agent, &input.target, |_, sim, _| {
        let scene = sim.snapshot();
        let ids = matching(&scene, &input.with, &input.predicates);
        let all: Vec<QueryItem> = ids
            .iter()
            .map(|id| {
                let e = scene.entities.iter().find(|e| e.id == *id).expect("matched");
                let values = input
                    .select
                    .iter()
                    .filter_map(|c| e.components.get(c).map(|v| (c.clone(), v.clone())))
                    .collect();
                QueryItem {
                    id: e.id,
                    name: e.name.clone(),
                    values,
                }
            })
            .collect();
        let (items, next_offset) = page(&all, input.offset, input.limit);
        Ok(QueryOutput {
            tick: sim.tick(),
            total: all.len(),
            items,
            next_offset,
        })
    })
}

#[derive(Deserialize, JsonSchema)]
pub struct EntityInput {
    #[serde(flatten)]
    pub target: WorldTarget,
    pub id: u64,
}

#[derive(Serialize, JsonSchema)]
pub struct EntityOutput {
    pub tick: u64,
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub components: Map<String, Value>,
}

fn entity(agent: &mut Agent, input: EntityInput) -> Result<EntityOutput, Errors> {
    with_world(agent, &input.target, |_, sim, _| {
        let scene = sim.snapshot();
        let e = scene.entities.iter().find(|e| e.id == input.id).ok_or_else(|| {
            Errors::from(
                Diagnostic::error(codes::NO_ENTITY, format!("no entity {}", input.id))
                    .entity(input.id)
                    .fix("list entities with observe_tree"),
            )
        })?;
        Ok(EntityOutput {
            tick: sim.tick(),
            id: e.id,
            name: e.name.clone(),
            components: e.components.clone().into_iter().collect(),
        })
    })
}

#[derive(Deserialize, JsonSchema)]
pub struct ScreenshotInput {
    #[serde(flatten)]
    pub target: WorldTarget,
    /// Camera entity id (default: the lowest-id camera).
    pub camera: Option<u64>,
    /// Image size `WIDTHxHEIGHT` (default 640x360).
    pub size: Option<String>,
    /// MSAA samples, 1 or 4 (default 4).
    pub msaa: Option<u32>,
    /// Output PNG path (default `.marionette/screenshots/<name>-t<tick>.png`).
    pub out: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct ScreenshotOutput {
    /// PNG path as given (relative to the project root unless absolute).
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub tick: u64,
    pub sha256: String,
    /// State hash of the world that was rendered (rendering never changes it).
    pub state_hash: String,
    pub gpu: String,
    pub render_ms: f64,
}

pub(crate) fn parse_size(size: &str) -> Result<(u32, u32), Errors> {
    size.split_once(['x', 'X'])
        .and_then(|(w, h)| Some((w.trim().parse::<u32>().ok()?, h.trim().parse::<u32>().ok()?)))
        .filter(|&(w, h)| (1..=8192).contains(&w) && (1..=8192).contains(&h))
        .ok_or_else(|| {
            Diagnostic::error("cli.bad_size", format!("invalid size `{size}`"))
                .fix("use WIDTHxHEIGHT with each side 1..8192, e.g. 1280x720")
                .into()
        })
}

/// Renders `sim` and returns RGBA pixels and the render time.
pub(crate) fn render_pixels(
    agent: &mut Agent,
    sim: &Sim,
    scene: &str,
    camera: Option<u64>,
    (w, h): (u32, u32),
    msaa: u32,
) -> Result<(Vec<u8>, f64, String), Errors> {
    if !matches!(msaa, 1 | 4) {
        return Err(
            Diagnostic::error("cli.bad_msaa", format!("unsupported MSAA sample count {msaa}"))
                .fix("use 1 or 4")
                .into(),
        );
    }
    let base = agent
        .resolve(scene)?
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let gpu = agent.gpu()?;
    let start = Instant::now();
    let mut off = Offscreen::new(gpu, w, h, msaa, base);
    off.render(gpu, &extract(&sim.world), camera)
        .map_err(|e| e.with_file(scene))?;
    let pixels = off.read_pixels(gpu)?;
    Ok((pixels, start.elapsed().as_secs_f64() * 1000.0, gpu.describe()))
}

fn screenshot(agent: &mut Agent, input: ScreenshotInput) -> Result<ScreenshotOutput, Errors> {
    let size = parse_size(input.size.as_deref().unwrap_or("640x360"))?;
    let msaa = input.msaa.unwrap_or(4);
    let label = match (&input.target.session, &input.target.scene) {
        (Some(s), _) => s.clone(),
        (None, Some(scene)) => Path::new(scene)
            .file_name()
            .map_or("scene".into(), |n| n.to_string_lossy().replace(".scene.json", "")),
        _ => String::new(),
    };
    let read_only = agent.read_only();
    let out = with_world(agent, &input.target, |agent, sim, scene| {
        let out = input
            .out
            .clone()
            .unwrap_or_else(|| format!(".marionette/screenshots/{label}-t{}.png", sim.tick()));
        if read_only && (Path::new(&out).is_absolute() || !out.replace('\\', "/").starts_with(".marionette/"))
        {
            return Err(Diagnostic::error(
                crate::codes::READ_ONLY,
                "in read-only mode screenshots may only be written under .marionette/",
            )
            .fix("omit `out` to use the default location")
            .into());
        }
        let (pixels, render_ms, gpu) = render_pixels(agent, sim, scene, input.camera, size, msaa)?;
        let png = encode_png(size.0, size.1, &pixels);
        let path = agent.resolve(&out)?;
        crate::write_atomic(&path, &out, &png)?;
        Ok(ScreenshotOutput {
            path: out,
            width: size.0,
            height: size.1,
            tick: sim.tick(),
            sha256: sha256_hex(&png),
            state_hash: sim.state_hash(),
            gpu,
            render_ms: (render_ms * 1000.0).round() / 1000.0,
        })
    })?;
    if let Some(id) = &input.target.session
        && let Ok(s) = crate::session::get_mut(agent, id)
    {
        s.stats.last_render_ms = Some(out.render_ms);
        s.log("screenshot", format!("wrote {}", out.path));
    }
    Ok(out)
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LogsInput {
    pub session: String,
    /// Return entries with `seq >= cursor` (default 0).
    #[serde(default)]
    pub cursor: usize,
    pub limit: Option<usize>,
}

#[derive(Serialize, JsonSchema)]
pub struct LogsOutput {
    pub entries: Vec<LogEntry>,
    /// Pass this as `cursor` next time to get only newer entries.
    pub next_cursor: usize,
}

fn logs(agent: &mut Agent, input: LogsInput) -> Result<LogsOutput, Errors> {
    let s = crate::session::get_mut(agent, &input.session)?;
    let (entries, _) = page(&s.logs, input.cursor, input.limit);
    let next_cursor = entries.last().map_or(input.cursor, |e| e.seq + 1);
    Ok(LogsOutput { entries, next_cursor })
}

#[derive(Serialize, JsonSchema)]
pub struct StatsOutput {
    pub session: String,
    pub tick: u64,
    pub entities: usize,
    pub steps: u64,
    pub step_ms_avg: f64,
    pub step_ms_max: f64,
    pub step_ms_last: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub render_ms_last: Option<f64>,
}

fn stats(agent: &mut Agent, input: crate::session::SessionInput) -> Result<StatsOutput, Errors> {
    let s = crate::session::get_mut(agent, &input.session)?;
    let r = |x: f64| (x * 1000.0).round() / 1000.0;
    let st = s.stats;
    Ok(StatsOutput {
        session: input.session,
        tick: s.sim.tick(),
        entities: s.sim.snapshot().entities.len(),
        steps: st.steps,
        step_ms_avg: r(if st.steps > 0 {
            st.total_ms / st.steps as f64
        } else {
            0.0
        }),
        step_ms_max: r(st.max_ms),
        step_ms_last: r(st.last_ms),
        render_ms_last: st.last_render_ms,
    })
}

#[derive(Deserialize, JsonSchema)]
pub struct RaycastInput {
    #[serde(flatten)]
    pub target: WorldTarget,
    pub origin: [f32; 3],
    pub direction: [f32; 3],
    /// Default 1000.
    pub max_distance: Option<f32>,
}

#[derive(Serialize, JsonSchema)]
pub struct RaycastOutput {
    pub tick: u64,
    /// The hit, or null when the ray hits nothing (or the scene has no physics).
    pub hit: Option<RaycastHit>,
}

#[derive(Serialize, JsonSchema)]
pub struct RaycastHit {
    pub entity: u64,
    pub distance: f32,
    pub point: [f32; 3],
    pub normal: [f32; 3],
}

fn raycast(agent: &mut Agent, input: RaycastInput) -> Result<RaycastOutput, Errors> {
    with_world(agent, &input.target, |_, sim, _| {
        let hit = marionette_physics::raycast(
            &sim.world,
            input.origin,
            input.direction,
            input.max_distance.unwrap_or(1000.0),
        )
        .map(|h| RaycastHit {
            entity: h.entity,
            distance: h.distance,
            point: h.point,
            normal: h.normal,
        });
        Ok(RaycastOutput {
            tick: sim.tick(),
            hit,
        })
    })
}

#[derive(Deserialize, JsonSchema)]
pub struct EventsInput {
    #[serde(flatten)]
    pub target: WorldTarget,
    /// Only this kind (e.g. `sound`).
    pub kind: Option<String>,
    /// Only events at or after this tick.
    pub since_tick: Option<u64>,
    #[serde(default)]
    pub offset: usize,
    pub limit: Option<usize>,
}

#[derive(Serialize, JsonSchema)]
pub struct EventsOutput {
    pub tick: u64,
    pub total: usize,
    #[schemars(with = "Vec<Value>")]
    pub events: Vec<marionette_core::EngineEvent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_offset: Option<usize>,
}

fn events(agent: &mut Agent, input: EventsInput) -> Result<EventsOutput, Errors> {
    with_world(agent, &input.target, |_, sim, _| {
        let all: Vec<marionette_core::EngineEvent> = sim
            .events()
            .events
            .iter()
            .filter(|e| input.kind.as_ref().is_none_or(|k| &e.kind == k))
            .filter(|e| input.since_tick.is_none_or(|t| e.tick >= t))
            .cloned()
            .collect();
        let (events, next_offset) = page(&all, input.offset, input.limit);
        Ok(EventsOutput {
            tick: sim.tick(),
            total: all.len(),
            events,
            next_offset,
        })
    })
}
