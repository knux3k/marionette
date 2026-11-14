//! Porting operations: scan Unity/Unreal gameplay code, look up concept mappings, write
//! TypeScript skeletons, convert coordinates. Guide: `docs/ai/PORTING.md`; design: decision
//! 0020.

use crate::{Agent, Op, op};
use marionette_core::{Diagnostic, Errors};
use marionette_port::convert::{ValueKind, convert};
use marionette_port::map::{Engine, Entry, Section, Status, table};
use marionette_port::{Analysis, PortReport, scaffold};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::{Component, Path};

pub(crate) fn ops() -> Vec<Op> {
    vec![
        op(
            "port_scan",
            "Scans Unity C# or Unreal C++ gameplay code (a folder or one file; Blueprint graphs copied as text in .t3d files too) and reports each class (role, fields with TypeScript types, lifecycle methods and the hook they go into, components to add), every engine API call with its Marionette equivalent and status (ready, script, rewrite, gap, hazard, unknown), input action names, gaps and determinism hazards. Read-only.",
            false,
            scan,
        ),
        op(
            "port_map",
            "Looks up the Unity/Unreal to Marionette concept tables (base classes, lifecycle hooks, components, types, API calls) by name, section or status.",
            false,
            map,
        ),
        op(
            "port_scaffold",
            "Writes one TypeScript script skeleton per Unity/Unreal gameplay class (exported start/update/onCollisionEnter/onCollisionExit/onTriggerEnter/onTriggerExit hooks, fields as self.state defaults, the original code as comments annotated with Marionette equivalents) plus PORT-NOTES.md. Never overwrites unless `overwrite` is set.",
            true,
            write_scaffold,
        ),
        op(
            "port_serde",
            "Writes Rust serde types for C# [Serializable] classes that read and write Unity JsonUtility JSON; reports what it cannot map. Code is returned without `out`.",
            true,
            write_serde,
        ),
        op(
            "port_convert",
            "Converts a position, direction, velocity, quaternion, Euler rotation, scale or length from Unity or Unreal coordinates and units to Marionette's (right-handed, +Y up, -Z forward, meters).",
            false,
            convert_value,
        ),
    ]
}

fn run_scan(agent: &Agent, source: &str) -> Result<Analysis, Errors> {
    marionette_port::scan(&agent.resolve(source)?)
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ScanInput {
    /// A Unity project or Assets folder, an Unreal project or Source folder, or one
    /// .cs/.h/.cpp/.t3d file. Relative to the project root, or absolute.
    source: String,
    /// First class to list (classes are paged; API uses are always complete).
    #[serde(default)]
    offset: usize,
    /// Classes per page, default 50, at most 500.
    #[serde(default)]
    limit: Option<usize>,
    /// Places listed per API use, default 5.
    #[serde(default)]
    max_locations: Option<usize>,
}

fn scan(agent: &mut Agent, input: ScanInput) -> Result<PortReport, Errors> {
    let a = run_scan(agent, &input.source)?;
    Ok(a.report(
        input.offset,
        input.limit.unwrap_or(50),
        input.max_locations.unwrap_or(5).min(100),
    ))
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct MapInput {
    /// unity or unreal; both when omitted.
    #[serde(default)]
    engine: Option<Engine>,
    /// Only this section: bases, hooks, components, types or api.
    #[serde(default)]
    section: Option<Section>,
    /// Case-insensitive text that the source name or the Marionette equivalent contains.
    #[serde(default)]
    query: Option<String>,
    /// Only entries with this status.
    #[serde(default)]
    status: Option<Status>,
    #[serde(default)]
    offset: usize,
    /// Default 50, at most 500.
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Clone, Serialize, JsonSchema)]
struct MapRow {
    engine: Engine,
    section: Section,
    #[serde(flatten)]
    entry: Entry,
}

#[derive(Serialize, JsonSchema)]
struct MapOutput {
    total: usize,
    entries: Vec<MapRow>,
    next: Option<usize>,
}

fn map(_: &mut Agent, input: MapInput) -> Result<MapOutput, Errors> {
    let q = input.query.as_deref().map(str::to_lowercase);
    let mut rows = Vec::new();
    for engine in [Engine::Unity, Engine::Unreal] {
        if input.engine.is_some_and(|e| e != engine) {
            continue;
        }
        let t = table(engine);
        for section in Section::ALL {
            if input.section.is_some_and(|s| s != section) {
                continue;
            }
            for e in t.section(section) {
                if input.status.is_some_and(|s| s != e.status) {
                    continue;
                }
                if let Some(q) = &q
                    && !e.name.to_lowercase().contains(q)
                    && !e.marionette.to_lowercase().contains(q)
                {
                    continue;
                }
                rows.push(MapRow {
                    engine,
                    section,
                    entry: e.clone(),
                });
            }
        }
    }
    let (entries, next) = crate::page(&rows, input.offset, input.limit);
    Ok(MapOutput {
        total: rows.len(),
        entries,
        next,
    })
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ScaffoldInput {
    /// Same as `port_scan`'s source.
    source: String,
    /// Output folder for the .ts files and PORT-NOTES.md, relative to the project root.
    /// Default `scripts/ported`.
    #[serde(default)]
    out: Option<String>,
    /// The scene file the scripts will be used from, so the suggested `Script.path` is
    /// relative to it. Default: paths relative to the project root.
    #[serde(default)]
    scene: Option<String>,
    /// Replace existing files.
    #[serde(default)]
    overwrite: bool,
    /// Report what would be written without writing.
    #[serde(default)]
    dry_run: bool,
}

#[derive(Serialize, JsonSchema)]
struct ScaffoldFile {
    /// Written file, relative to the project root.
    path: String,
    /// The class it was made from, or `-` for PORT-NOTES.md.
    class: String,
    /// The value for the entity's `Script.path`.
    #[serde(skip_serializing_if = "Option::is_none")]
    script_path: Option<String>,
    bytes: usize,
}

#[derive(Serialize, JsonSchema)]
struct ScaffoldOutput {
    written: bool,
    files: Vec<ScaffoldFile>,
}

/// `to` relative to the folder of `from` (both relative to the project root).
fn relative(from_file: &str, to: &str) -> String {
    let from: Vec<_> = Path::new(from_file)
        .parent()
        .map(|p| p.components().collect())
        .unwrap_or_default();
    let to_c: Vec<_> = Path::new(to).components().collect();
    let common = from.iter().zip(&to_c).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<String> = from[common..].iter().map(|_| "..".to_string()).collect();
    parts.extend(to_c[common..].iter().filter_map(|c| match c {
        Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
        _ => None,
    }));
    parts.join("/")
}

fn write_scaffold(agent: &mut Agent, input: ScaffoldInput) -> Result<ScaffoldOutput, Errors> {
    let a = run_scan(agent, &input.source)?;
    let out_rel = input.out.unwrap_or_else(|| "scripts/ported".to_string());
    let out_dir = agent.resolve(&out_rel)?;
    if let Some(scene) = &input.scene {
        agent.resolve(scene)?;
    }
    let mut used = std::collections::BTreeSet::new();
    let mut planned: Vec<(String, String, Option<String>, String)> = Vec::new();
    let mut scripts = Vec::new();
    for (ci, c) in a.classes.iter().enumerate() {
        if !Analysis::makes_script(c) {
            continue;
        }
        let mut name = scaffold::script_name(c);
        let mut n = 2;
        while !used.insert(name.clone()) {
            name = format!("{}_{n}.ts", c.raw.name);
            n += 1;
        }
        let rel = format!("{}/{name}", out_rel.trim_end_matches('/'));
        let hint = match &input.scene {
            Some(scene) => relative(scene, &rel),
            None => rel.clone(),
        };
        let text = scaffold::skeleton(&a, ci, &hint);
        scripts.push((ci, rel.clone()));
        planned.push((rel, c.raw.name.clone(), Some(hint), text));
    }
    if let Some(text) = scaffold::rust_jobs(&a) {
        let rel = format!("{}/jobs.rs", out_rel.trim_end_matches('/'));
        planned.push((rel, "(job structs)".into(), None, text));
    }
    let notes_rel = format!("{}/PORT-NOTES.md", out_rel.trim_end_matches('/'));
    planned.push((notes_rel, "-".into(), None, scaffold::notes(&a, &scripts)));
    if !input.overwrite {
        let existing: Vec<String> = planned
            .iter()
            .filter(|p| out_dir.parent().is_some() && agent.root().join(&p.0).exists())
            .map(|p| p.0.clone())
            .collect();
        if !existing.is_empty() {
            return Err(Diagnostic::error(
                marionette_port::codes::EXISTS,
                format!("would overwrite {}", existing.join(", ")),
            )
            .fix("pass `overwrite: true`, or a different `out` folder")
            .into());
        }
    }
    let mut files = Vec::new();
    for (rel, class, hint, text) in planned {
        if !input.dry_run {
            crate::write_atomic(&agent.resolve(&rel)?, &rel, text.as_bytes())?;
        }
        files.push(ScaffoldFile {
            path: rel,
            class,
            script_path: hint,
            bytes: text.len(),
        });
    }
    Ok(ScaffoldOutput {
        written: !input.dry_run,
        files,
    })
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SerdeInput {
    /// Same as `port_scan`'s source.
    source: String,
    /// The .rs file to write, relative to the project root. Omit to get the code back.
    #[serde(default)]
    out: Option<String>,
    /// Only these types and the types their saved fields reach. Default: every
    /// [Serializable] type.
    #[serde(default)]
    roots: Vec<String>,
    /// Extra derives for every type (for example `Reflect`).
    #[serde(default)]
    derives: Vec<String>,
    /// C# type name -> Rust path of a type the project already has (hand-written, or its own
    /// vector type). Used as is, never generated.
    #[serde(default)]
    external: std::collections::BTreeMap<String, String>,
    /// Replace an existing `out` file.
    #[serde(default)]
    overwrite: bool,
    /// Return the code without writing.
    #[serde(default)]
    dry_run: bool,
}

#[derive(Serialize, JsonSchema)]
struct SerdeOutput {
    written: bool,
    /// The file written, relative to the project root.
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    bytes: usize,
    /// The generated module; only when nothing was written.
    #[serde(skip_serializing_if = "Option::is_none")]
    rust: Option<String>,
    types: Vec<marionette_port::serde_gen::TypeInfo>,
    notes: Vec<marionette_port::serde_gen::Note>,
}

fn write_serde(agent: &mut Agent, input: SerdeInput) -> Result<SerdeOutput, Errors> {
    let a = run_scan(agent, &input.source)?;
    let g = marionette_port::serde_gen::generate(
        &a,
        &marionette_port::serde_gen::Options {
            roots: input.roots,
            derives: input.derives,
            external: input.external,
        },
    );
    if g.types.is_empty() {
        return Err(Diagnostic::error(
            marionette_port::codes::NOTHING_FOUND,
            "no [Serializable] classes or structs found",
        )
        .fix("check `source`, or `roots` names")
        .into());
    }
    let bytes = g.rust.len();
    let write = input.out.is_some() && !input.dry_run;
    if let Some(out) = input.out.as_deref() {
        let path = agent.resolve(out)?;
        if write && !input.overwrite && path.exists() {
            return Err(
                Diagnostic::error(marionette_port::codes::EXISTS, format!("would overwrite {out}"))
                    .fix("pass `overwrite: true`, or a different `out`")
                    .into(),
            );
        }
        if write {
            crate::write_atomic(&path, out, g.rust.as_bytes())?;
        }
    }
    Ok(SerdeOutput {
        written: write,
        path: input.out.filter(|_| write),
        bytes,
        rust: (!write).then_some(g.rust),
        types: g.types,
        notes: g.notes,
    })
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ConvertInput {
    engine: Engine,
    kind: ValueKind,
    /// 3 numbers (position, direction, velocity, scale, euler in degrees: Unity x/y/z,
    /// Unreal pitch/yaw/roll), 4 for a quaternion (x, y, z, w), 1 for a length.
    value: Vec<f64>,
}

#[derive(Serialize, JsonSchema)]
struct ConvertOutput {
    /// The Marionette value: {x, y, z}, {x, y, z, w} or a single length.
    value: serde_json::Value,
    /// The rule applied.
    rule: String,
}

fn convert_value(_: &mut Agent, input: ConvertInput) -> Result<ConvertOutput, Errors> {
    let v = convert(input.engine, input.kind, &input.value)?;
    let value = match v.as_slice() {
        [l] => serde_json::json!(l),
        [x, y, z] => serde_json::json!({"x": x, "y": y, "z": z}),
        [x, y, z, w] => serde_json::json!({"x": x, "y": y, "z": z, "w": w}),
        _ => serde_json::Value::Null,
    };
    let rule = match (input.engine, input.kind) {
        (Engine::Unity, ValueKind::Position | ValueKind::Direction | ValueKind::Velocity) => {
            "(x, y, z) -> (x, y, -z)"
        }
        (Engine::Unity, ValueKind::Quaternion) => "(x, y, z, w) -> (-x, -y, z, w)",
        (Engine::Unity, ValueKind::Euler) => {
            "Unity Euler (Z, then X, then Y) to quaternion, then (x, y, z, w) -> (-x, -y, z, w)"
        }
        (Engine::Unity, _) => "unchanged",
        (Engine::Unreal, ValueKind::Position | ValueKind::Velocity) => "(x, y, z) cm -> (y, z, -x) / 100 m",
        (Engine::Unreal, ValueKind::Direction) => "(x, y, z) -> (y, z, -x)",
        (Engine::Unreal, ValueKind::Scale) => "(x, y, z) -> (y, z, x)",
        (Engine::Unreal, ValueKind::Length) => "cm / 100 -> m",
        (Engine::Unreal, ValueKind::Quaternion) => "(x, y, z, w) -> (-y, -z, x, w)",
        (Engine::Unreal, ValueKind::Euler) => {
            "FRotator (pitch, yaw, roll) to FQuat, then (x, y, z, w) -> (-y, -z, x, w)"
        }
    };
    Ok(ConvertOutput {
        value,
        rule: rule.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::relative;

    #[test]
    fn relative_paths() {
        assert_eq!(
            relative("scenes/main.scene.json", "scripts/ported/A.ts"),
            "../scripts/ported/A.ts"
        );
        assert_eq!(relative("main.scene.json", "scripts/A.ts"), "scripts/A.ts");
        assert_eq!(
            relative("scripts/s.scene.json", "scripts/ported/A.ts"),
            "ported/A.ts"
        );
    }
}
