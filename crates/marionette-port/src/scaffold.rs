//! TypeScript script skeletons for ported classes (decision 0020).
//!
//! One `.ts` file per gameplay class, written for the script API of decision 0022: exported
//! `start`, `update`, `onCollisionEnter`, `onCollisionExit`, `onTriggerEnter`, `onTriggerExit`
//! and `onDestroy` hooks taking `self: Entity`.
//! Fields become `self.state` entries with their defaults. Every original method body is
//! kept as comments inside the hook it belongs to, and each line that uses an engine API is
//! followed by a `->` line naming the Marionette equivalent. The skeleton compiles as is; an
//! agent then replaces the comments with code, using `docs/ai/PORTING.md`.

use crate::map::{Role, Status, table};
use crate::{Analysis, Class, Occurrence};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

pub fn script_name(c: &Class) -> String {
    format!("{}.ts", c.raw.name)
}

/// Hooks in the order they are written, with the source methods that feed each, in the
/// order their bodies are placed.
const ORDER: [(&str, &[&str]); 7] = [
    (
        "start",
        &[
            "Awake",
            "OnEnable",
            "Start",
            "PostInitializeComponents",
            "BeginPlay",
            "ReceiveBeginPlay",
            "NativeConstruct",
        ],
    ),
    (
        "update",
        &[
            "FixedUpdate",
            "Update",
            "LateUpdate",
            "Tick",
            "TickComponent",
            "ReceiveTick",
            "NativeTick",
        ],
    ),
    ("onCollisionEnter", &[]),
    ("onCollisionExit", &[]),
    ("onTriggerEnter", &[]),
    ("onTriggerExit", &[]),
    ("onDestroy", &[]),
];

fn default_for(ts: &str, kind: &str) -> String {
    if kind == "entity" || kind == "unknown" {
        return "null".into();
    }
    match ts {
        "number" => "0".into(),
        "boolean" => "false".into(),
        "string" => "\"\"".into(),
        "{ x: number, y: number, z: number }" => "{ x: 0, y: 0, z: 0 }".into(),
        "{ x: number, y: number }" => "{ x: 0, y: 0 }".into(),
        "{ x: number, y: number, z: number, w: number }" => "{ x: 0, y: 0, z: 0, w: 1 }".into(),
        "{ r: number, g: number, b: number, a: number }" => "{ r: 1, g: 1, b: 1, a: 1 }".into(),
        t if t.ends_with("[]") => "[]".into(),
        t if t.starts_with("Record<") => "{}".into(),
        _ if kind == "entity" => "null".into(),
        _ => "null".into(),
    }
}

fn js_value(v: &serde_json::Value) -> String {
    // JSON is valid TypeScript for literals; drop quotes on simple object keys.
    match v {
        serde_json::Value::Object(m) => {
            let parts: Vec<String> = m.iter().map(|(k, v)| format!("{k}: {}", js_value(v))).collect();
            format!("{{ {} }}", parts.join(", "))
        }
        other => other.to_string(),
    }
}

fn annotate(out: &mut String, indent: &str, occ: &[&Occurrence]) {
    let mut seen = BTreeSet::new();
    for o in occ {
        if !seen.insert(&o.name) {
            continue;
        }
        let mut target = match (o.marionette.is_empty(), o.status) {
            (true, Status::Ready) => "not needed".to_string(),
            (true, _) => "(no equivalent yet)".to_string(),
            _ => o.marionette.clone(),
        };
        if let Some(a) = &o.arg0 {
            target = target.replace("<action>", a);
        }
        if let Some(g) = &o.generic
            && let Some(e) = table(o.engine).component(g)
        {
            let name = e.marionette.split_whitespace().next().unwrap_or(g);
            if !name.is_empty() {
                target = target.replace("<Component>", name);
            }
        }
        let arg = match (&o.generic, &o.arg0) {
            (Some(g), _) => format!("<{g}>"),
            (None, Some(a)) => format!(" \"{a}\""),
            _ => String::new(),
        };
        let _ = write!(
            out,
            "{indent}//     -> {}{arg}: {target} [{}]",
            o.name,
            o.status.as_str()
        );
        if !o.note.is_empty() && !matches!(o.status, Status::Script | Status::Ready) {
            let _ = write!(out, " {}", o.note);
        }
        out.push('\n');
    }
}

/// Writes the method's original body as comments with annotations.
fn body(
    out: &mut String,
    a: &Analysis,
    ci: usize,
    mi: usize,
    by_line: &BTreeMap<(usize, u32), Vec<&Occurrence>>,
) {
    let c = &a.classes[ci];
    let m = &c.raw.methods[mi];
    let f = &a.files[m.file];
    let _ = writeln!(out, "  // --- {} ({}:{}) ---", m.name, f.path, m.line);
    if c.role == Role::Blueprint {
        out.push_str("  //   (Blueprint event: rebuild what its node chain does from the graph)\n");
        return;
    }
    if m.body.is_none() {
        out.push_str("  //   (no body found in the scanned files)\n");
        return;
    }
    let first = m.line as usize;
    let last = (m.end_line as usize).min(f.lines.len());
    // Common indentation of the body lines.
    let lines: Vec<&str> = (first..=last)
        .filter_map(|n| f.lines.get(n - 1).map(String::as_str))
        .collect();
    let strip = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    for (k, l) in lines.iter().enumerate() {
        let n = (first + k) as u32;
        let text = if l.len() >= strip {
            &l[strip..]
        } else {
            l.trim_start()
        };
        let _ = writeln!(out, "  //   {}", text.trim_end());
        if let Some(occ) = by_line.get(&(m.file, n)) {
            annotate(out, "  ", occ);
        }
    }
}

/// Builds the skeleton for class `ci`.
pub fn skeleton(a: &Analysis, ci: usize, path_hint: &str) -> String {
    let c = &a.classes[ci];
    let t = table(c.raw.engine);
    let report = a.class_report(ci);
    let mut by_line: BTreeMap<(usize, u32), Vec<&Occurrence>> = BTreeMap::new();
    for o in &a.occurrences {
        by_line.entry((o.file, o.line)).or_default().push(o);
    }
    let mut out = String::new();
    let lang = if c.role == Role::Blueprint {
        "Blueprint (graph text)"
    } else {
        t.language.as_str()
    };
    let src = &a.files[c.raw.file].path;
    let base = c.raw.base.as_deref().unwrap_or("-");
    let _ = writeln!(
        out,
        "// Ported from {} {lang}: {src}, class {} : {base}.",
        c.raw.engine.as_str(),
        c.raw.name
    );
    out.push_str(
        "// Skeleton written by `marionette op port_scaffold` (docs/ai/PORTING.md). The original code is\n",
    );
    out.push_str(
        "// kept as comments in the hook it belongs to; `->` lines name the Marionette equivalent and\n",
    );
    out.push_str(
        "// its status (ready, script, rewrite, gap, hazard). Replace the comments with TypeScript.\n//\n",
    );
    let _ = writeln!(
        out,
        "// Scene: give the entity  \"Script\": {{ \"path\": \"{path_hint}\" }}  (path relative to the scene file)"
    );
    if !report.components.is_empty() {
        out.push_str("// and these components (set their values from the source scene or prefab):\n");
        for n in &report.components {
            let target = if n.marionette.is_empty() {
                "(no equivalent yet)"
            } else {
                &n.marionette
            };
            let _ = writeln!(out, "//   {} -> {target} [{}]", n.source, n.status.as_str());
        }
    }
    let status_counts: Vec<String> = report.uses.iter().map(|(k, v)| format!("{v} {k}")).collect();
    if !status_counts.is_empty() {
        let _ = writeln!(
            out,
            "// Engine API uses in this class: {}.",
            status_counts.join(", ")
        );
    }
    out.push('\n');

    // Which methods go where.
    let mut placed: BTreeSet<usize> = BTreeSet::new();
    let mut hooks: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (hook, order) in ORDER {
        let mut list: Vec<usize> = Vec::new();
        for name in order {
            if let Some(mi) = c.raw.methods.iter().position(|m| m.name == *name) {
                list.push(mi);
            }
        }
        for (mi, m) in c.raw.methods.iter().enumerate() {
            if list.contains(&mi) {
                continue;
            }
            let (h, _, _, _) = a.hook_for(c, &m.name);
            if h.as_deref() == Some(hook) {
                list.push(mi);
            }
        }
        placed.extend(list.iter().copied());
        hooks.insert(hook, list);
    }

    // start: field defaults, then bodies.
    let fields: Vec<_> = c
        .raw
        .fields
        .iter()
        .filter(|f| !f.modifiers.iter().any(|m| m == "const" || m == "static"))
        .map(|f| a.field_report(c, f))
        .collect();
    let consts: Vec<_> = c
        .raw
        .fields
        .iter()
        .filter(|f| f.modifiers.iter().any(|m| m == "const" || m == "static"))
        .collect();
    for f in &consts {
        let fr = a.field_report(c, f);
        let v = fr
            .default
            .as_ref()
            .map(js_value)
            .unwrap_or_else(|| default_for(&fr.ts_type, &fr.kind));
        let _ = writeln!(
            out,
            "const {} = {v}; // {} {}{}",
            f.name,
            f.ty,
            f.name,
            f.init.as_ref().map(|i| format!(" = {i}")).unwrap_or_default()
        );
    }
    if !consts.is_empty() {
        out.push('\n');
    }

    for (hook, _) in ORDER {
        let list = &hooks[hook];
        let needed = hook == "update" || !list.is_empty() || (hook == "start" && !fields.is_empty());
        if !needed {
            continue;
        }
        let sig = if hook.starts_with("on") && hook != "onDestroy" {
            "self: Entity, other: Entity"
        } else {
            "self: Entity"
        };
        let _ = writeln!(out, "export function {hook}({sig}): void {{");
        if hook == "start" && !fields.is_empty() {
            out.push_str("  // Fields become self.state entries (persisted and part of the state hash).\n");
            for f in &fields {
                if f.kind == "component" {
                    let _ = writeln!(out, "  // {}: {} -> {}", f.name, f.source_type, f.note);
                    continue;
                }
                let v = f
                    .default
                    .as_ref()
                    .map(js_value)
                    .unwrap_or_else(|| default_for(&f.ts_type, &f.kind));
                let was = f
                    .default_source
                    .as_ref()
                    .map(|s| format!(", was = {s}"))
                    .unwrap_or_default();
                let note = if f.note.is_empty() {
                    String::new()
                } else {
                    format!(" {}", f.note)
                };
                let _ = writeln!(
                    out,
                    "  self.state.{0} = self.state.{0} ?? {v}; // {1}: {2}{was}{note}",
                    f.name, f.source_type, f.ts_type
                );
            }
        }
        if hook == "update" {
            for (name, (pattern, h)) in &c.bound {
                if h.is_empty() && !crate::is_timer_binder(pattern) {
                    let _ = writeln!(
                        out,
                        "  // {name}(self) was bound with {pattern}; call it here when its action or due tick fires."
                    );
                }
            }
        }
        for &mi in list {
            let (_, status, note, _) = a.hook_for(c, &c.raw.methods[mi].name);
            if !note.is_empty() && status != Status::Script {
                let _ = writeln!(out, "  // [{}] {}", status.as_str(), note);
            }
            body(&mut out, a, ci, mi, &by_line);
        }
        if c.role == Role::Blueprint && hook == "update" {
            out.push_str(
                "  // Blueprint nodes found in the graph (their order is in the graph, not here):\n",
            );
            let occ: Vec<&Occurrence> = a.occurrences.iter().filter(|o| o.file == c.raw.file).collect();
            annotate(&mut out, "  ", &occ);
        }
        out.push_str("}\n\n");
    }

    // Everything else: helper functions, coroutines, gap hooks.
    for (mi, m) in c.raw.methods.iter().enumerate() {
        if placed.contains(&mi) || m.name.starts_with('~') || m.name == c.raw.name {
            if m.name == c.raw.name && m.body.is_some() {
                out.push_str(
                    "// The constructor's defaults belong in the scene file (components) or in start.\n",
                );
                out.push_str("function constructorDefaults(self: Entity): void {\n");
                body(&mut out, a, ci, mi, &by_line);
                out.push_str("}\n\n");
            }
            continue;
        }
        let (_, status, note, bound) = a.hook_for(c, &m.name);
        if let Some(h) = t.hook(&m.name) {
            let _ = writeln!(
                out,
                "// [{}] {}: {}",
                h.status.as_str(),
                m.name,
                if h.note.is_empty() { &h.marionette } else { &h.note }
            );
        } else if m.ret == "IEnumerator" || m.ret.starts_with("IEnumerator<") {
            out.push_str(
                "// [rewrite] Coroutine: split it at each `yield return` into exported steps; each step ends with\n\
                 // engine.after(seconds, nextStep, arg) (WaitForSeconds) and shared locals go in self.state or arg.\n",
            );
        } else if bound.is_some() {
            let _ = writeln!(out, "// [{}] {note}", status.as_str());
        }
        let params = if m.params.is_empty() {
            String::new()
        } else {
            format!(" /* , {} */", m.params.replace("*/", "* /"))
        };
        let ident: String = m
            .name
            .chars()
            .map(|ch| {
                if ch.is_alphanumeric() || ch == '_' {
                    ch
                } else {
                    '_'
                }
            })
            .collect();
        // Timer callbacks must be exported: engine.after/every call exported functions by name.
        let export =
            if m.ret.starts_with("IEnumerator") || bound.as_deref().is_some_and(crate::is_timer_binder) {
                "export "
            } else {
                ""
            };
        let _ = writeln!(out, "{export}function {ident}(self: Entity{params}): void {{");
        body(&mut out, a, ci, mi, &by_line);
        out.push_str("}\n\n");
    }
    while out.ends_with("\n\n") {
        out.pop();
    }
    out
}

/// Port notes for the whole scan, written next to the skeletons.
pub fn notes(a: &Analysis, scripts: &[(usize, String)]) -> String {
    let r = a.report(0, 500, 3);
    let mut out = String::new();
    out.push_str(
        "# Port notes\n\nWritten by `marionette op port_scaffold`. Follow `docs/ai/PORTING.md`.\n\n",
    );
    let _ = writeln!(
        out,
        "Scanned {} files, {} classes, {} become scripts. Engine API uses: {}.\n",
        r.summary.files,
        r.summary.classes,
        r.summary.scripts,
        r.summary
            .uses
            .iter()
            .map(|(k, v)| format!("{v} {k}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    out.push_str(
        "## Scripts\n\n| Class | Source | Script | Components to add |\n| --- | --- | --- | --- |\n",
    );
    for (ci, path) in scripts {
        let cr = a.class_report(*ci);
        let comps: Vec<String> = cr
            .components
            .iter()
            .map(|n| {
                format!(
                    "{} -> {}",
                    n.source,
                    if n.marionette.is_empty() {
                        "gap"
                    } else {
                        &n.marionette
                    }
                )
            })
            .collect();
        let _ = writeln!(
            out,
            "| `{}` | `{}:{}` | `{path}` | {} |",
            cr.name,
            cr.file,
            cr.line,
            comps.join("; ").replace('|', "\\|")
        );
    }
    let jobs: Vec<&Class> = a.classes.iter().filter(|c| c.role == Role::Job).collect();
    if !jobs.is_empty() {
        out.push_str(
            "\n## Jobs (Rust, not scripts)\n\nSkeletons in `jobs.rs`; follow PORTING.md \"Burst jobs\".\n\n",
        );
        for c in jobs {
            let _ = writeln!(
                out,
                "- `{}` : {} (`{}:{}`)",
                c.raw.name,
                c.raw.base.as_deref().unwrap_or("-"),
                a.files[c.raw.file].path,
                c.raw.line
            );
        }
    }
    if !r.summary.actions.is_empty() {
        out.push_str("\n## Input actions to define\n\n");
        for act in &r.summary.actions {
            let _ = writeln!(out, "- `{act}`");
        }
    }
    for (title, status) in [
        ("Hazards (fix while porting)", Status::Hazard),
        ("Gaps (work around, then list what is left)", Status::Gap),
        ("Unknown engine calls", Status::Unknown),
    ] {
        let list: Vec<_> = r.api.iter().filter(|u| u.status == status).collect();
        if list.is_empty() {
            continue;
        }
        let _ = writeln!(out, "\n## {title}\n");
        for u in list {
            let at: Vec<String> = u
                .locations
                .iter()
                .map(|l| format!("{}:{}", l.file, l.line))
                .collect();
            let fix = if u.note.is_empty() {
                u.marionette.clone()
            } else {
                u.note.clone()
            };
            let _ = writeln!(out, "- `{}` x{} ({}): {}", u.name, u.count, at.join(", "), fix);
        }
    }
    out.push_str("\n## Coordinates\n\nPositions, rotations and sizes copied from the source scene must be converted: `marionette op port_convert '{\"engine\": \"...\", \"kind\": \"position\", \"value\": [x, y, z]}'`.\n");
    out
}

/// The Rust type for a C# field type in a job or a struct a job uses.
pub fn rust_type(ty: &str, enums: &BTreeSet<String>) -> String {
    let s = ty.trim();
    for (open, wrap) in [
        ("NativeArray<", "Vec"),
        ("NativeList<", "Vec"),
        ("NativeSlice<", "Vec"),
        ("NativeQueue<", "std::collections::VecDeque"),
    ] {
        if let Some(inner) = s.strip_prefix(open) {
            let inner = inner.strip_suffix('>').unwrap_or(inner);
            return format!("{wrap}<{}>", rust_type(inner, enums));
        }
    }
    if let Some(inner) = s.strip_prefix("NativeReference<") {
        return rust_type(inner.strip_suffix('>').unwrap_or(inner), enums);
    }
    if let Some(inner) = s.strip_suffix("[]") {
        return format!("Vec<{}>", rust_type(inner, enums));
    }
    match s {
        "int" => "i32".into(),
        "uint" => "u32".into(),
        "long" => "i64".into(),
        "ulong" => "u64".into(),
        "short" => "i16".into(),
        "ushort" => "u16".into(),
        "byte" => "u8".into(),
        "sbyte" => "i8".into(),
        "float" => "f32".into(),
        "double" => "f64".into(),
        "bool" => "bool".into(),
        "float2" | "Vector2" => "[f32; 2]".into(),
        "float3" | "Vector3" => "[f32; 3]".into(),
        "float4" | "Vector4" | "quaternion" | "Quaternion" => "[f32; 4]".into(),
        "int2" => "[i32; 2]".into(),
        "int3" => "[i32; 3]".into(),
        "int4" => "[i32; 4]".into(),
        "float4x4" | "Matrix4x4" => "[[f32; 4]; 4]".into(),
        e if enums.contains(e) => format!("i32 /* enum {e} */"),
        other => other.to_string(),
    }
}

/// Rust skeletons for every Unity job struct (`IJob`, `IJobFor`, ...) and the scanned
/// structs their fields use, or `None` when the source has no jobs. Fields keep their
/// names; each method's original code is kept as comments, with `->` lines as in the
/// TypeScript skeletons. `execute` is a real (empty) method so the file is a starting point
/// for a system of the game's own Rust module (decision 0062).
pub fn rust_jobs(a: &Analysis) -> Option<String> {
    let jobs: Vec<usize> = (0..a.classes.len())
        .filter(|&ci| a.classes[ci].role == Role::Job)
        .collect();
    if jobs.is_empty() {
        return None;
    }
    let mut by_line: BTreeMap<(usize, u32), Vec<&Occurrence>> = BTreeMap::new();
    for o in &a.occurrences {
        by_line.entry((o.file, o.line)).or_default().push(o);
    }
    // Structs the jobs' fields name (one level is enough for flat job data).
    let mut wanted: Vec<usize> = Vec::new();
    for &ji in &jobs {
        for f in &a.classes[ji].raw.fields {
            for (si, c) in a.classes.iter().enumerate() {
                if c.raw.is_struct
                    && c.role != Role::Job
                    && f.ty.split(['<', '>', '[', ']', ' ']).any(|w| w == c.raw.name)
                    && !wanted.contains(&si)
                {
                    wanted.push(si);
                }
            }
        }
    }
    let mut out = String::new();
    out.push_str("//! Rust skeletons for the job structs of the scanned source, written by\n");
    out.push_str("//! `marionette op port_scaffold` (docs/ai/PORTING.md, \"Burst jobs\").\n//!\n");
    out.push_str(
        "//! Put the arrays in one resource of the game's own Rust module (HOWTO.md \"Add a game's\n",
    );
    out.push_str(
        "//! own Rust module\") and call each job's `execute` from a `Stage::Logic` system, in the\n",
    );
    out.push_str("//! order the source scheduled them. The original code is kept as comments; `->` lines\n");
    out.push_str("//! name the equivalent. The file compiles once every type it names exists.\n\n");
    out.push_str("#![allow(dead_code, non_snake_case)]\n");
    for ci in wanted.iter().chain(jobs.iter()) {
        let c = &a.classes[*ci];
        let src = &a.files[c.raw.file].path;
        let _ = writeln!(
            out,
            "\n/// {} `{}` ({src}:{}).",
            if c.role == Role::Job { "Job" } else { "Struct" },
            c.raw.name,
            c.raw.line
        );
        let _ = writeln!(
            out,
            "#[derive(Debug, Clone, Default)]\npub struct {} {{",
            c.raw.name
        );
        for f in &c.raw.fields {
            if f.modifiers.iter().any(|m| m == "const" || m == "static") {
                let _ = writeln!(
                    out,
                    "    // {} {}: {} (constant: make it a `const`)",
                    f.ty,
                    f.name,
                    f.init.as_deref().unwrap_or("-")
                );
                continue;
            }
            let note = if f.attributes.iter().any(|x| x == "ReadOnly") {
                " // read-only in the job: borrow as &[T]"
            } else {
                ""
            };
            let _ = writeln!(out, "    pub {}: {},{note}", f.name, rust_type(&f.ty, &a.enums));
        }
        out.push_str("}\n");
        if c.raw.methods.is_empty() {
            continue;
        }
        let _ = writeln!(out, "\nimpl {} {{", c.raw.name);
        for (mi, m) in c.raw.methods.iter().enumerate() {
            let mut b = String::new();
            body(&mut b, a, *ci, mi, &by_line);
            if c.role == Role::Job && m.name == "Execute" {
                out.push_str("    /// The job body (`Execute`).\n    pub fn execute(&mut self) {\n");
                for l in b.lines() {
                    let _ = writeln!(out, "      {l}");
                }
                out.push_str("    }\n");
            } else {
                for l in b.lines() {
                    let _ = writeln!(out, "  {l}");
                }
            }
        }
        out.push_str("}\n");
    }
    Some(out)
}
