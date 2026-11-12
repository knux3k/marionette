//! `marionette`: the single command line tool. Every command accepts `--json`, which
//! prints one JSON object on stdout: `{"ok": true, ...}` or `{"ok": false, "diagnostics": [...]}`.
//! Exit code is 0 on success, 1 on any error.
//!
//! The engine's operations live in `marionette-agent`, shared with the MCP server.
//! `marionette op <name> '<json>'` runs any of them; the named commands below (`validate`,
//! `run --headless`, `screenshot`, `scene fmt`, `schema`) are shortcuts for the same operations.

use clap::{Parser, Subcommand};
use marionette_agent::Agent;
use marionette_core::{Diagnostic, Errors};
use serde_json::{Value, json};
use std::process::ExitCode;

mod render;

#[derive(Parser)]
#[command(name = "marionette", version, about = "Marionette game engine CLI")]
struct Cli {
    /// Print a single JSON object on stdout instead of human-readable text.
    #[arg(long, global = true)]
    json: bool,
    /// Project root; relative paths in operations resolve against it.
    #[arg(long, global = true, default_value = ".")]
    project: String,
    /// Refuse every operation that changes files or sessions.
    #[arg(long, global = true)]
    read_only: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print the JSON Schema of the scene format and every component and resource.
    Schema,
    /// Check a scene file against the schema.
    Validate { scene: String },
    /// Simulate a scene. With --headless, run N ticks and print a hash of the final state;
    /// otherwise open a window (closes after --ticks if given, or on Escape).
    Run {
        scene: String,
        /// Run without a window.
        #[arg(long)]
        headless: bool,
        /// Ticks to simulate (headless default: 1000; windowed default: until closed).
        #[arg(long)]
        ticks: Option<u64>,
        #[arg(long, default_value_t = 0)]
        seed: u64,
        /// Camera entity id (windowed mode; default: the lowest-id camera).
        #[arg(long)]
        camera: Option<u64>,
        /// Window size, e.g. 1280x720.
        #[arg(long, default_value = "1280x720")]
        size: String,
    },
    /// Simulate N ticks, then render one image with no window and save it as PNG.
    Screenshot {
        scene: String,
        #[arg(long, default_value_t = 0)]
        ticks: u64,
        #[arg(long, default_value_t = 0)]
        seed: u64,
        /// Camera entity id (default: the lowest-id camera).
        #[arg(long)]
        camera: Option<u64>,
        /// Image size, e.g. 1280x720.
        #[arg(long, default_value = "1280x720")]
        size: String,
        /// Output PNG path.
        #[arg(long)]
        out: String,
        /// MSAA sample count (1 or 4).
        #[arg(long, default_value_t = 4)]
        msaa: u32,
    },
    /// Measure frame time offscreen: each frame runs one tick, renders and waits for the GPU.
    Bench {
        scene: String,
        #[arg(long, default_value_t = 1000)]
        frames: u32,
        #[arg(long, default_value_t = 0)]
        seed: u64,
        #[arg(long)]
        camera: Option<u64>,
        #[arg(long, default_value = "1920x1080")]
        size: String,
        #[arg(long, default_value_t = 4)]
        msaa: u32,
    },
    /// Scene file tools.
    Scene {
        #[command(subcommand)]
        command: SceneCommand,
    },
    /// List every operation with its description and JSON Schemas.
    Ops,
    /// Run one operation: `marionette op entity_create '{"scene": "...", "components": {...}}'`.
    Op {
        name: String,
        /// Input as a JSON object (default `{}`).
        input: Option<String>,
    },
    /// Serve every operation as an MCP tool over stdio.
    Mcp,
}

#[derive(Subcommand)]
enum SceneCommand {
    /// Rewrite a scene file in canonical form.
    Fmt {
        scene: String,
        /// Do not write; fail if the file is not already canonical.
        #[arg(long)]
        check: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let mut agent = Agent::new(&cli.project, cli.read_only);
    let result = match cli.command {
        Command::Mcp => {
            return match marionette_agent::mcp::serve_stdio(agent) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::FAILURE
                }
            };
        }
        Command::Ops => Ok(json!({ "ok": true, "operations": agent.operations().collect::<Vec<_>>() })),
        Command::Op { name, input } => parse_input(input.as_deref())
            .and_then(|input| agent.call(&name, input))
            .map(|result| json!({ "ok": true, "operation": name, "result": result })),
        Command::Schema => agent
            .call("project_schema", json!({ "full": true }))
            .map(|v| v["schema"].clone()),
        Command::Validate { scene } => agent
            .call("scene_validate", json!({ "scene": scene }))
            .map(|v| merge("validate", v)),
        Command::Run {
            scene,
            headless: true,
            ticks,
            seed,
            ..
        } => agent
            .call(
                "scene_run",
                json!({ "scene": scene, "seed": seed, "ticks": ticks.unwrap_or(1000) }),
            )
            .map(|v| merge("run", v)),
        Command::Run {
            scene,
            headless: false,
            ticks,
            seed,
            camera,
            size,
        } => render::windowed(&scene, ticks, seed, camera, &size),
        Command::Screenshot {
            scene,
            ticks,
            seed,
            camera,
            size,
            out,
            msaa,
        } => agent
            .call(
                "observe_screenshot",
                json!({ "scene": scene, "ticks": ticks, "seed": seed, "camera": camera,
                        "size": size, "out": out, "msaa": msaa }),
            )
            .map(|v| merge("screenshot", v)),
        Command::Bench {
            scene,
            frames,
            seed,
            camera,
            size,
            msaa,
        } => render::bench(&scene, frames, seed, camera, &size, msaa),
        Command::Scene {
            command: SceneCommand::Fmt { scene, check: false },
        } => agent
            .call("scene_format", json!({ "scene": scene }))
            .map(|v| merge("fmt", v)),
        Command::Scene {
            command: SceneCommand::Fmt { scene, check: true },
        } => agent
            .call("scene_validate", json!({ "scene": scene }))
            .and_then(|v| {
                if v["canonical"] == true {
                    Ok(merge("fmt", json!({ "scene": scene, "changed": false })))
                } else {
                    Err(Diagnostic::error(
                        marionette_core::error::codes::SCENE_NOT_CANONICAL,
                        "scene file is not in canonical form",
                    )
                    .file(&scene)
                    .fix(format!("run `marionette scene fmt {scene}`"))
                    .into())
                }
            }),
    };
    match result {
        Ok(value) => {
            let always_json = ["$schema", "operation", "operations"]
                .iter()
                .any(|k| value.get(k).is_some());
            if cli.json || always_json {
                println!("{}", serde_json::to_string_pretty(&value).expect("serializable"));
            } else {
                println!("{}", human(&value));
            }
            ExitCode::SUCCESS
        }
        Err(errors) => {
            if cli.json {
                let out = json!({ "ok": false, "diagnostics": errors });
                println!("{}", serde_json::to_string_pretty(&out).expect("serializable"));
            } else {
                eprintln!("{errors}");
            }
            ExitCode::FAILURE
        }
    }
}

fn parse_input(input: Option<&str>) -> Result<Value, Errors> {
    match input {
        None => Ok(json!({})),
        Some(text) => serde_json::from_str(text).map_err(|e| {
            Diagnostic::error(
                marionette_agent::codes::BAD_INPUT,
                format!("input is not valid JSON: {e}"),
            )
            .at(Some(e.line()), Some(e.column()))
            .fix("pass one JSON object, quoted for your shell")
            .into()
        }),
    }
}

/// `{"ok": true, "command": ..., ...operation output}` for the shortcut commands.
fn merge(command: &str, v: Value) -> Value {
    let mut out = json!({ "ok": true, "command": command });
    if let (Value::Object(o), Value::Object(m)) = (&mut out, v) {
        o.extend(m);
    }
    out
}

/// Short human text for a successful result.
fn human(value: &Value) -> String {
    let s = |k: &str| {
        value
            .get(k)
            .map(|v| v.to_string().trim_matches('"').to_string())
            .unwrap_or_default()
    };
    match value.get("command").and_then(Value::as_str) {
        Some("validate") => format!("ok: {} ({} entities)", s("scene"), s("entities")),
        Some("run") => s("state_hash"),
        Some("screenshot") => format!(
            "wrote {} ({}x{}, state {})",
            s("path"),
            s("width"),
            s("height"),
            s("state_hash")
        ),
        Some("bench") => format!(
            "{} at {}x{}: avg {} ms, p99 {} ms, worst {} ms over {} frames",
            s("gpu"),
            s("width"),
            s("height"),
            s("avg_ms"),
            s("p99_ms"),
            s("max_ms"),
            s("frames")
        ),
        Some("fmt") if value["changed"] == json!(true) => format!("formatted {}", s("scene")),
        Some("fmt") => format!("already canonical: {}", s("scene")),
        _ => value.to_string(),
    }
}
