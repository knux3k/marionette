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
/** The entity a hook runs for, or one found with `engine.entity` / `engine.query`. */
interface Entity {
  /** Stable scene id. */
  readonly id: number;
  /** Per-entity script state. Saved after every turn (component `ScriptState`), so it survives
   *  snapshots and is part of the state hash. Plain JSON values only. */
  state: Record<string, any>;
  /** A copy of the component, or `undefined` if the entity does not have it. */
  get<K extends keyof Components>(name: K): Components[K] | undefined;
  /** Adds or replaces the component. Throws `script.bad_value` if the value does not fit. */
  set<K extends keyof Components>(name: K, value: Components[K]): void;
  has(name: keyof Components): boolean;
}

/** What a script module exports. Only `update` is required. Hooks for one entity run in
 *  this order each tick: `start` (first tick only), `onCollisionEnter`, `onCollisionExit`,
 *  `onTriggerEnter`, `onTriggerExit`, `update`. Entities run in id order. */
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
}

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
};

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
