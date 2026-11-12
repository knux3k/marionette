//! `marionette`: the single command line tool. Every command accepts `--json`, which
//! prints one JSON object on stdout: `{"ok": true, ...}` or `{"ok": false, "diagnostics": [...]}`.
//! Exit code is 0 on success, 1 on any error.

use clap::{Parser, Subcommand};
use marionette_core::error::codes;
use marionette_core::{Diagnostic, Errors, Sim, default_registry, scene, schema};
use serde_json::{Value, json};
use std::path::Path;
use std::process::ExitCode;

mod render;

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
            headless: true,
            ticks,
            seed,
            ..
        } => run(&scene, ticks.unwrap_or(1000), seed),
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
        } => render::screenshot(&scene, ticks, seed, camera, &size, &out, msaa),
        Command::Bench {
            scene,
            frames,
            seed,
            camera,
            size,
            msaa,
        } => render::bench(&scene, frames, seed, camera, &size, msaa),
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
        Some("screenshot") => format!(
            "wrote {} ({}x{}, state {})",
            s("out"),
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
        Some("fmt") if value["changed"] == json!(true) => format!("formatted {}", s("file")),
        Some("fmt") => format!("already canonical: {}", s("file")),
        _ => value.to_string(),
    }
}

pub(crate) fn read(path: &str) -> Result<String, Errors> {
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

fn run(path: &str, ticks: u64, seed: u64) -> Result<Value, Errors> {
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
