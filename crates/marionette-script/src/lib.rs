//! Marionette game scripts (plan line 4, decision 0022).
//!
//! Game logic is TypeScript. An entity with a [`Script`] component names a `.ts` file
//! (relative to the scene). The file is an ES module that exports
//! `update(self: Entity): void`; it runs once per tick for every entity that uses it, in
//! [`Stage::Script`], in entity-id order. Types are removed with oxc (no type checking at
//! run time) and the JavaScript runs in QuickJS-ng, an interpreter with no JIT.
//!
//! Determinism: the clock (`Date`, `performance`), `Math.random`, `WeakRef` and
//! `FinalizationRegistry` are removed; `Math.sin`, `Math.pow` and every other
//! transcendental function are replaced by the pure-Rust `libm` versions so results do not
//! depend on the platform's C library; `**` is rewritten to `Math.pow`. Randomness comes from
//! `engine.rand()` (the simulation's seeded RNG). Object and `Map` iteration order is fixed by
//! the language (insertion order).
//!
//! State: the script host is rebuilt from the files whenever the world is (re)loaded or
//! restored, so module-level variables do not survive snapshots. Keep per-entity state in
//! `self.state` (saved as the [`ScriptState`] component, part of the state hash) or in
//! components.
//!
//! Errors are data: a script that fails to load fails the scene load with a `script.*`
//! diagnostic (file, line, column); an exception during a tick becomes a `script_error`
//! event and the tick goes on.

pub mod strip;
pub mod typings;

use bevy_ecs::prelude::*;
use bevy_reflect::Reflect;
use marionette_core::actions::ActionState;
use marionette_core::builtin::{DEFAULT_DT, FixedStep, SimClock};
use marionette_core::module::{EventLog, Module, RayCaster, RayFilter, SceneDir, Stage};
use marionette_core::rng::SimRng;
use marionette_core::scene::{SceneId, SceneName};
use marionette_core::{Diagnostic, Errors, Kind, Registry};
use rquickjs::function::Opt;
use rquickjs::{Context, Ctx, Exception, Function, Module as JsModule, Object, Persistent, Runtime, Value};
use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

/// Stable error codes of this crate.
pub mod codes {
    /// A script file cannot be read.
    pub const READ: &str = "script.read";
    /// A script has a syntax error (types are removed before running).
    pub const SYNTAX: &str = "script.syntax";
    /// A script throws while loading, or its module cannot be evaluated.
    pub const LOAD: &str = "script.load";
    /// A script does not export an `update` function.
    pub const NO_UPDATE: &str = "script.no_update";
    /// The JavaScript engine could not be created.
    pub const ENGINE: &str = "script.engine";
    /// One script call ran longer than [`STEP_BUDGET`](super::STEP_BUDGET) allows (an
    /// endless loop, usually). The call is stopped; the tick goes on.
    pub const BUDGET: &str = "script.budget";
}

/// Most memory the script heap of one world may use. Past it, allocations throw inside the
/// script (a `script_error` event), and the engine goes on (decision 0032).
pub const MEMORY_LIMIT: usize = 64 << 20;

/// Most work one script call (module load, or one entity's hooks in a tick) may do, in
/// QuickJS interrupt checks. The interpreter checks every fixed number of operations, so the
/// cut-off point is deterministic for a given history. About half a second of pure looping
/// on a desktop CPU.
pub const STEP_BUDGET: u64 = 50_000;

/// Most native stack one script call may use; deeper recursion throws a `RangeError`.
pub const STACK_LIMIT: usize = 512 << 10;

/// Runs a TypeScript file every tick. `path` is relative to the scene file. The module must
/// export `function update(self: Entity): void` (see the generated `marionette.d.ts`).
#[derive(Component, Reflect, Debug, Clone, PartialEq)]
pub struct Script {
    pub path: String,
}

/// A script's per-entity state (`self.state`), as JSON text. Written by the engine after
/// each `update`; part of the state hash and of snapshots.
#[derive(Component, Reflect, Debug, Clone, PartialEq)]
pub struct ScriptState {
    pub data: String,
}

/// Pending timers of one entity (`engine.after`, `engine.every`), as JSON text. Written by
/// the engine; present only on entities that ever scheduled a timer. Part of the state hash
/// and of snapshots, so timers survive save and restore.
#[derive(Component, Reflect, Debug, Clone, PartialEq)]
pub struct ScriptTimers {
    pub data: String,
}

/// Marks an entity for removal: at the end of the script stage of tick `tick` (or the first
/// later one), the entity's script gets `onDestroy`, then the entity is removed.
/// `engine.destroy` adds it; scene files may use it for a fixed lifetime.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq)]
pub struct DestroyAt {
    pub tick: u64,
}

/// The next scene id for entities that scripts spawn. Added the first time a script spawns
/// or destroys an entity; ids are never reused, so an id kept in `self.state` can never
/// point at a newer entity.
#[derive(Resource, Reflect, Debug, Clone, Copy, PartialEq)]
pub struct SpawnIds {
    pub next: u64,
}

/// Most `onDestroy` rounds per tick (an `onDestroy` may destroy more entities). Entities
/// still marked after this many rounds are handled in the next tick.
pub const MAX_DESTROY_ROUNDS: usize = 64;

/// The script module. `registry` builds the engine-wide type registry (scripts read and
/// write any registered component by name).
pub struct ScriptModule {
    registry: fn() -> Registry,
    cache: OnceLock<Arc<Registry>>,
}

impl ScriptModule {
    pub fn new(registry: fn() -> Registry) -> Self {
        Self {
            registry,
            cache: OnceLock::new(),
        }
    }
    fn registry(&self) -> Arc<Registry> {
        self.cache.get_or_init(|| Arc::new((self.registry)())).clone()
    }
}

impl Module for ScriptModule {
    fn name(&self) -> &'static str {
        "script"
    }
    fn register(&self, r: &mut Registry) {
        r.register_component::<Script>();
        r.register_component::<ScriptState>();
        r.register_component::<ScriptTimers>();
        r.register_component::<DestroyAt>();
        r.register_resource::<SpawnIds>();
    }
    fn add_systems(&self, schedule: &mut Schedule) {
        schedule.add_systems(run_scripts.in_set(Stage::Script));
    }
    fn init(&self, world: &mut World) -> marionette_core::Result<()> {
        let host = ScriptHost::build(world, self.registry())?;
        match host {
            Some(h) => world.insert_resource(h),
            None => {
                world.remove_resource::<ScriptHost>();
            }
        }
        Ok(())
    }
}

/// Re-reads every script file of `world` and replaces the running scripts (hot reload).
/// Components, `ScriptState`, the tick and the RNG are untouched. On error the old scripts
/// keep running. Returns the script paths now loaded.
pub fn reload(world: &mut World) -> marionette_core::Result<Vec<String>> {
    let Some(registry) = world.get_resource::<ScriptHost>().map(|h| h.registry.clone()) else {
        // No scripts were loaded before; nothing in this world can have changed meaning.
        return Ok(Vec::new());
    };
    match ScriptHost::build(world, registry)? {
        Some(h) => {
            let paths = h.scripts.keys().cloned().collect();
            world.insert_resource(h);
            Ok(paths)
        }
        None => {
            world.remove_resource::<ScriptHost>();
            Ok(Vec::new())
        }
    }
}

/// The world a script call may touch. Set only while `run_scripts` calls into JavaScript.
struct Bridge {
    world: *mut World,
    registry: Arc<Registry>,
    ids: BTreeMap<u64, Entity>,
    /// Scene names to ids; the lowest id wins when a name repeats.
    names: BTreeMap<String, u64>,
    current: u64,
}

// SAFETY: the pointer is only dereferenced on the thread running `run_scripts`, while that
// function holds `&mut World` and does not use it until every script call has returned.
unsafe impl Send for Bridge {}

type Shared = Arc<Mutex<Option<Bridge>>>;

/// Loaded scripts of one world. Field order matters: saved functions are dropped before
/// the context and the runtime.
#[derive(Resource)]
pub struct ScriptHost {
    scripts: BTreeMap<String, Persistent<Object<'static>>>,
    /// Scripts first used by a spawned entity that failed to load (reported once).
    failed: std::collections::BTreeSet<String>,
    run: Persistent<Function<'static>>,
    destroy: Persistent<Function<'static>>,
    bridge: Shared,
    registry: Arc<Registry>,
    /// Interrupt checks used by the current call; reset before each call.
    fuel: Arc<AtomicU64>,
    ctx: Context,
    rt: Runtime,
}

// SAFETY: built with rquickjs' `parallel` feature, so `Runtime` and `Context` are Send + Sync
// and every use of the saved values goes through `Context::with`, which locks the runtime.
// `Persistent` is not marked Send/Sync by rquickjs only because it cannot know that; the
// saved values here are restored only inside `with` and dropped with the host, before the
// context and runtime (field order).
unsafe impl Send for ScriptHost {}
unsafe impl Sync for ScriptHost {}

fn engine_error(e: impl std::fmt::Display) -> Errors {
    Diagnostic::error(codes::ENGINE, format!("cannot create the script engine: {e}")).into()
}

/// The pending exception as text (message and stack).
fn exception_text(ctx: &Ctx<'_>, e: rquickjs::Error) -> String {
    if !matches!(e, rquickjs::Error::Exception) {
        return e.to_string();
    }
    let v: Value = ctx.catch();
    if let Some(ex) = v.as_exception() {
        let msg = ex.message().unwrap_or_default();
        let stack = ex.stack().unwrap_or_default();
        return if stack.is_empty() {
            msg
        } else {
            format!("{msg}\n{}", stack.trim_end())
        };
    }
    ctx.json_stringify(v)
        .ok()
        .flatten()
        .and_then(|s| s.to_string().ok())
        .unwrap_or_else(|| "unknown exception".into())
}

/// Reads a script file and removes its types.
fn source(dir: &std::path::Path, p: &str) -> marionette_core::Result<String> {
    let text = std::fs::read_to_string(dir.join(p)).map_err(|e| {
        Errors::from(
            Diagnostic::error(codes::READ, format!("cannot read script `{p}`: {e}"))
                .component("Script")
                .fix("check `Script.path`; it is relative to the scene file"),
        )
    })?;
    strip::strip(&text, p).map_err(|e| {
        Errors::from(
            Diagnostic::error(codes::SYNTAX, e.message)
                .file(p.to_string())
                .at(e.line, e.column),
        )
    })
}

/// Evaluates one script module and returns its namespace.
fn declare<'js>(
    ctx: &Ctx<'js>,
    fuel: &AtomicU64,
    path: &str,
    js: String,
) -> marionette_core::Result<Object<'js>> {
    let load_err =
        |msg: String| -> Errors { Diagnostic::error(codes::LOAD, msg).file(path.to_string()).into() };
    let declared =
        JsModule::declare(ctx.clone(), path.to_string(), js).map_err(|e| load_err(exception_text(ctx, e)))?;
    fuel.store(0, Ordering::Relaxed);
    let module = guarded(ctx, fuel, || {
        let (module, promise) = declared.eval()?;
        promise.finish::<()>()?;
        Ok(module)
    })
    .map_err(load_err)?;
    let ns: Object = module.namespace().map_err(|e| load_err(exception_text(ctx, e)))?;
    let update: Option<Function> = ns.get("update").ok();
    if update.is_none() {
        return Err(
            Diagnostic::error(codes::NO_UPDATE, "the script does not export `update`")
                .file(path.to_string())
                .fix("add `export function update(self: Entity): void { ... }`")
                .into(),
        );
    }
    Ok(ns)
}

fn scene_dir(world: &World) -> std::path::PathBuf {
    world
        .get_resource::<SceneDir>()
        .map(|d| d.0.clone())
        .unwrap_or_default()
}

impl ScriptHost {
    /// Builds a host for every distinct `Script.path` in `world`; `None` if there are none.
    fn build(world: &mut World, registry: Arc<Registry>) -> marionette_core::Result<Option<Self>> {
        let mut q = world.query::<&Script>();
        let paths: std::collections::BTreeSet<String> = q.iter(world).map(|s| s.path.clone()).collect();
        if paths.is_empty() {
            return Ok(None);
        }
        let dir = scene_dir(world);
        let mut sources = Vec::new();
        for p in &paths {
            sources.push((p.clone(), source(&dir, p)?));
        }

        let rt = Runtime::new().map_err(engine_error)?;
        // Runaway scripts fail as data instead of taking the engine down (decision 0032):
        // bounded heap, bounded stack, bounded work per call.
        rt.set_memory_limit(MEMORY_LIMIT);
        rt.set_max_stack_size(STACK_LIMIT);
        let fuel = Arc::new(AtomicU64::new(0));
        let f = fuel.clone();
        rt.set_interrupt_handler(Some(Box::new(move || {
            f.fetch_add(1, Ordering::Relaxed) >= STEP_BUDGET
        })));
        let ctx = Context::full(&rt).map_err(engine_error)?;
        let bridge: Shared = Arc::new(Mutex::new(None));
        ctx.with(|ctx| install(&ctx, &bridge)).map_err(engine_error)?;
        let (scripts, run, destroy) = ctx.with(|ctx| -> marionette_core::Result<_> {
            ctx.eval::<(), _>(include_str!("prelude.js"))
                .map_err(|e| engine_error(exception_text(&ctx, e)))?;
            let run: Function = ctx.globals().get("__run").map_err(engine_error)?;
            let destroy: Function = ctx.globals().get("__destroy").map_err(engine_error)?;
            let mut scripts = BTreeMap::new();
            for (path, js) in sources {
                let ns = declare(&ctx, &fuel, &path, js)?;
                scripts.insert(path.clone(), Persistent::save(&ctx, ns));
            }
            Ok((
                scripts,
                Persistent::save(&ctx, run),
                Persistent::save(&ctx, destroy),
            ))
        })?;
        Ok(Some(Self {
            scripts,
            failed: Default::default(),
            run,
            destroy,
            bridge,
            registry,
            fuel,
            ctx,
            rt,
        }))
    }

    /// Loads scripts that entities spawned this tick use for the first time. A script that
    /// fails is reported once as a `script_error` event; its entities do nothing until the
    /// next reload.
    fn load_new(&mut self, world: &mut World, tick: u64) {
        let mut q = world.query::<&Script>();
        let missing: std::collections::BTreeSet<String> = q
            .iter(world)
            .filter(|s| !self.scripts.contains_key(&s.path) && !self.failed.contains(&s.path))
            .map(|s| s.path.clone())
            .collect();
        if missing.is_empty() {
            return;
        }
        let dir = scene_dir(world);
        for path in missing {
            let loaded = source(&dir, &path).and_then(|js| {
                self.ctx
                    .with(|ctx| declare(&ctx, &self.fuel, &path, js).map(|ns| Persistent::save(&ctx, ns)))
            });
            match loaded {
                Ok(ns) => {
                    self.scripts.insert(path, ns);
                }
                Err(errs) => {
                    let message = errs.0.first().map(|d| format!("{}: {}", d.code, d.message));
                    world.resource_mut::<EventLog>().push(
                        tick,
                        "script_error",
                        None,
                        None,
                        serde_json::json!({ "script": path, "message": message.unwrap_or_default() }),
                    );
                    self.failed.insert(path);
                }
            }
        }
    }
}

/// Runs one call into JavaScript and turns any failure into a message. A panic in engine
/// code under the call (a bug, or an engine function used where it cannot work) is caught
/// here, so it cannot unwind through the engine; rquickjs re-raises such panics when the
/// exception is read, so the exception text is read inside the guard too. Budget overruns
/// get the `script.budget` code.
fn guarded<R>(ctx: &Ctx<'_>, fuel: &AtomicU64, f: impl FnOnce() -> rquickjs::Result<R>) -> Result<R, String> {
    match catch_unwind(AssertUnwindSafe(|| f().map_err(|e| exception_text(ctx, e)))) {
        Ok(Ok(r)) => Ok(r),
        Ok(Err(text)) if fuel.load(Ordering::Relaxed) > STEP_BUDGET => Err(format!(
            "{}: the call ran past its work budget ({STEP_BUDGET} interrupt checks) and was stopped; look for an endless loop\n{text}",
            codes::BUDGET
        )),
        Ok(Err(text)) => Err(text),
        Err(payload) => {
            let msg = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "engine panic".into());
            Err(format!("script.engine_panic: {msg}"))
        }
    }
}

/// The bridge lock. A panic while it was held (caught by [`guarded`]) leaves it poisoned;
/// its data is still consistent (plain values), so poisoning is ignored.
fn bridge_lock(bridge: &Shared) -> MutexGuard<'_, Option<Bridge>> {
    bridge.lock().unwrap_or_else(|p| p.into_inner())
}

impl ScriptHost {
    /// Bytes the script heap of this world uses now (QuickJS's own count).
    pub fn heap_bytes(&self) -> u64 {
        self.rt.memory_usage().memory_used_size.max(0) as u64
    }
}

/// Bytes the script heap of `world` uses, or `None` if it runs no scripts.
pub fn heap_bytes(world: &World) -> Option<u64> {
    world.get_resource::<ScriptHost>().map(ScriptHost::heap_bytes)
}

/// Runs `f` with the world of the current script call.
fn with_world<R>(bridge: &Shared, f: impl FnOnce(&mut World, &mut Bridge) -> R) -> R {
    let mut guard = bridge_lock(bridge);
    let b = guard
        .as_mut()
        .expect("engine functions are only callable during update()");
    // SAFETY: see `Bridge`.
    let world = unsafe { &mut *b.world };
    f(world, b)
}

fn throw(ctx: &Ctx<'_>, msg: String) -> rquickjs::Error {
    Exception::throw_message(ctx, &msg)
}

/// `Entity.playClip` / `blendClip` / `stopClip`: the same `Animator` changes as the
/// `anim_play`, `anim_blend` and `anim_stop` operations (one implementation, in
/// `marionette-anim`). Unknown clips throw `anim.unknown_clip`.
fn animate(world: &mut World, e: Entity, json: &str) -> Result<(), String> {
    use marionette_anim::{Animator, Rigs};
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|err| format!("script.bad_value: {err}"))?;
    let num = |k: &str, d: f64| v.get(k).and_then(serde_json::Value::as_f64).unwrap_or(d) as f32;
    let looping = v
        .get("looping")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true);
    let clip = v.get("clip").and_then(serde_json::Value::as_str);
    if let Some(clip) = clip {
        let model = world
            .get::<marionette_core::spatial::Model>(e)
            .ok_or_else(|| "anim.no_model: this entity has no Model".to_string())?;
        let rig = world
            .get_resource::<Rigs>()
            .and_then(|r| r.0.get(&model.asset))
            .ok_or_else(|| {
                format!(
                    "anim.asset: `{}` was not loaded as an animated model",
                    model.asset
                )
            })?;
        if rig.clip(clip).is_none() {
            let names: Vec<&str> = rig.clips.iter().map(|c| c.name.as_str()).collect();
            return Err(format!(
                "anim.unknown_clip: `{}` has no clip `{clip}` (clips: {})",
                model.asset,
                names.join(", ")
            ));
        }
    }
    let mut animator = world.get::<Animator>(e).cloned().unwrap_or_default();
    match (v.get("op").and_then(serde_json::Value::as_str), clip) {
        (Some("play"), Some(c)) => animator.play(c, num("fade", 0.2), num("speed", 1.0), looping),
        (Some("blend"), Some(c)) => animator.blend(
            c,
            num("weight", 1.0),
            num("fade", 0.2),
            num("speed", 1.0),
            looping,
        ),
        (Some("stop"), c) => animator.stop(c, num("fade", 0.2)),
        _ => return Err("script.bad_value: unknown animation call".into()),
    }
    world.entity_mut(e).insert(animator);
    Ok(())
}

/// Native functions behind the prelude, the deterministic math, and the removed globals.
fn install(ctx: &Ctx<'_>, bridge: &Shared) -> rquickjs::Result<()> {
    let g = ctx.globals();
    for name in ["Date", "WeakRef", "FinalizationRegistry", "performance"] {
        g.remove(name)?;
    }
    let math: Object = g.get("Math")?;
    math.remove("random")?;
    macro_rules! math1 {
        ($($name:literal => $f:path),* $(,)?) => {
            $( math.set($name, Function::new(ctx.clone(), |x: f64| $f(x))?)?; )*
        };
    }
    math1! {
        "sin" => libm::sin, "cos" => libm::cos, "tan" => libm::tan,
        "asin" => libm::asin, "acos" => libm::acos, "atan" => libm::atan,
        "sinh" => libm::sinh, "cosh" => libm::cosh, "tanh" => libm::tanh,
        "asinh" => libm::asinh, "acosh" => libm::acosh, "atanh" => libm::atanh,
        "exp" => libm::exp, "expm1" => libm::expm1, "log" => libm::log, "log1p" => libm::log1p,
        "log2" => libm::log2, "log10" => libm::log10, "cbrt" => libm::cbrt,
    }
    math.set(
        "atan2",
        Function::new(ctx.clone(), |y: f64, x: f64| libm::atan2(y, x))?,
    )?;
    math.set("pow", Function::new(ctx.clone(), |x: f64, y: f64| js_pow(x, y))?)?;
    math.set(
        "hypot",
        Function::new(ctx.clone(), |args: rquickjs::function::Rest<f64>| {
            args.0.iter().fold(0.0, |acc: f64, &v| libm::hypot(acc, v))
        })?,
    )?;

    let h = Object::new(ctx.clone())?;
    let b = bridge.clone();
    h.set(
        "get",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'_>, id: f64, name: String| -> rquickjs::Result<Option<String>> {
                with_world(&b, |world, br| {
                    let entry = component(&ctx, &br.registry, &name)?;
                    let Some(&e) = br.ids.get(&(id as u64)) else {
                        return Ok(None);
                    };
                    Ok(entry
                        .get(world, Some(e))
                        .map(|v| br.registry.to_json(v.as_partial_reflect()).to_string()))
                })
            },
        )?,
    )?;
    let b = bridge.clone();
    h.set(
        "set",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'_>, id: f64, name: String, json: String| -> rquickjs::Result<()> {
                with_world(&b, |world, br| {
                    let entry = component(&ctx, &br.registry, &name)?;
                    let Some(&e) = br.ids.get(&(id as u64)) else {
                        return Err(throw(
                            &ctx,
                            format!("script.no_entity: entity {id} does not exist"),
                        ));
                    };
                    let value: serde_json::Value = serde_json::from_str(&json)
                        .map_err(|err| throw(&ctx, format!("script.bad_value: {err}")))?;
                    let boxed = br.registry.from_json(entry, &value).map_err(|err| {
                        throw(
                            &ctx,
                            format!("script.bad_value: `{name}` does not accept {value}: {err}"),
                        )
                    })?;
                    entry.insert(world, Some(e), boxed);
                    Ok(())
                })
            },
        )?,
    )?;
    let b = bridge.clone();
    h.set(
        "has",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'_>, id: f64, name: String| -> rquickjs::Result<bool> {
                with_world(&b, |world, br| {
                    let entry = component(&ctx, &br.registry, &name)?;
                    Ok(br
                        .ids
                        .get(&(id as u64))
                        .is_some_and(|&e| entry.get(world, Some(e)).is_some()))
                })
            },
        )?,
    )?;
    let b = bridge.clone();
    h.set(
        "anim",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'_>, id: f64, json: String| -> rquickjs::Result<()> {
                with_world(&b, |world, br| {
                    let Some(&e) = br.ids.get(&(id as u64)) else {
                        return Err(throw(
                            &ctx,
                            format!("script.no_entity: entity {id} does not exist"),
                        ));
                    };
                    animate(world, e, &json).map_err(|msg| throw(&ctx, msg))
                })
            },
        )?,
    )?;
    let b = bridge.clone();
    h.set(
        "exists",
        Function::new(ctx.clone(), move |id: f64| {
            with_world(&b, |_, br| br.ids.contains_key(&(id as u64)))
        })?,
    )?;
    let b = bridge.clone();
    h.set(
        "name",
        Function::new(ctx.clone(), move |id: f64| {
            with_world(&b, |world, br| {
                let e = *br.ids.get(&(id as u64))?;
                world.get::<SceneName>(e).map(|n| n.0.clone())
            })
        })?,
    )?;
    let b = bridge.clone();
    h.set(
        "find",
        Function::new(ctx.clone(), move |name: String| {
            with_world(&b, |_, br| br.names.get(&name).map(|id| *id as f64))
        })?,
    )?;
    let b = bridge.clone();
    h.set(
        "query",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'_>, name: String| -> rquickjs::Result<Vec<f64>> {
                with_world(&b, |world, br| {
                    let entry = component(&ctx, &br.registry, &name)?;
                    Ok(br
                        .ids
                        .iter()
                        .filter(|(_, e)| entry.get(world, Some(**e)).is_some())
                        .map(|(id, _)| *id as f64)
                        .collect())
                })
            },
        )?,
    )?;
    let b = bridge.clone();
    h.set(
        "resource",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'_>, name: String| -> rquickjs::Result<Option<String>> {
                with_world(&b, |world, br| {
                    let entry = br
                        .registry
                        .get(&name)
                        .filter(|e| e.kind == Kind::Resource)
                        .ok_or_else(|| {
                            throw(
                                &ctx,
                                format!("script.unknown_resource: `{name}` is not a resource"),
                            )
                        })?;
                    Ok(entry
                        .get(world, None)
                        .map(|v| br.registry.to_json(v.as_partial_reflect()).to_string()))
                })
            },
        )?,
    )?;
    let b = bridge.clone();
    h.set(
        "tick",
        Function::new(ctx.clone(), move || {
            with_world(&b, |w, _| {
                w.get_resource::<SimClock>().map_or(0, |c| c.tick) as f64
            })
        })?,
    )?;
    let b = bridge.clone();
    h.set(
        "dt",
        Function::new(ctx.clone(), move || {
            with_world(&b, |w, _| {
                w.get_resource::<FixedStep>().map_or(DEFAULT_DT, |s| s.dt) as f64
            })
        })?,
    )?;
    let b = bridge.clone();
    h.set(
        "rand",
        Function::new(ctx.clone(), move || {
            with_world(&b, |w, _| {
                w.get_resource_mut::<SimRng>()
                    .map_or(0.0, |mut r| r.next_f32() as f64)
            })
        })?,
    )?;
    let b = bridge.clone();
    h.set(
        "action",
        Function::new(ctx.clone(), move |name: String| {
            action(&b, &name, |a, n| a.value(n) as f64)
        })?,
    )?;
    let b = bridge.clone();
    h.set(
        "pressed",
        Function::new(ctx.clone(), move |name: String| {
            action(&b, &name, |a, n| a.pressed(n))
        })?,
    )?;
    let b = bridge.clone();
    h.set(
        "just_pressed",
        Function::new(ctx.clone(), move |name: String| {
            action(&b, &name, |a, n| a.just_pressed(n))
        })?,
    )?;
    let b = bridge.clone();
    h.set(
        "emit",
        Function::new(ctx.clone(), move |kind: String, detail: Opt<String>| {
            event(
                &b,
                &kind,
                detail
                    .0
                    .and_then(|d| serde_json::from_str(&d).ok())
                    .unwrap_or_default(),
            )
        })?,
    )?;
    let b = bridge.clone();
    h.set(
        "log",
        Function::new(ctx.clone(), move |message: String| {
            event(&b, "script_log", serde_json::json!({ "message": message }))
        })?,
    )?;
    let b = bridge.clone();
    h.set(
        "spawn",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'_>, json: String, name: Option<String>| -> rquickjs::Result<f64> {
                with_world(&b, |world, br| {
                    let comps: serde_json::Value = serde_json::from_str(&json)
                        .map_err(|err| throw(&ctx, format!("script.bad_value: {err}")))?;
                    let serde_json::Value::Object(comps) = comps else {
                        return Err(throw(
                            &ctx,
                            "script.bad_value: spawn takes an object of components".into(),
                        ));
                    };
                    spawn(&ctx, world, br, comps, name).map(|id| id as f64)
                })
            },
        )?,
    )?;
    let b = bridge.clone();
    h.set(
        "clone",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'_>, source: f64, json: String, name: Option<String>| -> rquickjs::Result<f64> {
                with_world(&b, |world, br| {
                    let Some(&e) = br.ids.get(&(source as u64)) else {
                        return Err(throw(
                            &ctx,
                            format!("script.no_entity: entity {source} does not exist"),
                        ));
                    };
                    let overrides: serde_json::Value = serde_json::from_str(&json)
                        .map_err(|err| throw(&ctx, format!("script.bad_value: {err}")))?;
                    let serde_json::Value::Object(overrides) = overrides else {
                        return Err(throw(
                            &ctx,
                            "script.bad_value: clone overrides must be an object of components".into(),
                        ));
                    };
                    let mut comps = serde_json::Map::new();
                    for entry in br.registry.of_kind(Kind::Component) {
                        // A copy starts fresh: its own `start`, no timers, not marked for removal.
                        if NOT_CLONED.contains(&entry.name) {
                            continue;
                        }
                        if let Some(v) = entry.get(world, Some(e)) {
                            comps.insert(
                                entry.name.to_string(),
                                br.registry.to_json(v.as_partial_reflect()),
                            );
                        }
                    }
                    comps.extend(overrides);
                    let name = name.or_else(|| world.get::<SceneName>(e).map(|n| n.0.clone()));
                    spawn(&ctx, world, br, comps, name).map(|id| id as f64)
                })
            },
        )?,
    )?;
    let b = bridge.clone();
    h.set(
        "destroy",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'_>, id: f64, after_ticks: f64| -> rquickjs::Result<()> {
                with_world(&b, |world, br| {
                    let Some(&e) = br.ids.get(&(id as u64)) else {
                        return Err(throw(
                            &ctx,
                            format!("script.no_entity: entity {id} does not exist"),
                        ));
                    };
                    let now = world.get_resource::<SimClock>().map_or(0, |c| c.tick);
                    let at = now + after_ticks.max(0.0) as u64;
                    // The earliest removal wins (Unity's Destroy behaves the same way).
                    let at = world.get::<DestroyAt>(e).map_or(at, |d| d.tick.min(at));
                    world.entity_mut(e).insert(DestroyAt { tick: at });
                    Ok(())
                })
            },
        )?,
    )?;
    let b = bridge.clone();
    h.set(
        "raycast",
        Function::new(
            ctx.clone(),
            move |o: Vec<f64>, d: Vec<f64>, max: f64, exclude: Option<f64>| -> Option<String> {
                with_world(&b, |world, _| {
                    let caster = world.get_resource::<RayCaster>().copied()?;
                    let v = |a: &[f64]| [0, 1, 2].map(|i| a.get(i).copied().unwrap_or(0.0) as f32);
                    let filter = RayFilter {
                        exclude: exclude.map(|e| e as u64),
                        skip_sensors: true,
                    };
                    let hit = (caster.0)(world, v(&o), v(&d), max as f32, filter)?;
                    let p = |a: [f32; 3]| serde_json::json!({ "x": a[0], "y": a[1], "z": a[2] });
                    Some(
                        serde_json::json!({
                            "entity": hit.entity,
                            "distance": hit.distance,
                            "point": p(hit.point),
                            "normal": p(hit.normal),
                        })
                        .to_string(),
                    )
                })
            },
        )?,
    )?;
    g.set("__host", h)?;
    Ok(())
}

/// Components a clone does not copy from its source.
const NOT_CLONED: [&str; 3] = ["ScriptState", "ScriptTimers", "DestroyAt"];

/// The next unused scene id; ids are never reused (see [`SpawnIds`]).
fn next_id(world: &mut World, ids: &BTreeMap<u64, Entity>) -> u64 {
    let max_live = ids.keys().next_back().copied().unwrap_or(0);
    let next = world.get_resource::<SpawnIds>().map_or(0, |s| s.next);
    let id = next.max(max_live + 1);
    world.insert_resource(SpawnIds { next: id + 1 });
    id
}

/// Spawns a scene entity now, after checking every component value first (nothing is
/// spawned if one is wrong). Its script, if any, starts on the next tick.
fn spawn(
    ctx: &Ctx<'_>,
    world: &mut World,
    br: &mut Bridge,
    comps: serde_json::Map<String, serde_json::Value>,
    name: Option<String>,
) -> rquickjs::Result<u64> {
    let mut values = Vec::new();
    for (k, v) in &comps {
        let entry = component(ctx, &br.registry, k)?;
        let boxed = br
            .registry
            .from_json(entry, v)
            .map_err(|err| throw(ctx, format!("script.bad_value: `{k}` does not accept {v}: {err}")))?;
        values.push((entry, boxed));
    }
    let id = next_id(world, &br.ids);
    let mut ent = world.spawn(SceneId(id));
    if let Some(n) = &name {
        ent.insert(SceneName(n.clone()));
    }
    let e = ent.id();
    for (entry, boxed) in values {
        entry.insert(world, Some(e), boxed);
    }
    br.ids.insert(id, e);
    if let Some(n) = name {
        br.names.entry(n).or_insert(id);
    }
    Ok(id)
}

fn action<R: Default>(bridge: &Shared, name: &str, f: impl FnOnce(&ActionState, &str) -> R) -> R {
    with_world(bridge, |w, _| {
        w.get_resource::<ActionState>()
            .map(|a| f(a, name))
            .unwrap_or_default()
    })
}

fn event(bridge: &Shared, kind: &str, detail: serde_json::Value) {
    with_world(bridge, |w, br| {
        let tick = w.get_resource::<SimClock>().map_or(0, |c| c.tick);
        if let Some(mut log) = w.get_resource_mut::<EventLog>() {
            log.push(tick, kind, Some(br.current), None, detail);
        }
    })
}

fn component<'r>(
    ctx: &Ctx<'_>,
    registry: &'r Registry,
    name: &str,
) -> rquickjs::Result<&'r marionette_core::registry::TypeEntry> {
    registry
        .get(name)
        .filter(|e| e.kind == Kind::Component)
        .ok_or_else(|| {
            throw(
                ctx,
                format!("script.unknown_component: `{name}` is not a component"),
            )
        })
}

/// `Math.pow` with the special cases JavaScript defines and C's `pow` does not.
fn js_pow(x: f64, y: f64) -> f64 {
    if y.is_nan() || ((x == 1.0 || x == -1.0) && y.is_infinite()) {
        return f64::NAN;
    }
    libm::pow(x, y)
}

/// Contacts of one entity from the previous tick, other entity ids in event order.
#[derive(Default, Clone)]
struct Contacts {
    collision_enter: Vec<f64>,
    collision_exit: Vec<f64>,
    trigger_enter: Vec<f64>,
    trigger_exit: Vec<f64>,
}

impl<'js> rquickjs::IntoJs<'js> for Contacts {
    fn into_js(self, ctx: &Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        let o = Object::new(ctx.clone())?;
        o.set("collisionEnter", self.collision_enter)?;
        o.set("collisionExit", self.collision_exit)?;
        o.set("triggerEnter", self.trigger_enter)?;
        o.set("triggerExit", self.trigger_exit)?;
        Ok(o.into_value())
    }
}

fn contacts(
    world: &World,
    ids: &BTreeMap<u64, Entity>,
    registry: &Registry,
    tick: u64,
) -> BTreeMap<u64, Contacts> {
    let mut out: BTreeMap<u64, Contacts> = BTreeMap::new();
    if tick == 0 {
        return out;
    }
    let mut events = Vec::new();
    for ev in world.resource::<EventLog>().of_tick(tick - 1) {
        let enter = match ev.kind.as_str() {
            "collision_started" => true,
            "collision_stopped" => false,
            _ => continue,
        };
        if let (Some(a), Some(b)) = (ev.entity, ev.other) {
            events.push((enter, a, b));
        }
    }
    let sensor = |id: u64| -> bool {
        let (Some(entry), Some(&e)) = (registry.get("Collider"), ids.get(&id)) else {
            return false;
        };
        entry.get(world, Some(e)).is_some_and(|v| {
            registry.to_json(v.as_partial_reflect())["sensor"] == serde_json::Value::Bool(true)
        })
    };
    for (enter, a, b) in events {
        let trigger = sensor(a) || sensor(b);
        for (me, other) in [(a, b), (b, a)] {
            let c = out.entry(me).or_default();
            let list = match (trigger, enter) {
                (false, true) => &mut c.collision_enter,
                (false, false) => &mut c.collision_exit,
                (true, true) => &mut c.trigger_enter,
                (true, false) => &mut c.trigger_exit,
            };
            list.push(other as f64);
        }
    }
    out
}

/// Scene ids and names of every scene entity; the lowest id wins when a name repeats.
fn index(world: &mut World) -> (BTreeMap<u64, Entity>, BTreeMap<String, u64>) {
    let mut q = world.query::<(Entity, &SceneId, Option<&SceneName>)>();
    let mut ids = BTreeMap::new();
    let mut names = BTreeMap::new();
    for (e, id, name) in q.iter(world) {
        ids.insert(id.0, e);
        if let Some(n) = name {
            names
                .entry(n.0.clone())
                .and_modify(|v: &mut u64| *v = (*v).min(id.0))
                .or_insert(id.0);
        }
    }
    (ids, names)
}

/// What one script call returned: state text (absent after an error), timers text (absent
/// when the entity never used timers) and the error text, if the call threw.
struct Outcome {
    state: Option<String>,
    timers: Option<String>,
    error: Option<String>,
}

fn outcome(r: Result<rquickjs::Array<'_>, String>) -> Outcome {
    match r {
        Ok(a) => Outcome {
            state: a.get(0).ok().flatten(),
            timers: a.get(1).ok().flatten(),
            error: a.get(2).ok().flatten(),
        },
        Err(err) => Outcome {
            state: None,
            timers: None,
            error: Some(err),
        },
    }
}

/// One entity's finished turn: id, entity, state and timers before, outcome, script path.
type Turn = (u64, Entity, Option<String>, Option<String>, Outcome, String);

fn script_error(world: &mut World, tick: u64, id: Option<u64>, path: &str, message: String) {
    world.resource_mut::<EventLog>().push(
        tick,
        "script_error",
        id,
        None,
        serde_json::json!({ "script": path, "message": message }),
    );
}

/// The `Stage::Script` system: calls every script's hooks for its entities, by id; then
/// removes entities marked with [`DestroyAt`] (after their `onDestroy`); then loads scripts
/// that spawned entities use for the first time.
fn run_scripts(world: &mut World) {
    let tick = world.get_resource::<SimClock>().map_or(0, |c| c.tick);
    let mut host = world.remove_resource::<ScriptHost>();
    if let Some(host) = &host {
        turns(world, host, tick);
    }
    destroy_marked(world, host.as_ref(), tick);
    if let Some(mut host) = host.take() {
        host.load_new(world, tick);
        world.insert_resource(host);
    }
}

fn turns(world: &mut World, host: &ScriptHost, tick: u64) {
    let (ids, names) = index(world);
    let mut q = world.query::<(&SceneId, &Script, Option<&ScriptState>, Option<&ScriptTimers>)>();
    let mut work: Vec<_> = q
        .iter(world)
        .map(|(id, s, state, timers)| {
            (
                id.0,
                s.path.clone(),
                state.map(|s| s.data.clone()),
                timers.map(|t| t.data.clone()),
            )
        })
        .collect();
    work.sort_by_key(|w| w.0);
    // Physics runs after scripts, so contacts reported last tick are delivered now. A
    // contact involving a sensor collider goes to the trigger hooks instead (Unity's split).
    let contacts = contacts(world, &ids, &host.registry, tick);
    let entity_of = ids.clone();
    *bridge_lock(&host.bridge) = Some(Bridge {
        world: world as *mut World,
        registry: host.registry.clone(),
        ids,
        names,
        current: 0,
    });
    // Results are applied after all calls, so no `&mut World` is used while scripts run.
    let mut results: Vec<Turn> = Vec::new();
    host.ctx.with(|ctx| {
        let run = host.run.clone().restore(&ctx).expect("prelude function");
        for (id, path, state, timers) in work {
            let Some(module) = host.scripts.get(&path) else {
                continue;
            };
            let module = module.clone().restore(&ctx).expect("script module");
            let c = contacts.get(&id).cloned().unwrap_or_default();
            if let Some(b) = bridge_lock(&host.bridge).as_mut() {
                b.current = id;
            }
            // No `ScriptState` yet means `start` has not run for this entity.
            let is_new = state.is_none();
            let state_in = state.clone().unwrap_or_else(|| "{}".into());
            host.fuel.store(0, Ordering::Relaxed);
            let out = outcome(guarded(&ctx, &host.fuel, || {
                let a = run.call((module, id as f64, state_in, is_new, c, timers.clone()))?;
                while ctx.execute_pending_job() {}
                Ok(a)
            }));
            results.push((id, entity_of[&id], state, timers, out, path));
        }
    });
    *bridge_lock(&host.bridge) = None;
    for (id, e, state_before, timers_before, out, path) in results {
        // Timers are saved even when the turn threw: a one-shot timer that fired is gone,
        // so a failing callback is not called again every tick.
        if let Some(t) = out.timers
            && timers_before.as_ref() != Some(&t)
        {
            world.entity_mut(e).insert(ScriptTimers { data: t });
        }
        if let Some(after) = out.state {
            // Always written the first time, so its presence records that `start` ran.
            if state_before.as_ref() != Some(&after) {
                world.entity_mut(e).insert(ScriptState { data: after });
            }
        }
        if let Some(message) = out.error {
            script_error(world, tick, Some(id), &path, message);
        }
    }
}

/// Removes entities whose [`DestroyAt`] tick has come, in id order, calling `onDestroy`
/// first. An `onDestroy` may mark more entities; they go in the next round, up to
/// [`MAX_DESTROY_ROUNDS`] rounds per tick.
fn destroy_marked(world: &mut World, host: Option<&ScriptHost>, tick: u64) {
    for _ in 0..MAX_DESTROY_ROUNDS {
        let mut q = world.query::<(
            Entity,
            &SceneId,
            &DestroyAt,
            Option<&Script>,
            Option<&ScriptState>,
        )>();
        let mut due: Vec<_> = q
            .iter(world)
            .filter(|(_, _, d, _, _)| d.tick <= tick)
            .map(|(e, id, _, s, st)| (id.0, e, s.map(|s| s.path.clone()), st.map(|s| s.data.clone())))
            .collect();
        if due.is_empty() {
            return;
        }
        due.sort_by_key(|d| d.0);
        if let Some(host) = host
            && due.iter().any(|d| d.2.is_some())
        {
            let (ids, names) = index(world);
            *bridge_lock(&host.bridge) = Some(Bridge {
                world: world as *mut World,
                registry: host.registry.clone(),
                ids,
                names,
                current: 0,
            });
            let mut errors = Vec::new();
            host.ctx.with(|ctx| {
                let destroy = host.destroy.clone().restore(&ctx).expect("prelude function");
                for (id, _, path, state) in &due {
                    let (Some(path), Some(module)) = (path, path.as_ref().and_then(|p| host.scripts.get(p)))
                    else {
                        continue;
                    };
                    let module = module.clone().restore(&ctx).expect("script module");
                    if let Some(b) = bridge_lock(&host.bridge).as_mut() {
                        b.current = *id;
                    }
                    let state_in = state.clone().unwrap_or_else(|| "{}".into());
                    host.fuel.store(0, Ordering::Relaxed);
                    let out = outcome(guarded(&ctx, &host.fuel, || {
                        let a = destroy.call((module, *id as f64, state_in))?;
                        while ctx.execute_pending_job() {}
                        Ok(a)
                    }));
                    if let Some(message) = out.error {
                        errors.push((*id, path.clone(), message));
                    }
                }
            });
            *bridge_lock(&host.bridge) = None;
            for (id, path, message) in errors {
                script_error(world, tick, Some(id), &path, message);
            }
        }
        let mut highest = 0;
        for (id, e, _, _) in due {
            world.despawn(e);
            highest = highest.max(id);
            world
                .resource_mut::<EventLog>()
                .push(tick, "destroyed", Some(id), None, serde_json::Value::Null);
        }
        // Ids are never reused, also for entities from the scene file.
        let next = world.get_resource::<SpawnIds>().map_or(0, |s| s.next);
        world.insert_resource(SpawnIds {
            next: next.max(highest + 1),
        });
    }
}
