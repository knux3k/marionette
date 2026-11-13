//! The game starter operations (decision 0150): `game_brief` turns a prompt into a brief;
//! `game_make` turns a brief into a playable, tested project. Templates, levels and the brief
//! format live in `marionette-starter`; this file resolves assets through the lookup order,
//! writes the files, builds the scene, and lets the autopilot prove the level can be won.

use crate::{Agent, Op, codes, op};
use marionette_core::{Diagnostic, Errors};
use marionette_starter::template::{self, SlotKind};
use marionette_starter::{Art, ArtMap, Brief, Template, View, codes as starter_codes, prompt};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;

pub(crate) fn ops() -> Vec<Op> {
    vec![
        op(
            "game_brief",
            "Turns a one-line game idea into a game brief (marionette.brief/1: template, title, style, colors, one description per asset slot) without writing anything, or checks a brief you wrote. Read it, improve it (you write better briefs than the keyword reader), then pass it to game_make.",
            false,
            brief_op,
        ),
        op(
            "game_make",
            "Makes a playable game project from a brief or a prompt: level from the brief's seed, the template's TypeScript game script, scene recipe, input map, generation recipes for art not found by the asset lookup order (project files, CC0 pool), and tests. The template's autopilot must win the level (other seeds are tried if not), and doing nothing must not. `refresh` re-reads the project's brief.json, swaps in art built since, keeps edits outside the script's CONFIG block and re-pins the tests. `generate` runs generate_run for missing art first.",
            true,
            make_op,
        ),
    ]
}

fn bad(msg: impl Into<String>, fix: impl Into<String>) -> Errors {
    Diagnostic::error(codes::BAD_INPUT, msg.into()).fix(fix.into()).into()
}

// ---------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct BriefInput {
    /// The game idea in plain words ("a cozy 3d maze where a moth collects lanterns").
    #[serde(default)]
    prompt: Option<String>,
    /// Check this brief instead of reading a prompt.
    #[serde(default)]
    brief: Option<Brief>,
    /// Force a template (`maze` or `platformer`).
    #[serde(default)]
    template: Option<Template>,
    /// Force a view (`2d` or `3d`; 3D is maze only).
    #[serde(default)]
    view: Option<View>,
}

#[derive(Serialize, JsonSchema)]
struct SlotInfo {
    name: &'static str,
    /// `sprite`, `tile`, `backdrop` or `model`.
    kind: &'static str,
    describe: String,
}

#[derive(Serialize, JsonSchema)]
struct BriefOutput {
    brief: Brief,
    slots: Vec<SlotInfo>,
}

fn kind_name(k: SlotKind) -> &'static str {
    match k {
        SlotKind::Sprite { .. } => "sprite",
        SlotKind::Tile { .. } => "tile",
        SlotKind::Backdrop => "backdrop",
        SlotKind::Model { .. } => "model",
    }
}

fn brief_from(prompt_text: Option<&str>, brief: Option<Brief>, t: Option<Template>, v: Option<View>) -> Result<Brief, Errors> {
    let b = match (brief, prompt_text) {
        (Some(b), _) => b,
        (None, Some(p)) if !p.trim().is_empty() => prompt::brief_from_prompt(p, t, v),
        _ => return Err(bad("give a `prompt` or a `brief`", "for example {\"prompt\": \"a 3d maze about a moth collecting lanterns\"}")),
    };
    b.check()?;
    Ok(b)
}

fn brief_op(_agent: &mut Agent, input: BriefInput) -> Result<BriefOutput, Errors> {
    let brief = brief_from(input.prompt.as_deref(), input.brief, input.template, input.view)?;
    let slots = template::slots(brief.template, brief.view)
        .iter()
        .map(|s| SlotInfo {
            name: s.name,
            kind: kind_name(s.kind),
            describe: template::describe(&brief, s).to_string(),
        })
        .collect();
    Ok(BriefOutput { brief, slots })
}

// ---------------------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct MakeInput {
    /// Project folder to create (must not exist or be empty), or to refresh.
    path: String,
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    brief: Option<Brief>,
    #[serde(default)]
    template: Option<Template>,
    #[serde(default)]
    view: Option<View>,
    /// Re-make an existing starter project from its brief.json (or from `brief`).
    #[serde(default)]
    refresh: bool,
    /// Run generate_run for slots still waiting for art (paid providers follow the project's
    /// providers.json, keys and spend caps), then use what was built.
    #[serde(default)]
    generate: bool,
    /// Seeds to try when the autopilot cannot win a level (default 8).
    #[serde(default)]
    tries: Option<u32>,
}

#[derive(Serialize, JsonSchema)]
struct SlotResult {
    slot: String,
    /// `brief` (its `use`), `generated` (a built recipe output), `project` or `pool` (found
    /// by the lookup order), or `placeholder` (waiting for generation).
    source: String,
    /// Project-relative file drawn, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    file: Option<String>,
}

#[derive(Serialize, JsonSchema)]
struct Autoplay {
    won: bool,
    /// Seconds the autopilot took to win.
    seconds: f64,
    /// Level seed used (written back to brief.json).
    seed: u64,
    /// Seeds tried before one could be won.
    tries: u32,
}

#[derive(Serialize, JsonSchema)]
struct MakeOutput {
    path: String,
    title: String,
    template: Template,
    view: View,
    files: Vec<String>,
    assets: Vec<SlotResult>,
    /// Recipes in assets/generated still to build (generate_run in the project, then
    /// game_make with `refresh`).
    pending: Vec<String>,
    /// What generate_run reported, when `generate` was set.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    generation: Vec<String>,
    autoplay: Autoplay,
    tests_passed: usize,
    tests_failed: usize,
    /// AI handoff files for agents working on the game: `written`, or why not.
    handoff: String,
}

const BRIEF_FILE: &str = "brief.json";

fn pretty(v: &impl Serialize) -> String {
    let mut s = serde_json::to_string_pretty(v).expect("serializes");
    s.push('\n');
    s
}

fn make_op(agent: &mut Agent, input: MakeInput) -> Result<MakeOutput, Errors> {
    let dir = agent.resolve(&input.path)?;
    let has_files = dir.is_dir() && std::fs::read_dir(&dir).map(|mut d| d.next().is_some()).unwrap_or(false);
    let brief = if input.refresh {
        if !has_files {
            return Err(bad(
                format!("`{}` is not a starter project to refresh", input.path),
                "drop `refresh` to make a new project",
            ));
        }
        match input.brief {
            Some(b) => b,
            None => {
                let text = std::fs::read_to_string(dir.join(BRIEF_FILE)).map_err(|e| {
                    bad(format!("cannot read {}/{BRIEF_FILE}: {e}", input.path), "refresh needs the project's brief.json")
                })?;
                serde_json::from_str::<Brief>(&text).map_err(|e| {
                    Errors::from(
                        Diagnostic::error(starter_codes::BAD_BRIEF, format!("brief.json: {e}"))
                            .file(format!("{}/{BRIEF_FILE}", input.path))
                            .at(Some(e.line()), Some(e.column())),
                    )
                })?
            }
        }
    } else {
        if has_files {
            return Err(Diagnostic::error(codes::EXISTS, format!("`{}` already has files", input.path))
                .fix("pick a new folder, or pass `refresh: true` to re-make a starter project")
                .into());
        }
        brief_from(input.prompt.as_deref(), input.brief, input.template, input.view)?
    };
    brief.check()?;
    if agent.read_only() {
        return Err(Diagnostic::error(codes::READ_ONLY, "game_make writes files").into());
    }
    std::fs::create_dir_all(&dir).map_err(|e| bad(format!("cannot create `{}`: {e}", input.path), "check the path"))?;
    let mut game = Agent::with_engine(&dir, false, agent.engine().clone());
    let mut files = Vec::new();
    let write = |game: &Agent, path: &str, text: String, files: &mut Vec<String>| -> Result<(), Errors> {
        game.write_file(path, text.as_bytes())?;
        files.push(path.to_string());
        Ok(())
    };

    if !dir.join("marionette.json").exists() {
        let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "game".into());
        write(&game, "marionette.json", pretty(&json!({"format": "marionette.project", "name": name, "version": 1})), &mut files)?;
    }
    if !dir.join("providers.json").exists() {
        write(
            &game,
            "providers.json",
            pretty(&json!({"format": "marionette.providers/1", "image": {"provider": "qwen"}, "model3d": {"provider": "tripo"}})),
            &mut files,
        )?;
    }

    // Art: resolve, then (optionally) generate what is missing and resolve again.
    let (mut art, mut results, mut pending) = resolve_art(&mut game, &brief, &mut files)?;
    let mut generation = Vec::new();
    if input.generate && !pending.is_empty() {
        let names: Vec<String> = pending.clone();
        match game.call("generate_run", json!({"names": names})) {
            Ok(report) => generation.push(compact(&report)),
            Err(e) => generation.extend(e.0.iter().map(|d| format!("{}: {}", d.code, d.message))),
        }
        (art, results, pending) = resolve_art(&mut game, &brief, &mut files)?;
    }

    write(&game, "scenes/main.actions.json", pretty(&template::actions(brief.template)), &mut files)?;
    write(&game, "tests/autoplay.inputs.json", pretty(&template::autoplay_inputs()), &mut files)?;

    // The brief goes in first, so a project whose level could not be won can still be
    // fixed and refreshed.
    write(&game, BRIEF_FILE, pretty(&brief), &mut files)?;

    // Level: the brief's seed first; if the autopilot cannot win it, the next seeds.
    let tries = input.tries.unwrap_or(8).clamp(1, 64);
    let mut outcome = None;
    let mut seed = brief.seed;
    for attempt in 0..tries {
        seed = brief.seed + attempt as u64;
        let map = template::level(&brief, seed);
        build(&mut game, &brief, &art, &map, input.refresh)?;
        let limit = max_ticks(&brief, &map);
        if let Some(t) = autoplay_win(&game, limit)? {
            outcome = Some((t, attempt + 1));
            break;
        }
    }
    let Some((won_tick, tried)) = outcome else {
        return Err(Diagnostic::error(
            starter_codes::UNWINNABLE,
            format!("the autopilot could not win any of {tries} levels (seeds {} to {seed})", brief.seed),
        )
        .fix("try another seed or an easier difficulty in the brief, or raise `tries`")
        .into());
    };
    let mut brief = brief;
    brief.seed = seed;

    // Pin the tests to what this build does.
    let auto_ticks = won_tick + 30;
    let auto_hash = run_hash(&game, auto_ticks, Some("tests/autoplay.inputs.json"))?;
    write(&game, "tests/autoplay.test.json", pretty(&template::autoplay_test(auto_ticks, &auto_hash)), &mut files)?;
    let idle_hash = run_hash(&game, template::IDLE_TICKS, None)?;
    write(&game, "tests/idle.test.json", pretty(&template::idle_test(template::IDLE_TICKS, &idle_hash)), &mut files)?;
    write(&game, BRIEF_FILE, pretty(&brief), &mut files)?;
    write(&game, "README.md", template::readme(&brief), &mut files)?;
    files.extend([template::SCRIPT.to_string(), "build.ops.json".into(), template::SCENE.to_string()]);
    files.sort();
    files.dedup();

    // AI handoff files for the game (owned by the skills work; the operation may not be in
    // this build yet, which is reported and not an error).
    let handoff = match agent.call(
        "handoff_write",
        json!({"path": input.path, "kind": "game", "title": brief.title, "summary": brief.pitch}),
    ) {
        Ok(_) => "written".to_string(),
        Err(e) if e.0.iter().any(|d| d.code == codes::UNKNOWN_OPERATION) => "not available in this build".into(),
        Err(e) => e.0.iter().map(|d| format!("{}: {}", d.code, d.message)).collect::<Vec<_>>().join("; "),
    };

    let tests = game.call("test_run", json!({}))?;
    let passed = tests["passed"].as_u64().unwrap_or(0) as usize;
    let failed = tests["failed"].as_u64().unwrap_or(0) as usize;
    Ok(MakeOutput {
        path: input.path,
        title: brief.title.clone(),
        template: brief.template,
        view: brief.view,
        files,
        assets: results,
        pending,
        generation,
        autoplay: Autoplay {
            won: true,
            seconds: (won_tick as f64 / 60.0 * 10.0).round() / 10.0,
            seed,
            tries: tried,
        },
        tests_passed: passed,
        tests_failed: failed,
        handoff,
    })
}

/// A short line from a generate_run report (counts, not the whole report).
fn compact(report: &Value) -> String {
    let mut text = report.to_string();
    if text.len() > 600 {
        text.truncate(600);
        text.push_str("...");
    }
    text
}

fn generated_file(dir: &Path, slot: &str, kind: SlotKind) -> Option<String> {
    let ext = if matches!(kind, SlotKind::Model { .. }) { "glb" } else { "png" };
    let rel = format!("assets/generated/{slot}.{ext}");
    dir.join(&rel).is_file().then_some(rel)
}

/// Art for every slot through the lookup order: the brief's `use`, a built recipe output,
/// then project files and the CC0 pool (with the slot's `search` words); otherwise a
/// placeholder and the slot's generation recipes.
fn resolve_art(game: &mut Agent, brief: &Brief, files: &mut Vec<String>) -> Result<(ArtMap, Vec<SlotResult>, Vec<String>), Errors> {
    let dir = game.root().to_path_buf();
    let mut art = ArtMap::new();
    let mut results = Vec::new();
    let mut pending = Vec::new();
    for slot in template::slots(brief.template, brief.view) {
        let sb = brief.assets.get(slot.name).cloned().unwrap_or_default();
        let model = matches!(slot.kind, SlotKind::Model { .. });
        let mut found: Option<(String, String)> = None;
        if let Some(u) = sb.use_asset.as_deref() {
            if let Some(id) = u.strip_prefix("pool:") {
                let out = game.call("pool_fetch", json!({"id": id}))?;
                let main = out["main"].as_str().ok_or_else(|| bad(format!("pool asset `{id}` has no main file"), "pick another asset"))?;
                found = Some(("brief".into(), main.to_string()));
            } else if dir.join(u).is_file() {
                found = Some(("brief".into(), u.to_string()));
            } else {
                return Err(Diagnostic::error(starter_codes::BAD_BRIEF, format!("assets.{}.use: `{u}` not found", slot.name))
                    .fix("give a path relative to the game folder, or pool:<id>")
                    .into());
            }
        }
        if found.is_none() {
            found = generated_file(&dir, slot.name, slot.kind).map(|f| ("generated".into(), f));
        }
        if found.is_none()
            && let Some(words) = sb.search.as_deref().filter(|w| !w.trim().is_empty())
        {
            let kind = if model { "model" } else { "texture" };
            if let Ok(r) = game.call("asset_search", json!({"query": words, "kind": kind, "limit": 1})) {
                for step in r["steps"].as_array().into_iter().flatten() {
                    let Some(hit) = step["results"].as_array().and_then(|a| a.first()) else { continue };
                    let id = hit["id"].as_str().unwrap_or_default();
                    match step["name"].as_str() {
                        Some("project_files") if !id.starts_with("assets/generated/") => {
                            found = Some(("project".into(), id.to_string()));
                        }
                        Some("cc0_pool") => {
                            let out = game.call("pool_fetch", json!({"id": id}))?;
                            if let Some(main) = out["main"].as_str() {
                                found = Some(("pool".into(), main.to_string()));
                            }
                        }
                        _ => {}
                    }
                    if found.is_some() {
                        break;
                    }
                }
            }
        }
        let a = match &found {
            Some((_, file)) => {
                let from_scene = format!("../{file}");
                if model {
                    Some(Art::Model {
                        asset: from_scene,
                        scale: 1.0,
                    })
                } else {
                    std::fs::read(dir.join(file)).ok().and_then(|bytes| marionette_starter::sprite_art(&bytes, from_scene))
                }
            }
            None => None,
        };
        match (a, found) {
            (Some(a), Some((source, file))) => {
                art.insert(slot.name.to_string(), a);
                results.push(SlotResult {
                    slot: slot.name.into(),
                    source,
                    file: Some(file),
                });
            }
            _ => {
                // Recipes are rewritten from the brief each time, so a changed description
                // becomes a new request; outputs already built are never touched.
                for (name, recipe) in template::recipes(brief, &slot) {
                    let path = format!("assets/generated/{name}.recipe.json");
                    game.write_file(&path, pretty(&recipe).as_bytes())?;
                    files.push(path);
                }
                pending.push(slot.name.to_string());
                results.push(SlotResult {
                    slot: slot.name.into(),
                    source: "placeholder".into(),
                    file: None,
                });
            }
        }
    }
    Ok((art, results, pending))
}

/// Writes the script (or replaces its config block), the scene recipe, and rebuilds the scene.
fn build(game: &mut Agent, brief: &Brief, art: &ArtMap, map: &[String], refresh: bool) -> Result<(), Errors> {
    let config = template::config(brief, art, map);
    let script_path = game.root().join(template::SCRIPT);
    let script = if refresh && script_path.is_file() {
        let old = std::fs::read_to_string(&script_path).map_err(|e| bad(format!("cannot read the script: {e}"), "check the file"))?;
        template::replace_config(&old, &template::config_block(&config)).ok_or_else(|| {
            Errors::from(
                Diagnostic::error(
                    starter_codes::NO_CONFIG_BLOCK,
                    format!("{} has no `{}` ... `{}` block", template::SCRIPT, template::CONFIG_BEGIN, template::CONFIG_END),
                )
                .file(template::SCRIPT)
                .fix("put the markers back around `const CONFIG`, or delete the script to get a fresh one"),
            )
        })?
    } else {
        template::script(brief, &config)
    };
    game.write_file(template::SCRIPT, script.as_bytes())?;
    let mut ops = template::build_ops(brief, map);
    // A person's own scene operations run after the template's.
    let custom = game.root().join("custom.ops.json");
    if custom.is_file() {
        let text = std::fs::read_to_string(&custom).map_err(|e| bad(format!("custom.ops.json: {e}"), "check the file"))?;
        let extra: Value = serde_json::from_str(&text).map_err(|e| bad(format!("custom.ops.json: {e}"), "format marionette.ops"))?;
        let list = ops["ops"].as_array_mut().expect("ops list");
        let validate = list.pop();
        list.extend(extra["ops"].as_array().cloned().unwrap_or_default());
        list.extend(validate);
    }
    game.write_file("build.ops.json", pretty(&ops).as_bytes())?;
    let scene = game.root().join(template::SCENE);
    if scene.exists() {
        std::fs::remove_file(&scene).map_err(|e| bad(format!("cannot replace the scene: {e}"), "check the file"))?;
    }
    game.call("batch_run", json!({"file": "build.ops.json"}))?;
    Ok(())
}

fn max_ticks(brief: &Brief, map: &[String]) -> u64 {
    match brief.template {
        // Three minutes is far more than any maze needs.
        Template::Maze => 60 * 180,
        Template::Platformer => {
            let cols = map.first().map_or(0, |r| r.len()) as f64;
            ((cols * 0.6 + 20.0).round() as u64 + 2) * 60
        }
    }
}

fn game_state(sim: &mut marionette_core::Sim) -> Value {
    let scene = sim.snapshot();
    scene
        .entities
        .iter()
        .find(|e| e.id == template::GAME_ENTITY)
        .and_then(|e| e.components.get("ScriptState"))
        .and_then(|s| s["data"].as_str())
        .and_then(|d| serde_json::from_str(d).ok())
        .unwrap_or(Value::Null)
}

/// Lets the autopilot play; returns the tick it won on, or None if it lost or ran out.
fn autoplay_win(game: &Agent, limit: u64) -> Result<Option<u64>, Errors> {
    let mut sim = crate::project::load_sim(game, template::SCENE, 1)?;
    crate::engine::feed(game, &mut sim, Some("tests/autoplay.inputs.json"), &[])?;
    let mut ticks = 0;
    while ticks < limit {
        sim.run(300);
        ticks += 300;
        let s = game_state(&mut sim);
        match s["result"].as_str() {
            Some("won") => return Ok(s["wonTick"].as_u64()),
            Some("lost") => return Ok(None),
            _ => {}
        }
    }
    Ok(None)
}

fn run_hash(game: &Agent, ticks: u64, input: Option<&str>) -> Result<String, Errors> {
    let mut sim = crate::project::load_sim(game, template::SCENE, 1)?;
    crate::engine::feed(game, &mut sim, input, &[])?;
    sim.run(ticks);
    Ok(sim.state_hash())
}
