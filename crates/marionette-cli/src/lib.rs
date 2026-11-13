//! `marionette`: the single command line tool. Every command accepts `--json`, which
//! prints one JSON object on stdout: `{"ok": true, ...}` or `{"ok": false, "diagnostics": [...]}`.
//! Exit code is 0 on success, 1 on any error.
//!
//! The engine's operations live in `marionette-agent`, shared with the MCP server.
//! `marionette op <name> '<json>'` runs any of them; the named commands below (`validate`,
//! `run --headless`, `screenshot`, `scene fmt`, `schema`) are shortcuts for the same operations.
//!
//! A game with its own Rust modules or operations makes its own binary with the same commands:
//! `fn main() -> ExitCode { marionette_cli::main_with(engine) }` (decision 0062).

use clap::{Parser, Subcommand};
use marionette_agent::{Agent, Engine};
use marionette_core::{Diagnostic, Errors};
use serde_json::{Value, json};
use std::process::ExitCode;

mod render;
mod stability;

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
        #[command(flatten)]
        window: render::WindowFlags,
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
    /// Stability suite (operation `stability_run`): run every game mode with seeded random
    /// input and check no panic, no hang, replay and snapshot determinism, time budgets and
    /// memory growth. Writes `report.json` and, per failure, `<mode>-seed<N>.crash.json` and a
    /// replayable `<mode>-seed<N>.inputs.json` to --out. Exit code 1 if any mode failed.
    Stability {
        /// Suite file.
        #[arg(long, default_value = "stability/suite.json")]
        suite: String,
        /// Only this mode (repeatable).
        #[arg(long = "mode")]
        modes: Vec<String>,
        /// Long soak: long_ticks per mode and 3 seeds (default: fast, for every push).
        #[arg(long)]
        long: bool,
        /// First seed (default: the suite's seed).
        #[arg(long)]
        seed: Option<u64>,
        /// Seeds per mode.
        #[arg(long)]
        seeds: Option<u32>,
        /// Ticks per mode, overriding the suite.
        #[arg(long)]
        ticks: Option<u64>,
        /// Check time budgets even in a debug build.
        #[arg(long)]
        enforce_timing: bool,
        /// Report directory.
        #[arg(long, default_value = "target/stability")]
        out: String,
    },
    /// Scene file tools.
    Scene {
        #[command(subcommand)]
        command: SceneCommand,
    },
    /// List operations with their descriptions and JSON Schemas. `--brief` gives one line each.
    Ops {
        /// Only name, mutating flag and summary (no schemas): a few KB instead of about 300 KB.
        #[arg(long)]
        brief: bool,
        /// Only these groups or operations (as for `mcp --tools`).
        #[arg(long, value_delimiter = ',')]
        tools: Vec<String>,
    },
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
    /// Generate assets from recipes (last step of the asset lookup order): plan, run, list,
    /// and approve an image for a paid 3D model (a person only).
    Generate {
        #[command(subcommand)]
        command: GenerateCommand,
    },
    /// Serve operations as MCP tools over stdio (all of them unless `--tools` says otherwise).
    Mcp {
        /// Comma-separated operation groups or names to list: `core` (edit, run, observe,
        /// test), a group such as `pool` or `anim`, an operation name, or `all` (default).
        #[arg(long, value_delimiter = ',')]
        tools: Vec<String>,
        /// Leave output schemas out of the tool listing (smaller context for the client).
        #[arg(long)]
        lean: bool,
    },
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
enum GenerateCommand {
    /// What a run would do: cache hits, calls, cost, refusals (operation `generate_plan`).
    Plan {
        /// Recipe names (default: all).
        names: Vec<String>,
        #[arg(long, default_value = "assets/generated")]
        dir: String,
    },
    /// Build recipes into assets with provenance (operation `generate_run`).
    Run {
        names: Vec<String>,
        #[arg(long, default_value = "assets/generated")]
        dir: String,
        /// Use cached provider outputs only.
        #[arg(long)]
        offline: bool,
    },
    /// Recipes and the state of their outputs (operation `generate_list`).
    List {
        #[arg(long, default_value = "assets/generated")]
        dir: String,
    },
    /// Approve a generated image so a paid 3D model may be made from it. A person must type
    /// the image's name at an interactive terminal; there is no operation for this. With
    /// `--all`, every reference image under the project that still waits is listed and a
    /// person types `yes` once to approve them all.
    Approve {
        /// The image recipe to approve (not with `--all`).
        #[arg(required_unless_present = "all", conflicts_with = "all")]
        name: Option<String>,
        /// Approve every waiting reference image under the project at once.
        #[arg(long)]
        all: bool,
        #[arg(long, default_value = "assets/generated")]
        dir: String,
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

/// The `marionette` tool with the built-in engine.
pub fn main() -> ExitCode {
    main_with(Engine::standard())
}

/// The `marionette` tool with a game's engine (its own modules and operations added with
/// [`marionette_agent::EngineBuilder`]). Every command, the MCP server included, uses it.
pub fn main_with(engine: Engine) -> ExitCode {
    // Parsing may print help and exit; nothing is held yet at that point.
    let cli = Cli::parse();
    // Tie child processes to this process and remove what crashed engine processes left
    // behind; this process's own run folder is removed on the way out (decision 0032). A
    // panic or kill skips `shutdown`; the next start's sweep covers that.
    marionette_host::init();
    let _ = marionette_host::run_dir();
    let code = run(cli, engine);
    marionette_host::shutdown();
    code
}

fn run(cli: Cli, engine: Engine) -> ExitCode {
    let mut agent = Agent::with_engine(&cli.project, cli.read_only, engine.clone());
    let result = match cli.command {
        Command::Mcp { tools, lean } => {
            let options = marionette_agent::mcp::McpOptions { tools, lean };
            return match marionette_agent::mcp::serve_stdio(agent, &options) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::FAILURE
                }
            };
        }
        Command::Docs { out, check } => docs(&agent, &out, check),
        Command::Ops { brief, tools } => ops(&agent, brief, &tools),
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
            window,
        } => render::windowed(render::WindowedRun {
            engine: &engine,
            path: &scene,
            ticks,
            seed,
            camera,
            size: &size,
            input: input.as_deref(),
            record: record.as_deref(),
            mute,
            window,
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
            engine: &engine,
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
        Command::Stability {
            suite,
            modes,
            long,
            seed,
            seeds,
            ticks,
            enforce_timing,
            out,
        } => {
            let input = json!({ "suite": suite, "modes": modes, "length": if long { "long" } else { "fast" },
                                "seed": seed, "seeds": seeds, "ticks": ticks,
                                "enforce_timing": enforce_timing.then_some(true) });
            let report = agent
                .call("stability_run", input)
                .and_then(|v| stability::write(&out, v));
            // A hung worker thread may still be running: report, then end the process.
            let code = match &report {
                Ok(v) if v["passed"] == true => 0,
                _ => 1,
            };
            let _ = print_result(cli.json, report);
            std::process::exit(code);
        }
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
        Command::Generate { command } => match command {
            GenerateCommand::Plan { names, dir } => agent
                .call("generate_plan", json!({ "names": names, "dir": dir }))
                .map(|v| merge("generate_plan", v)),
            GenerateCommand::Run { names, dir, offline } => agent
                .call(
                    "generate_run",
                    json!({ "names": names, "dir": dir, "offline": offline }),
                )
                .map(|v| merge("generate_run", v)),
            GenerateCommand::List { dir } => agent
                .call("generate_list", json!({ "dir": dir }))
                .map(|v| merge("generate_list", v)),
            GenerateCommand::Approve { name, all, dir } => match name {
                Some(name) if !all => approve(&cli.project, &name, &dir, cli.read_only),
                _ => approve_all(&cli.project, cli.read_only),
            },
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
    print_result(cli.json, result)
}

/// JSON for stdout: indented for a person at a terminal, one compact line when piped (an
/// agent or a script reads it; indentation only costs tokens there, decision 0061).
fn json_text(value: &Value) -> String {
    use std::io::IsTerminal;
    if std::io::stdout().is_terminal() {
        serde_json::to_string_pretty(value)
    } else {
        serde_json::to_string(value)
    }
    .expect("serializable")
}

fn print_result(json_out: bool, result: Result<Value, Errors>) -> ExitCode {
    match result {
        Ok(value) => {
            let always_json = ["$schema", "operation", "operations"]
                .iter()
                .any(|k| value.get(k).is_some());
            if json_out || always_json {
                println!("{}", json_text(&value));
            } else {
                println!("{}", human(&value));
            }
            ExitCode::SUCCESS
        }
        Err(errors) => {
            if json_out {
                let out = json!({ "ok": false, "diagnostics": errors });
                println!("{}", json_text(&out));
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
    // Files of parts left out of this build (cargo features) are not written.
    vec![
        ("operations.md", marionette_agent::docs::operations(agent)),
        ("components.md", marionette_agent::docs::components(agent)),
        #[cfg(feature = "port")]
        ("porting-map.md", marionette_agent::docs::porting_map()),
        ("cli.md", cli.replace("marionette.exe", "marionette")),
        #[cfg(feature = "script")]
        ("marionette.d.ts", marionette_agent::docs::script_typings(agent)),
        #[cfg(feature = "jobs-ts")]
        ("jobs.d.ts", marionette_agent::docs::job_typings()),
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

#[cfg(not(feature = "generate"))]
fn approve(_: &str, _: &str, _: &str, _: bool) -> Result<Value, Errors> {
    approve_all("", false)
}

#[cfg(not(feature = "generate"))]
fn approve_all(_: &str, _: bool) -> Result<Value, Errors> {
    Err(Diagnostic::error(
        marionette_agent::codes::UNKNOWN_OPERATION,
        "this build has no generation (cargo feature `generate`)",
    )
    .into())
}

/// `marionette ops`: the operation table, whole or brief, optionally only some groups.
fn ops(agent: &Agent, brief: bool, tools: &[String]) -> Result<Value, Errors> {
    let selected = marionette_agent::mcp::select_operations(agent, tools)
        .map_err(|e| Errors::from(Diagnostic::error(marionette_agent::codes::BAD_INPUT, e)))?;
    let list: Vec<Value> = agent
        .operations()
        .filter(|o| selected.contains(&o.name))
        .map(|o| {
            if brief {
                json!({ "name": o.name, "mutating": o.mutating, "description": o.description })
            } else {
                json!(o)
            }
        })
        .collect();
    Ok(json!({ "ok": true, "operations": list }))
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

/// `marionette generate approve`: a person looks at the image and types its name.
#[cfg(feature = "generate")]
fn approve(project: &str, name: &str, dir: &str, read_only: bool) -> Result<Value, Errors> {
    use marionette_generate::approval;
    use std::io::{BufRead, IsTerminal, Write};
    if read_only {
        return Err(Diagnostic::error(
            marionette_agent::codes::READ_ONLY,
            "approving writes a file and --read-only is set",
        )
        .into());
    }
    let root = std::path::Path::new(project);
    let target = marionette_generate::Target {
        root,
        dir,
        names: &[],
    };
    let (path, file, sha) = marionette_generate::approvable(&target, name)?;
    let interactive = std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
    let mut confirmed = false;
    if interactive {
        eprintln!(
            "Approve {} (sha256 {}) so a paid 3D model may be made from it?",
            path.display(),
            &sha[..16]
        );
        eprint!("Look at the image first. Type its name ({name}) to approve: ");
        let _ = std::io::stderr().flush();
        let mut line = String::new();
        let _ = std::io::stdin().lock().read_line(&mut line);
        confirmed = line.trim() == name;
    }
    if !confirmed && interactive {
        return Err(Diagnostic::error(
            marionette_generate::codes::APPROVAL_NEEDS_PERSON,
            "not approved: the typed name did not match",
        )
        .into());
    }
    approval::approve_by_person(&root.join(dir), name, &file, &sha, confirmed)?;
    Ok(merge(
        "generate_approve",
        json!({ "name": name, "file": file, "sha256": sha, "approval": format!("{name}{}", approval::SUFFIX) }),
    ))
}

/// `marionette generate approve --all`: lists every waiting reference image and approves them
/// all after a person types `yes` at an interactive terminal.
#[cfg(feature = "generate")]
fn approve_all(project: &str, read_only: bool) -> Result<Value, Errors> {
    use marionette_generate::approval;
    use std::io::{BufRead, IsTerminal, Write};
    if read_only {
        return Err(Diagnostic::error(
            marionette_agent::codes::READ_ONLY,
            "approving writes files and --read-only is set",
        )
        .into());
    }
    let root = std::path::Path::new(project);
    let pending = marionette_generate::pending_approvals(root)?;
    if pending.is_empty() {
        return Ok(merge("generate_approve", json!({ "approved": [] })));
    }
    let interactive = std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
    let mut confirmed = false;
    if interactive {
        eprintln!("These images wait for approval; each lets a paid 3D model be made from it:");
        for p in &pending {
            eprintln!("  {}/{}  (sha256 {})", p.dir, p.file, &p.sha256[..16]);
        }
        eprint!("Look at them first. Type yes to approve all {}: ", pending.len());
        let _ = std::io::stderr().flush();
        let mut line = String::new();
        let _ = std::io::stdin().lock().read_line(&mut line);
        confirmed = line.trim().eq_ignore_ascii_case("yes");
        if !confirmed {
            return Err(Diagnostic::error(
                marionette_generate::codes::APPROVAL_NEEDS_PERSON,
                "not approved: the answer was not yes",
            )
            .into());
        }
    }
    for p in &pending {
        approval::approve_by_person(&root.join(&p.dir), &p.name, &p.file, &p.sha256, confirmed)?;
    }
    Ok(merge("generate_approve", json!({ "approved": pending })))
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
        Some("stability") => stability::human(value),
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
        Some("generate_plan") => value["steps"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|st| {
                let blocked = st["blocked"]["message"]
                    .as_str()
                    .map(|m| format!(" ({m})"))
                    .unwrap_or_default();
                format!(
                    "{}: {} via {} {}, est ${}{}",
                    st["name"].as_str().unwrap_or(""),
                    st["status"].as_str().unwrap_or(""),
                    st["provider"].as_str().unwrap_or(""),
                    st["model"].as_str().unwrap_or(""),
                    st["estimate_usd"],
                    blocked
                )
            })
            .chain([format!(
                "total est ${}; spent ${} of cap ${}",
                s("total_estimate_usd"),
                s("spent_usd"),
                value["cap"]["usd"]
            )])
            .collect::<Vec<_>>()
            .join("\n"),
        Some("generate_run") => value["built"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|b| {
                format!(
                    "{}: {} ({})",
                    b["name"].as_str().unwrap_or(""),
                    b["file"].as_str().unwrap_or(""),
                    b["how"].as_str().unwrap_or("")
                )
            })
            .chain(value["blocked"].as_array().into_iter().flatten().map(|b| {
                format!(
                    "{}: refused: {}",
                    b["name"].as_str().unwrap_or(""),
                    b["problem"]["message"].as_str().unwrap_or("")
                )
            }))
            .chain([format!("{} calls, ${} spent now", s("calls"), s("cost_usd"))])
            .collect::<Vec<_>>()
            .join("\n"),
        Some("generate_list") => value["recipes"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|r| {
                format!(
                    "{}: {} {}",
                    r["name"].as_str().unwrap_or(""),
                    r["output"].as_str().unwrap_or(""),
                    r["state"].as_str().unwrap_or("")
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Some("generate_approve") => format!("approved {}", s("file")),
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
