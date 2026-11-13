//! `marionette.d.ts`: TypeScript declarations for scripts, generated from the schema, so
//! component types in scripts cannot drift from the engine. Global declarations (no
//! imports needed); check scripts with `tsc --noEmit --strict` next to this file.

use marionette_core::Registry;
use serde_json::Value;
use std::fmt::Write;

fn doc(out: &mut String, indent: &str, text: Option<&str>) {
    if let Some(t) = text.filter(|t| !t.is_empty()) {
        let _ = writeln!(
            out,
            "{indent}/** {} */",
            t.replace("*/", "* /").replace('\n', " ")
        );
    }
}

/// TypeScript type for one JSON Schema node (the subset `marionette_core::schema` emits).
fn ty(s: &Value, indent: &str) -> String {
    if let Some(r) = s.get("$ref").and_then(Value::as_str) {
        return r.rsplit('/').next().unwrap_or("unknown").to_string();
    }
    if let Some(c) = s.get("const") {
        return c.to_string();
    }
    if let Some(alts) = s
        .get("oneOf")
        .or_else(|| s.get("anyOf"))
        .and_then(Value::as_array)
    {
        let parts: Vec<String> = alts.iter().map(|a| ty(a, indent)).collect();
        return parts.join(" | ");
    }
    if let Some(e) = s.get("enum").and_then(Value::as_array) {
        return e.iter().map(Value::to_string).collect::<Vec<_>>().join(" | ");
    }
    match s.get("type") {
        Some(Value::Array(types)) => types
            .iter()
            .map(|t| ty(&serde_json::json!({ "type": t }), indent))
            .collect::<Vec<_>>()
            .join(" | "),
        Some(Value::String(t)) => match t.as_str() {
            "number" | "integer" => "number".into(),
            "string" => "string".into(),
            "boolean" => "boolean".into(),
            "null" => "null".into(),
            "array" => match (s.get("prefixItems").and_then(Value::as_array), s.get("items")) {
                (Some(items), _) => format!(
                    "[{}]",
                    items.iter().map(|i| ty(i, indent)).collect::<Vec<_>>().join(", ")
                ),
                (None, Some(items)) => format!("Array<{}>", ty(items, indent)),
                _ => "unknown[]".into(),
            },
            "object" => object(s, indent),
            _ => "unknown".into(),
        },
        _ => "unknown".into(),
    }
}

fn object(s: &Value, indent: &str) -> String {
    let props = s.get("properties").and_then(Value::as_object);
    match props {
        Some(props) if !props.is_empty() => {
            let inner = format!("{indent}  ");
            let required: Vec<&str> = s
                .get("required")
                .and_then(Value::as_array)
                .map(|r| r.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            let mut out = String::from("{\n");
            for (k, v) in props {
                doc(&mut out, &inner, v.get("description").and_then(Value::as_str));
                let opt = if required.contains(&k.as_str()) { "" } else { "?" };
                let _ = writeln!(out, "{inner}{k}{opt}: {};", ty(v, &inner));
            }
            out.push_str(indent);
            out.push('}');
            out
        }
        _ => match s.get("additionalProperties") {
            Some(v @ Value::Object(_)) => format!("Record<string, {}>", ty(v, indent)),
            _ => "Record<string, never>".into(),
        },
    }
}

const ENGINE_API: &str = r#"
/** A point or direction in meters (2D uses x and y). */
type Vec3 = { x: number; y: number; z: number };
/** What `engine.raycast` hit. */
interface RayHit {
  entity: Entity;
  /** Meters along the ray. */
  distance: number;
  point: Vec3;
  normal: Vec3;
}
/** The entity a hook runs for, or one found with `engine.entity` / `engine.query`. */
interface Entity {
  /** Stable scene id. */
  readonly id: number;
  /** Scene name (`entity_create` name, or the name given to `engine.spawn`), or null. */
  readonly name: string | null;
  /** Per-entity script state. Saved after every turn (component `ScriptState`), so it survives
   *  snapshots and is part of the state hash. Plain JSON values only. */
  state: Record<string, any>;
  /** A copy of the component, or `undefined` if the entity does not have it. */
  get<K extends keyof Components>(name: K): Components[K] | undefined;
  /** Adds or replaces the component. Throws `script.bad_value` if the value does not fit. */
  set<K extends keyof Components>(name: K, value: Components[K]): void;
  has(name: keyof Components): boolean;
  /** Plays a clip of this entity's animated `Model`, cross-fading other clips out over
   *  `fade` seconds (default 0.2). Changes the `Animator` component, like `anim_play`.
   *  Throws `anim.unknown_clip` for a clip the model does not have. */
  playClip(clip: string, options?: { fade?: number; speed?: number; looping?: boolean }): void;
  /** Sets one clip's blend weight (e.g. walk 0.7 + run 0.3) without touching other clips. */
  blendClip(clip: string, weight: number, options?: { fade?: number; speed?: number; looping?: boolean }): void;
  /** Fades out one clip, or every clip when `clip` is omitted. */
  stopClip(clip?: string, options?: { fade?: number }): void;
  /** Same as `engine.destroy(this, seconds)`. */
  destroy(seconds?: number): void;
}

/** A message from `engine.send`. `from` is the sender's id (it may be gone by now). */
interface Message {
  from: number;
  kind: string;
  detail: any;
}
/** What a script module exports. Only `update` is required. Hooks for one entity run in
 *  this order each tick: `start` (first tick only), `onCollisionEnter`, `onCollisionExit`,
 *  `onTriggerEnter`, `onTriggerExit`, `onMessage` (once per waiting message, in the order
 *  sent), due timers, `update`. Entities run in id order.
 *  After every entity's turn, entities marked by `engine.destroy` get `onDestroy` (id
 *  order) and are removed. */
interface ScriptHooks {
  /** Once, on the first tick this entity runs (Unity `Start`, Unreal `BeginPlay`). */
  start?(self: Entity): void;
  /** Every fixed tick (Unity `Update` and `FixedUpdate`, Unreal `Tick`). */
  update(self: Entity): void;
  /** A physics contact with `other` began on the previous tick. */
  onCollisionEnter?(self: Entity, other: Entity): void;
  /** A physics contact with `other` ended on the previous tick. */
  onCollisionExit?(self: Entity, other: Entity): void;
  /** Like `onCollisionEnter`, for contacts where either collider is a sensor (`Collider.sensor`). */
  onTriggerEnter?(self: Entity, other: Entity): void;
  onTriggerExit?(self: Entity, other: Entity): void;
  /** A message sent with `engine.send` to this entity. */
  onMessage?(self: Entity, message: Message): void;
  /** Just before the entity is removed by `engine.destroy` (Unity `OnDestroy`, Unreal
   *  `EndPlay`). It may change, spawn or destroy other entities. */
  onDestroy?(self: Entity): void;
}

/** A timer callback: an exported function of the same script. `arg` is the JSON value given
 *  when the timer was set (null if none). */
type TimerCallback = ((self: Entity, arg: any) => void) | string;

declare const engine: {
  /** Ticks simulated so far. */
  readonly tick: number;
  /** Seconds per tick (fixed). */
  readonly dt: number;
  /** Seeded random number in [0, 1) from the simulation's RNG (Math.random does not exist). */
  rand(): number;
  /** Value of an input action this tick (buttons 0 or 1, axes -1..1). */
  action(name: string): number;
  pressed(name: string): boolean;
  /** Pressed this tick and not the tick before. */
  justPressed(name: string): boolean;
  /** Records an event (`observe_events`, test assertions) for the current entity. */
  emit(kind: string, detail?: unknown): void;
  /** Records a `script_log` event. `console.log` does the same. */
  log(...parts: unknown[]): void;
  entity(id: number): Entity | null;
  /** The entity with this scene name (lowest id if several), or null. */
  find(name: string): Entity | null;
  /** Every entity with the component, in id order. */
  query(name: keyof Components): Entity[];
  resource<K extends keyof Resources>(name: K): Resources[K] | undefined;
  /** First solid collider hit by a ray from `origin` along `direction` (any length) within
 *  `maxDistance` meters (default 1000), skipping sensors and `exclude` (usually `self`).
 *  Sees the physics world as the last physics step left it. Null without a hit or without
 *  physics. 2D ignores z. */
  raycast(origin: Vec3, direction: Vec3, maxDistance?: number, exclude?: Entity | number): RayHit | null;
  /** Adds an entity now, with a new id that is never reused (Unity `Instantiate`, Unreal
   *  `SpawnActor`). Throws `script.bad_value` and spawns nothing if a component does not
   *  fit. Its script's `start` and `update` first run on the next tick; physics gives it a
   *  body in this tick's physics step. */
  spawn(components: Partial<Components>, name?: string): Entity;
  /** Spawns a copy of `source` (every component except ScriptState, ScriptTimers and
   *  DestroyAt), with `overrides` replacing whole components. The name defaults to the
   *  source's name. A copy's script starts fresh. */
  clone(source: Entity | number, overrides?: Partial<Components>, name?: string): Entity;
  /** Removes the entity at the end of this tick's script stage, after its `onDestroy`
   *  (Unity `Destroy(gameObject, seconds)`). With `seconds`, at the end of the script stage
   *  that many seconds later. Recorded as component `DestroyAt`, so it survives snapshots;
   *  the earliest removal wins. Emits a `destroyed` event. */
  destroy(entity: Entity | number, seconds?: number): void;
  /** Calls `fn(self, arg)` once, `seconds` from now, rounded to whole ticks (at least the
   *  next tick); Unity `Invoke`, a coroutine's `WaitForSeconds`, Unreal `SetTimer`. Timers
   *  belong to the entity whose hook is running, fire before its `update`, are saved as the
   *  `ScriptTimers` component (snapshots and the state hash include them) and are dropped
   *  when it is destroyed. Returns a handle for `cancel`. */
  after(seconds: number, fn: TimerCallback, arg?: unknown): number;
  /** Like `after`, counted in ticks (a whole number >= 1). */
  afterTicks(ticks: number, fn: TimerCallback, arg?: unknown): number;
  /** Calls `fn(self, arg)` every `seconds`, first after `firstSeconds` (default: `seconds`);
   *  Unity `InvokeRepeating`, Unreal looping `SetTimer`. Returns a handle for `cancel`. */
  every(seconds: number, fn: TimerCallback, arg?: unknown, firstSeconds?: number): number;
  /** Cancels this entity's timers: one handle, every timer calling `fn`, or all (no
   *  argument). Returns how many were cancelled. */
  cancel(which?: number | TimerCallback): number;
  /** How many of this entity's timers are pending (all, one handle, or those calling `fn`);
   *  Unity `IsInvoking`. */
  pending(which?: number | TimerCallback): number;
  /** Sends a message to another scripted entity (or this one). It is kept in the target's
   *  `ScriptInbox` component and delivered to its `onMessage` at the start of the target's
   *  next turn: this tick if the target's id is higher than the sender's, otherwise next
   *  tick. `detail` must be plain JSON. Throws if the target has no `Script`. */
  send(target: Entity | number, kind: string, detail?: unknown): void;
  /** Job TypeScript (`*.job.ts`, compiled, parallel) and job arrays (`JobData`). */
  readonly jobs: Jobs;
};

/** The typed arrays a job function takes. */
type JobArray = Float32Array | Float64Array | Int32Array;
/** An array argument made by `engine.jobs.read` or `engine.jobs.whole`. */
interface JobArgRef { readonly mode: "write" | "read" | "whole" }
/** One argument of a job function, in parameter order: a number (for `number` or `i32`),
 *  a boolean, a job array name of this entity (chunked, changes kept), a typed array of the
 *  script (chunked, changed in place), or a `read`/`whole`/`chunkStart` argument. */
type JobArg = number | boolean | string | JobArray | JobArgRef;
interface Jobs {
  /** Runs an exported function of a job file (path relative to the scene) in parallel
   *  chunks of `chunk` elements (default 16384) over the chunked arrays, which must have
   *  equal lengths (no chunked array: one call). Job arrays are used in place. Returns one
   *  value per chunk in chunk order (empty for `void`). Throws `jobts.*` errors. */
  run(path: string, fn: string, args?: JobArg[], options?: { chunk?: number }): number[];
  /** `run`, then the per-chunk values added in chunk order (deterministic). */
  sum(path: string, fn: string, args?: JobArg[], options?: { chunk?: number }): number;
  /** A copy of a job array of `entity` (default: the entity whose hook runs), or null. */
  get(name: string, entity?: Entity | number): JobArray | null;
  /** Creates or replaces a job array (its type follows the typed array). Saved as state. */
  set(name: string, values: JobArray, entity?: Entity | number): void;
  /** A chunked, read-only argument (a job array name, or a typed array). */
  read(array: string | JobArray, entity?: Entity | number): JobArgRef;
  /** The whole array to every chunk, read only (lookup tables, neighbours). */
  whole(array: string | JobArray, entity?: Entity | number): JobArgRef;
  /** A job array of another entity, chunked, changes kept. */
  write(array: string | JobArray, entity?: Entity | number): JobArgRef;
  /** An `i32` argument receiving the chunk's first element index. */
  readonly chunkStart: JobArgRef;
}

declare const console: { log(...parts: unknown[]): void; warn(...parts: unknown[]): void; error(...parts: unknown[]): void };
"#;

/// The full `marionette.d.ts` text for `registry`.
pub fn typings(registry: &Registry) -> String {
    let doc_v = marionette_core::schema::schema(registry);
    let mut out = String::from(
        "// GENERATED by `marionette docs` (operation `script_types`) from the engine schema. Do not edit.\n\
         // Marionette game scripts are TypeScript ES modules: export the hooks of `ScriptHooks`.\n\
         // Determinism: no Date, no Math.random (use engine.rand), Math functions are platform-independent.\n",
    );
    out.push_str(ENGINE_API);
    let defs = doc_v["$defs"].as_object().cloned().unwrap_or_default();
    let mut comps = Vec::new();
    let mut res = Vec::new();
    for (name, s) in &defs {
        let kind = s.get("x-marionette-kind").and_then(Value::as_str);
        match kind {
            Some("component") => comps.push(name.clone()),
            Some("resource") => res.push(name.clone()),
            _ => continue,
        }
        out.push('\n');
        doc(&mut out, "", s.get("description").and_then(Value::as_str));
        let _ = writeln!(out, "type {name} = {};", ty(s, ""));
    }
    out.push_str("\n/** Every component, by the name used in scene files. */\ninterface Components {\n");
    for c in &comps {
        let _ = writeln!(out, "  {c}: {c};");
    }
    out.push_str("}\n\n/** Every resource, by the name used in scene files. */\ninterface Resources {\n");
    for r in &res {
        let _ = writeln!(out, "  {r}: {r};");
    }
    out.push_str("}\n");
    out
}
