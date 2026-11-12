//! Project operations: init from a template, validate every file, schema, plus single-scene
//! validate / format / headless run.

use crate::{Agent, Op, codes, op, read_text, write_atomic};
use marionette_core::{Diagnostic, Errors, Kind, Sim, scene, schema};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;

pub(crate) fn ops() -> Vec<Op> {
    vec![
        op(
            "project_init",
            "Create a new project from the template: PROTOCOL.md (the owner's rules; read it first), \
             AGENTS.md, marionette.json, scenes/main.scene.json and tests/main.test.json.",
            true,
            init,
        ),
        op(
            "project_validate",
            "Validate every *.scene.json and *.test.json under the project. Read-only.",
            false,
            validate_all,
        ),
        op(
            "project_schema",
            "List component and resource types. Pass `types` for their JSON Schemas, or `full: true` \
             for the whole scene-file schema.",
            false,
            schema_op,
        ),
        op(
            "scene_create",
            "Create a new scene file (empty, or `basic` with camera, light and ground).",
            true,
            create,
        ),
        op(
            "scene_validate",
            "Validate one scene file. Also reports whether it is in canonical form.",
            false,
            validate_one,
        ),
        op(
            "scene_format",
            "Rewrite one scene file in canonical form.",
            true,
            format,
        ),
        op(
            "scene_run",
            "Load a scene, simulate `ticks` ticks headless with `seed`, return the state hash. Nothing is written.",
            false,
            run,
        ),
    ]
}

const MANIFEST: &str = "marionette.json";
const AGENTS_TEMPLATE: &str = include_str!("../templates/AGENTS.md");
const PROTOCOL_TEMPLATE: &str = include_str!("../../../docs/PROTOCOL.md");

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InitInput {
    /// Directory to create (relative to the current project root). Must not exist or be empty.
    pub path: String,
    /// Project name (default: the directory name).
    pub name: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct InitOutput {
    pub path: String,
    pub files: Vec<String>,
}

fn transform(x: f32, y: f32, z: f32, s: f32) -> Value {
    json!({
        "translation": { "x": x, "y": y, "z": z },
        "rotation": { "x": 0, "y": 0, "z": 0, "w": 1 },
        "scale": { "x": s, "y": s, "z": s }
    })
}

/// The `basic` scene: camera (1), sun (2), ground (3).
pub(crate) fn basic_scene() -> Value {
    json!({
        "format": scene::FORMAT,
        "version": scene::VERSION,
        "resources": {
            "FixedStep": { "dt": 1.0 / 60.0 },
            "AmbientLight": { "color": { "r": 0.6, "g": 0.7, "b": 1.0, "a": 1 }, "intensity": 0.35 }
        },
        "entities": [
            { "id": 1, "name": "camera", "components": {
                "Transform": transform(0.0, 3.0, 8.0, 1.0),
                "LookAt": { "target": { "x": 0, "y": 0.5, "z": 0 }, "up": { "x": 0, "y": 1, "z": 0 } },
                "Camera": {
                    "projection": { "Perspective": { "fov_y_degrees": 50, "near": 0.1, "far": 200 } },
                    "clear_color": { "r": 0.35, "g": 0.5, "b": 0.75, "a": 1 }
                }
            }},
            { "id": 2, "name": "sun", "components": {
                "DirectionalLight": {
                    "direction": { "x": -0.4, "y": -1, "z": -0.5 },
                    "color": { "r": 1, "g": 0.95, "b": 0.85, "a": 1 },
                    "illuminance": 1.1
                }
            }},
            { "id": 3, "name": "ground", "components": {
                "Transform": { "translation": { "x": 0, "y": 0, "z": 0 }, "rotation": { "x": 0, "y": 0, "z": 0, "w": 1 },
                               "scale": { "x": 20, "y": 1, "z": 20 } },
                "Model": { "asset": "primitive:plane" },
                "Material": { "base_color": { "r": 0.35, "g": 0.36, "b": 0.38, "a": 1 }, "metallic": 0, "roughness": 0.9 }
            }}
        ]
    })
}

fn canonical(agent: &Agent, doc: &Value) -> String {
    scene::format_text(&doc.to_string(), agent.registry()).expect("built-in templates are valid")
}

fn init(agent: &mut Agent, input: InitInput) -> Result<InitOutput, Errors> {
    let dir = agent.resolve(&input.path)?;
    if dir.exists()
        && std::fs::read_dir(&dir)
            .map(|mut d| d.next().is_some())
            .unwrap_or(true)
    {
        return Err(
            Diagnostic::error(codes::EXISTS, format!("`{}` exists and is not empty", input.path))
                .fix("choose a new directory name")
                .into(),
        );
    }
    let name = input.name.unwrap_or_else(|| {
        dir.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "game".into())
    });
    let mut main = basic_scene();
    main["entities"].as_array_mut().expect("array").push(json!({
        "id": 4, "name": "ball", "components": {
            "Transform": transform(0.0, 0.5, 0.0, 1.0),
            "Model": { "asset": "primitive:sphere" },
            "Material": { "base_color": { "r": 0.9, "g": 0.3, "b": 0.2, "a": 1 }, "metallic": 0, "roughness": 0.4 },
            "Position": { "x": -2.0, "y": 0.5, "z": 0.0 },
            "Velocity": { "x": 1.0, "y": 0.0, "z": 0.0 }
        }
    }));
    let test = json!({
        "scene": "scenes/main.scene.json",
        "seed": 1,
        "ticks": 60,
        "assertions": [
            { "count": { "with": ["Camera"], "equals": 1 } },
            { "field": { "entity": 4, "path": "Position.x", "op": "gt", "value": -1.5 } }
        ]
    });
    let manifest = json!({ "format": "marionette.project", "version": 1, "name": name });
    let files: Vec<(&str, String)> = vec![
        (
            MANIFEST,
            format!("{}\n", serde_json::to_string_pretty(&manifest).expect("json")),
        ),
        ("PROTOCOL.md", PROTOCOL_TEMPLATE.to_string()),
        ("AGENTS.md", AGENTS_TEMPLATE.to_string()),
        ("CLAUDE.md", AGENTS_TEMPLATE.to_string()),
        ("scenes/main.scene.json", canonical(agent, &main)),
        (
            "tests/main.test.json",
            format!("{}\n", serde_json::to_string_pretty(&test).expect("json")),
        ),
    ];
    for (rel, text) in &files {
        let shown = format!("{}/{rel}", input.path);
        write_atomic(&dir.join(rel), &shown, text.as_bytes())?;
    }
    Ok(InitOutput {
        path: input.path,
        files: files.iter().map(|(f, _)| f.to_string()).collect(),
    })
}

#[derive(Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct Empty {}

#[derive(Serialize, JsonSchema)]
pub struct FileReport {
    pub path: String,
    pub ok: bool,
    #[schemars(with = "Vec<Value>")]
    pub diagnostics: Errors,
}

#[derive(Serialize, JsonSchema)]
pub struct ValidateAllOutput {
    pub ok: bool,
    pub files: Vec<FileReport>,
}

/// All files under `dir` with the given suffix, sorted, skipping hidden and build directories.
pub(crate) fn find_files(dir: &Path, suffix: &str) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if p.is_dir() {
                if !name.starts_with('.') && name != "target" {
                    stack.push(p);
                }
            } else if name.ends_with(suffix) {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

pub(crate) fn rel(agent: &Agent, p: &Path) -> String {
    p.strip_prefix(agent.root())
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

fn validate_all(agent: &mut Agent, _: Empty) -> Result<ValidateAllOutput, Errors> {
    let root = agent.root().to_path_buf();
    let mut files = Vec::new();
    for p in find_files(&root, ".scene.json") {
        let shown = rel(agent, &p);
        let diagnostics = match check_scene(agent, &p, &shown) {
            Ok(_) => Errors::default(),
            Err(e) => e,
        };
        files.push(FileReport {
            ok: !diagnostics.has_errors(),
            path: shown,
            diagnostics,
        });
    }
    for p in find_files(&root, ".test.json") {
        let shown = rel(agent, &p);
        let diagnostics = match crate::scenario::load_file(agent, &p, &shown) {
            Ok(_) => Errors::default(),
            Err(e) => e,
        };
        files.push(FileReport {
            ok: !diagnostics.has_errors(),
            path: shown,
            diagnostics,
        });
    }
    Ok(ValidateAllOutput {
        ok: files.iter().all(|f| f.ok),
        files,
    })
}

/// Validates a scene file; returns (entity count, canonical?).
fn check_scene(agent: &Agent, path: &Path, shown: &str) -> Result<(usize, bool), Errors> {
    let text = read_text(path, shown)?;
    let formatted = scene::format_text(&text, agent.registry()).map_err(|e| e.with_file(shown))?;
    let entities = scene::parse(&formatted).map(|s| s.entities.len()).unwrap_or(0);
    Ok((entities, formatted == text))
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SchemaInput {
    /// Type names to return JSON Schemas for (e.g. ["Transform", "Camera"]).
    #[serde(default)]
    pub types: Vec<String>,
    /// Return the complete scene-file schema document.
    #[serde(default)]
    pub full: bool,
}

#[derive(Serialize, JsonSchema)]
pub struct SchemaOutput {
    pub components: Vec<String>,
    pub resources: Vec<String>,
    /// JSON Schemas of the requested `types`.
    #[serde(skip_serializing_if = "serde_json::Map::is_empty")]
    pub defs: serde_json::Map<String, Value>,
    /// The full schema document, only when `full` was set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<Value>,
}

fn schema_op(agent: &mut Agent, input: SchemaInput) -> Result<SchemaOutput, Errors> {
    let doc = schema::schema(agent.registry());
    let names = |k| agent.registry().of_kind(k).map(|e| e.name.to_string()).collect();
    let mut defs = serde_json::Map::new();
    for t in &input.types {
        match doc["$defs"].get(t) {
            Some(d) => {
                defs.insert(t.clone(), d.clone());
            }
            None => {
                return Err(
                    Diagnostic::error(codes::NOT_FOUND, format!("no type named `{t}`"))
                        .fix("call project_schema without `types` to list the type names")
                        .into(),
                );
            }
        }
    }
    Ok(SchemaOutput {
        components: names(Kind::Component),
        resources: names(Kind::Resource),
        defs,
        schema: input.full.then_some(doc),
    })
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateInput {
    /// Scene file to create, e.g. `scenes/level1.scene.json`.
    pub scene: String,
    /// `empty` (default) or `basic` (camera, light, ground).
    #[serde(default)]
    pub template: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct CreateOutput {
    pub scene: String,
    pub entities: usize,
}

fn create(agent: &mut Agent, input: CreateInput) -> Result<CreateOutput, Errors> {
    let path = agent.resolve(&input.scene)?;
    if !input.scene.ends_with(".scene.json") {
        return Err(
            Diagnostic::error(codes::PATH, "scene files must end in `.scene.json`")
                .fix(format!(
                    "use `{}.scene.json`",
                    input.scene.trim_end_matches(".json")
                ))
                .into(),
        );
    }
    if path.exists() {
        return Err(
            Diagnostic::error(codes::EXISTS, format!("`{}` already exists", input.scene))
                .file(&input.scene)
                .fix("edit it with entity_* / component_* operations, or pick another name")
                .into(),
        );
    }
    let doc = match input.template.as_deref().unwrap_or("empty") {
        "empty" => json!({ "format": scene::FORMAT, "version": scene::VERSION, "entities": [] }),
        "basic" => basic_scene(),
        other => {
            return Err(
                Diagnostic::error(codes::BAD_INPUT, format!("unknown template `{other}`"))
                    .fix("use `empty` or `basic`")
                    .into(),
            );
        }
    };
    let text = canonical(agent, &doc);
    let entities = doc["entities"].as_array().map_or(0, Vec::len);
    write_atomic(&path, &input.scene, text.as_bytes())?;
    Ok(CreateOutput {
        scene: input.scene,
        entities,
    })
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SceneInput {
    pub scene: String,
}

#[derive(Serialize, JsonSchema)]
pub struct ValidateOneOutput {
    pub scene: String,
    pub entities: usize,
    /// True when the file is byte-identical to its canonical form.
    pub canonical: bool,
}

fn validate_one(agent: &mut Agent, input: SceneInput) -> Result<ValidateOneOutput, Errors> {
    let path = agent.resolve(&input.scene)?;
    let (entities, canonical) = check_scene(agent, &path, &input.scene)?;
    Ok(ValidateOneOutput {
        scene: input.scene,
        entities,
        canonical,
    })
}

#[derive(Serialize, JsonSchema)]
pub struct FormatOutput {
    pub scene: String,
    pub changed: bool,
}

fn format(agent: &mut Agent, input: SceneInput) -> Result<FormatOutput, Errors> {
    let path = agent.resolve(&input.scene)?;
    let text = read_text(&path, &input.scene)?;
    let formatted = scene::format_text(&text, agent.registry()).map_err(|e| e.with_file(&input.scene))?;
    let changed = formatted != text;
    if changed {
        write_atomic(&path, &input.scene, formatted.as_bytes())?;
    }
    Ok(FormatOutput {
        scene: input.scene,
        changed,
    })
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunInput {
    pub scene: String,
    #[serde(default)]
    pub seed: u64,
    #[serde(default)]
    pub ticks: u64,
    /// Scripted input or a recording to replay (`*.inputs.json`, relative to the project root).
    pub input: Option<String>,
    /// Extra input events (absolute ticks).
    #[serde(default)]
    pub events: Vec<marionette_input::InputEvent>,
    /// Write the recording of every action change to this file.
    pub record: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct RunOutput {
    pub scene: String,
    pub seed: u64,
    pub ticks: u64,
    pub state_hash: String,
    /// Number of action changes recorded (only with `record`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recorded_events: Option<usize>,
}

pub(crate) fn load_sim(agent: &Agent, scene: &str, seed: u64) -> Result<Sim, Errors> {
    let path = agent.resolve(scene)?;
    let text = read_text(&path, scene)?;
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    Sim::with_modules(&text, Some(scene), seed, crate::engine::modules(), dir)
}

fn run(agent: &mut Agent, input: RunInput) -> Result<RunOutput, Errors> {
    let mut sim = load_sim(agent, &input.scene, input.seed)?;
    crate::engine::feed(agent, &mut sim, input.input.as_deref(), &input.events)?;
    sim.run(input.ticks);
    let recorded_events = match &input.record {
        Some(out) => Some(crate::engine::write_recording(agent, &sim, out)?),
        None => None,
    };
    Ok(RunOutput {
        scene: input.scene,
        seed: input.seed,
        ticks: input.ticks,
        state_hash: sim.state_hash(),
        recorded_events,
    })
}
