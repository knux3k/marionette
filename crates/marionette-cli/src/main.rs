//! `marionette`: the single command line tool. Every command accepts `--json`, which
//! prints one JSON object on stdout: `{"ok": true, ...}` or `{"ok": false, "diagnostics": [...]}`.
//! Exit code is 0 on success, 1 on any error.

use clap::{Parser, Subcommand};
use marionette_core::error::codes;
use marionette_core::{Diagnostic, Errors, Sim, default_registry, scene, schema};
use serde_json::{Value, json};
use std::path::Path;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "marionette", version, about = "Marionette game engine CLI")]
struct Cli {
    /// Print a single JSON object on stdout instead of human-readable text.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print the JSON Schema of the scene format and every component and resource.
    Schema,
    /// Check a scene file against the schema.
    Validate { scene: String },
    /// Simulate a scene and print a hash of the final state.
    Run {
        scene: String,
        /// Run without a window (required: windowed mode does not exist yet).
        #[arg(long)]
        headless: bool,
        #[arg(long, default_value_t = 1000)]
        ticks: u64,
        #[arg(long, default_value_t = 0)]
        seed: u64,
    },
    /// Scene file tools.
    Scene {
        #[command(subcommand)]
        command: SceneCommand,
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
    let result = match cli.command {
        Command::Schema => Ok(schema::schema(&default_registry())),
        Command::Validate { scene } => validate(&scene),
        Command::Run {
            scene,
            headless,
            ticks,
            seed,
        } => run(&scene, headless, ticks, seed),
        Command::Scene {
            command: SceneCommand::Fmt { scene, check },
        } => fmt(&scene, check),
    };
    match result {
        Ok(value) => {
            if cli.json || value.get("$schema").is_some() {
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

/// Short human text for a successful result.
fn human(value: &Value) -> String {
    let s = |k: &str| {
        value
            .get(k)
            .map(|v| v.to_string().trim_matches('"').to_string())
            .unwrap_or_default()
    };
    match value.get("command").and_then(Value::as_str) {
        Some("validate") => format!("ok: {} ({} entities)", s("file"), s("entities")),
        Some("run") => s("state_hash"),
        Some("fmt") if value["changed"] == json!(true) => format!("formatted {}", s("file")),
        Some("fmt") => format!("already canonical: {}", s("file")),
        _ => value.to_string(),
    }
}

fn read(path: &str) -> Result<String, Errors> {
    std::fs::read_to_string(path).map_err(|e| {
        Diagnostic::error(codes::IO_READ, format!("cannot read {path}: {e}"))
            .file(path)
            .fix("check the path exists and is readable")
            .into()
    })
}

fn validate(path: &str) -> Result<Value, Errors> {
    let text = read(path)?;
    let registry = default_registry();
    let parsed = scene::parse(&text).map_err(|e| e.with_file(path))?;
    scene::validate(&parsed, &registry, Some(&text)).map_err(|e| e.with_file(path))?;
    Ok(json!({ "ok": true, "command": "validate", "file": path, "entities": parsed.entities.len() }))
}

fn run(path: &str, headless: bool, ticks: u64, seed: u64) -> Result<Value, Errors> {
    if !headless {
        return Err(
            Diagnostic::error("cli.no_window", "windowed mode is not available yet")
                .fix("pass --headless")
                .into(),
        );
    }
    let text = read(path)?;
    let mut sim = Sim::from_text(&text, Some(path), default_registry(), seed)?;
    sim.run(ticks);
    Ok(json!({
        "ok": true,
        "command": "run",
        "file": path,
        "ticks": ticks,
        "seed": seed,
        "state_hash": sim.state_hash(),
    }))
}

fn fmt(path: &str, check: bool) -> Result<Value, Errors> {
    let text = read(path)?;
    let formatted = scene::format_text(&text, &default_registry()).map_err(|e| e.with_file(path))?;
    let changed = formatted != text;
    if check && changed {
        return Err(
            Diagnostic::error(codes::SCENE_NOT_CANONICAL, "scene file is not in canonical form")
                .file(path)
                .fix(format!("run `marionette scene fmt {path}`"))
                .into(),
        );
    }
    if changed && !check {
        std::fs::write(Path::new(path), &formatted).map_err(|e| {
            Errors::from(Diagnostic::error(codes::IO_WRITE, format!("cannot write {path}: {e}")).file(path))
        })?;
    }
    Ok(
        json!({ "ok": true, "command": "fmt", "file": path, "changed": changed, "written": changed && !check }),
    )
}
