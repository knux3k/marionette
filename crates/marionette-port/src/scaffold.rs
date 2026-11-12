//! TypeScript script skeletons for ported classes (decision 0020).
//!
//! One `.ts` file per gameplay class, written for the script API of decision 0022: exported
//! `start`, `update`, `onCollisionEnter`, `onCollisionExit`, `onTriggerEnter` and `onTriggerExit` hooks taking `self: Entity`.
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
const ORDER: [(&str, &[&str]); 6] = [
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
        let sig = if hook.starts_with("on") {
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
                if h.is_empty() {
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
        } else if m.ret == "IEnumerator" {
            out.push_str("// [rewrite] Coroutine: keep a step number and a due tick in self.state and advance them in update.\n");
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
        let _ = writeln!(out, "function {ident}(self: Entity{params}): void {{");
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
