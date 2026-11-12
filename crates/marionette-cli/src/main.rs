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
        /// Scripted input or a recording to replay (`*.inputs.json`).
        #[arg(long)]
        input: Option<String>,
        /// Write every input action change to this file when the run ends (replayable).
        #[arg(long)]
        record: Option<String>,
        /// Windowed mode: do not open the audio device.
        #[arg(long)]
        mute: bool,
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
        /// Image size per eye (default 1920x1080, or 2160x2160 with --headset).
        #[arg(long)]
        size: Option<String>,
        #[arg(long, default_value_t = 4)]
        msaa: u32,
        /// Views per frame: 2 renders a stereo pair (each eye at --size), as a headset does.
        #[arg(long, default_value_t = 1)]
        eyes: u32,
        /// Emulate a headset: 2 eyes at 2160x2160 each (unless --size), 64 mm apart, and the
        /// head turning +-25 degrees every 2 seconds, so culling and batching see motion.
        #[arg(long)]
        headset: bool,
        /// Untimed ticks (each with a frame) before measuring.
        #[arg(long, default_value_t = 30)]
        warmup: u32,
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
    /// AI providers: list them, show this project's choices and spending, switch one.
    Provider {
        #[command(subcommand)]
        command: ProviderCommand,
    },
    /// Serve every operation as an MCP tool over stdio.
    Mcp,
    /// Write the generated reference docs (operations, components, CLI help).
    Docs {
        /// Output directory.
        #[arg(long, default_value = "docs/ai/reference")]
        out: String,
        /// Do not write; fail if the committed files differ from what the code generates.
        #[arg(long)]
        check: bool,
    },
}

#[derive(Subcommand)]
enum ProviderCommand {
    /// Every provider, the kinds it offers and whether its key is set (operation `provider_list`).
    List,
    /// Provider and model per kind, readiness, spending and cap (operation `provider_status`).
    Status,
    /// Switch one kind to a provider: `marionette provider use image qwen` (operation `provider_use`).
    Use {
        /// language, image or model3d.
        kind: String,
        /// Provider name, e.g. mock, claude, qwen, tripo.
        provider: String,
        /// Model name (default: the provider's default model).
        #[arg(long)]
        model: Option<String>,
    },
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
        Command::Docs { out, check } => docs(&agent, &out, check),
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
            input,
            record,
            ..
        } => agent
            .call(
                "scene_run",
                json!({ "scene": scene, "seed": seed, "ticks": ticks.unwrap_or(1000),
                        "input": input, "record": record }),
            )
            .map(|v| merge("run", v)),
        Command::Run {
            scene,
            headless: false,
            ticks,
            seed,
            camera,
            size,
            input,
            record,
            mute,
        } => render::windowed(render::WindowedRun {
            path: &scene,
            ticks,
            seed,
            camera,
            size: &size,
            input: input.as_deref(),
            record: record.as_deref(),
            mute,
        }),
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
            eyes,
            headset,
            warmup,
        } => render::bench(render::BenchOptions {
            warmup,
            path: &scene,
            frames,
            seed,
            camera,
            size: size
                .as_deref()
                .unwrap_or(if headset { "2160x2160" } else { "1920x1080" }),
            msaa,
            eyes: if headset { 2 } else { eyes },
            headset,
        }),
        Command::Provider { command } => match command {
            ProviderCommand::List => agent
                .call("provider_list", json!({}))
                .map(|v| merge("provider_list", v)),
            ProviderCommand::Status => agent
                .call("provider_status", json!({}))
                .map(|v| merge("provider_status", v)),
            ProviderCommand::Use {
                kind,
                provider,
                model,
            } => {
                let mut input = json!({ "kind": kind, "provider": provider });
                if let Some(m) = model {
                    input["model"] = json!(m);
                }
                agent
                    .call("provider_use", input)
                    .map(|v| merge("provider_use", v))
            }
        },
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

/// Generated reference files: (file name, contents).
fn generated(agent: &Agent) -> Vec<(&'static str, String)> {
    use clap::CommandFactory;
    let fence = "```";
    let mut cli = String::from(
        "# CLI reference\n\nGENERATED by `marionette docs` from the CLI definition. Do not edit by hand.\n\n",
    );
    let mut cmd = Cli::command();
    cli.push_str(&format!("{fence}text\n{}\n{fence}\n\n", cmd.render_long_help()));
    for sub in cmd.get_subcommands_mut() {
        let name = sub.get_name().to_string();
        cli.push_str(&format!(
            "## `marionette {name}`\n\n{fence}text\n{}\n{fence}\n\n",
            sub.render_long_help()
        ));
    }
    vec![
        ("operations.md", marionette_agent::docs::operations(agent)),
        ("components.md", marionette_agent::docs::components(agent)),
        ("cli.md", cli.replace("marionette.exe", "marionette")),
    ]
}

fn docs(agent: &Agent, out: &str, check: bool) -> Result<Value, Errors> {
    let dir = std::path::Path::new(out);
    let mut changed = Vec::new();
    for (name, text) in generated(agent) {
        let path = dir.join(name);
        let current = std::fs::read_to_string(&path)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        if current == text {
            continue;
        }
        changed.push(path.to_string_lossy().replace('\\', "/"));
        if !check {
            std::fs::create_dir_all(dir)
                .and_then(|_| std::fs::write(&path, &text))
                .map_err(|e| {
                    Errors::from(Diagnostic::error(
                        marionette_core::error::codes::IO_WRITE,
                        format!("cannot write {}: {e}", path.display()),
                    ))
                })?;
        }
    }
    if check && !changed.is_empty() {
        return Err(Diagnostic::error(
            "docs.stale",
            format!("generated docs are out of date: {}", changed.join(", ")),
        )
        .fix("run `marionette docs` and commit the result")
        .into());
    }
    Ok(json!({ "ok": true, "command": "docs", "changed": changed }))
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
            "{} at {}x{} x{} eyes: avg {} ms, p99 {} ms, worst {} ms over {} frames \
             (median sim {} / extract {} / record {} / gpu wait {} ms; {} draw calls, {} instances, {} triangles)",
            s("gpu"),
            s("width"),
            s("height"),
            s("eyes"),
            s("avg_ms"),
            s("p99_ms"),
            s("max_ms"),
            s("frames"),
            value["phases_p50_ms"]["sim"],
            value["phases_p50_ms"]["extract"],
            value["phases_p50_ms"]["record"],
            value["phases_p50_ms"]["gpu_wait"],
            value["render"]["draw_calls"],
            value["render"]["instances"],
            value["render"]["triangles"]
        ),
        Some("fmt") if value["changed"] == json!(true) => format!("formatted {}", s("scene")),
        Some("fmt") => format!("already canonical: {}", s("scene")),
        Some("provider_use") => format!(
            "{} now uses {} (model {}; was {}); readiness: {}",
            s("kind"),
            s("provider"),
            s("model"),
            value["previous"]["provider"].as_str().unwrap_or(""),
            s("readiness")
        ),
        Some("provider_status") => value["kinds"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|k| {
                format!(
                    "{}: {} ({}), {}",
                    k["kind"].as_str().unwrap_or(""),
                    k["provider"].as_str().unwrap_or(""),
                    k["model"].as_str().unwrap_or(""),
                    k["readiness"].as_str().unwrap_or("")
                )
            })
            .chain([format!(
                "spent ${} of cap ${} ({})",
                s("spent_usd"),
                value["cap"]["usd"],
                value["cap"]["source"].as_str().unwrap_or("")
            )])
            .collect::<Vec<_>>()
            .join("\n"),
        Some("provider_list") => value["providers"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|p| {
                format!(
                    "{}: {} [{}]",
                    p["name"].as_str().unwrap_or(""),
                    p["kinds"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", "),
                    p["readiness"].as_str().unwrap_or("")
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => value.to_string(),
    }
}
