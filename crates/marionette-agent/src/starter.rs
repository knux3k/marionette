//! The game starter operations (decision 0150): `game_interview` lists the questions to ask
//! the person before making a game from a prompt (decision 0151); `game_brief` turns a prompt
//! (and the answers) into a brief;
//! `game_make` turns a brief into a playable, tested project. Templates, levels and the brief
//! format live in `marionette-starter`; this file resolves assets through the lookup order,
//! writes the files, builds the scene, and lets the autopilot prove the level can be won.

use crate::{Agent, Op, codes, op};
use marionette_core::{Diagnostic, Errors};
use marionette_starter::interview::{self, Answers, Question};
use marionette_starter::template::{self, SlotKind};
use marionette_starter::{Art, ArtMap, Brief, Template, View, codes as starter_codes};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;

pub(crate) fn ops() -> Vec<Op> {
    vec![
        op(
            "game_interview",
            "Step one of making a game from an idea: returns the questions to ask the person (genre, view, setting, player, pickups, enemy, goal, art style, difficulty, length, title, anything else), each with choices or examples and the guess used if unanswered. Ask the person the open ones, call again with `answers` to see the guesses follow, then pass the same `answers` to game_brief or game_make. Writes nothing.",
            false,
            interview_op,
        ),
        op(
            "game_brief",
            "Turns a one-line game idea (plus game_interview `answers`) into a game brief (marionette.brief/1: template, title, style, colors, one description per asset slot) without writing anything, or checks a brief you wrote. Read it, improve it (you write better briefs than the keyword reader), then pass it to game_make.",
            false,
            brief_op,
        ),
        op(
            "game_make",
            "Makes a playable game project from a brief or from a prompt plus the person's game_interview `answers` (a bare prompt is refused unless `skip_interview` says the person declined the questions): level from the brief's seed, the template's TypeScript game script, scene recipe, input map, generation recipes for art not found by the asset lookup order (project files, CC0 pool), and tests. The template's autopilot must win the level (other seeds are tried if not), and doing nothing must not. `refresh` re-reads the project's brief.json, swaps in art built since, keeps edits outside the script's CONFIG block and re-pins the tests. `generate` runs generate_run for missing art first.",
            true,
            make_op,
        ),
    ]
}

fn bad(msg: impl Into<String>, fix: impl Into<String>) -> Errors {
    Diagnostic::error(codes::BAD_INPUT, msg.into())
        .fix(fix.into())
        .into()
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
    /// The person's replies to game_interview, by question id.
    #[serde(default)]
    answers: Answers,
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

fn brief_from(
    prompt_text: Option<&str>,
    brief: Option<Brief>,
    answers: &Answers,
    t: Option<Template>,
    v: Option<View>,
) -> Result<Brief, Errors> {
    let b = match (brief, prompt_text) {
        (Some(b), _) => b,
        (None, Some(p)) if !p.trim().is_empty() => interview::brief_from_answers(p, answers, t, v)?,
        _ => {
            return Err(bad(
                "give a `prompt` or a `brief`",
                "for example {\"prompt\": \"a 3d maze about a moth collecting lanterns\"}",
            ));
        }
    };
    b.check()?;
    Ok(b)
}

fn brief_op(_agent: &mut Agent, input: BriefInput) -> Result<BriefOutput, Errors> {
    let brief = brief_from(
        input.prompt.as_deref(),
        input.brief,
        &input.answers,
        input.template,
        input.view,
    )?;
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
struct InterviewInput {
    /// The game idea in the person's words.
    prompt: String,
    /// Replies so far, by question id.
    #[serde(default)]
    answers: Answers,
    /// A template already decided (its question is then not asked).
    #[serde(default)]
    template: Option<Template>,
    /// A view already decided.
    #[serde(default)]
    view: Option<View>,
    /// Also list answered questions (default: only the open ones).
    #[serde(default)]
    all: bool,
}

#[derive(Serialize, JsonSchema)]
struct InterviewOutput {
    /// Questions to ask, in order (open ones only unless `all`).
    questions: Vec<Question>,
    /// Questions answered so far.
    answered: usize,
    /// What the starter cannot do for this idea, to raise with the person.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    notes: Vec<String>,
    /// What to do next.
    next: &'static str,
}

fn interview_op(_agent: &mut Agent, input: InterviewInput) -> Result<InterviewOutput, Errors> {
    if input.prompt.trim().is_empty() {
        return Err(bad(
            "`prompt` is empty",
            "give the game idea in the person's words",
        ));
    }
    let (questions, notes) = interview::questions(&input.prompt, &input.answers, input.template, input.view)?;
    let answered = questions.iter().filter(|q| q.answer.is_some()).count();
    let questions = if input.all {
        questions
    } else {
        questions.into_iter().filter(|q| q.answer.is_none()).collect()
    };
    Ok(InterviewOutput {
        next: if questions.is_empty() {
            "All answered: call game_make with `path`, `prompt` and these `answers` (or game_brief to review the brief first)."
        } else {
            "Ask the person these questions (several at once is fine; offer the guess as the default), then call game_make with `path`, `prompt` and their `answers`. Unanswered questions keep their guess."
        },
        questions,
        answered,
        notes,
    })
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
    /// The person's replies to game_interview, by question id. Needed with a `prompt` (or
    /// `skip_interview`); recorded in the project's interview.json.
    #[serde(default)]
    answers: Answers,
    /// The person asked to build from the prompt without questions: every question takes its
    /// guess.
    #[serde(default)]
    skip_interview: bool,
    /// Re-make an existing starter project from its brief.json (or from `brief`).
    #[serde(default)]
    refresh: bool,
    /// Run generate_run for slots still waiting for art (paid providers follow the project's
    /// providers.json, keys and spend caps), then use what was built.
    #[serde(default)]
    generate: bool,
    /// With `refresh`: replace the script's CONFIG block even though it was edited by hand
    /// (the old block is saved to `.config.backup`). Without it, a hand-edited block stops the
    /// refresh.
    #[serde(default)]
    overwrite_config: bool,
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
    /// AI handoff files and skills for agents working on the game: per operation, `written`
    /// or why not.
    handoff: Vec<String>,
    /// What the person asked for beyond the template (interview question `extra`): build it
    /// as game code next.
    #[serde(skip_serializing_if = "Option::is_none")]
    wishes: Option<String>,
}

const BRIEF_FILE: &str = "brief.json";
/// The CONFIG block the last build wrote, to tell a hand edit from the starter's own output.
const GENERATED_CONFIG: &str = ".config.generated";
const CONFIG_BACKUP: &str = ".config.backup";
const INTERVIEW_FILE: &str = "interview.json";

fn pretty(v: &impl Serialize) -> Result<String, Errors> {
    crate::json_text(v)
}

fn make_op(agent: &mut Agent, input: MakeInput) -> Result<MakeOutput, Errors> {
    let dir = agent.resolve(&input.path)?;
    let has_files = dir.is_dir()
        && std::fs::read_dir(&dir)
            .map(|mut d| d.next().is_some())
            .unwrap_or(false);
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
                    bad(
                        format!("cannot read {}/{BRIEF_FILE}: {e}", input.path),
                        "refresh needs the project's brief.json",
                    )
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
            return Err(
                Diagnostic::error(codes::EXISTS, format!("`{}` already has files", input.path))
                    .fix("pick a new folder, or pass `refresh: true` to re-make a starter project")
                    .into(),
            );
        }
        if input.brief.is_none() && input.answers.is_empty() && !input.skip_interview {
            return Err(Diagnostic::error(
                starter_codes::INTERVIEW_NEEDED,
                "a game from a bare prompt is the keyword reader's guess; interview the person first",
            )
            .fix("call game_interview with the prompt, ask the person its questions, then pass their replies as `answers`; set `skip_interview: true` only if the person asked to build without questions")
            .into());
        }
        brief_from(
            input.prompt.as_deref(),
            input.brief,
            &input.answers,
            input.template,
            input.view,
        )?
    };
    let wishes = input
        .answers
        .get("extra")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    brief.check()?;
    if agent.read_only() {
        return Err(Diagnostic::error(codes::READ_ONLY, "game_make writes files").into());
    }
    std::fs::create_dir_all(&dir)
        .map_err(|e| bad(format!("cannot create `{}`: {e}", input.path), "check the path"))?;
    let mut game = Agent::with_engine(&dir, false, agent.engine().clone());
    let mut files = Vec::new();
    let write = |game: &Agent, path: &str, text: String, files: &mut Vec<String>| -> Result<(), Errors> {
        game.write_file(path, text.as_bytes())?;
        files.push(path.to_string());
        Ok(())
    };

    if !dir.join("marionette.json").exists() {
        let name = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "game".into());
        write(
            &game,
            "marionette.json",
            pretty(&json!({"format": "marionette.project", "name": name, "version": 1}))?,
            &mut files,
        )?;
    }
    if !dir.join("providers.json").exists() {
        write(
            &game,
            "providers.json",
            pretty(
                &json!({"format": "marionette.providers/1", "image": {"provider": "qwen"}, "model3d": {"provider": "tripo"}}),
            )?,
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

    write(
        &game,
        "scenes/main.actions.json",
        pretty(&template::actions(brief.template))?,
        &mut files,
    )?;
    write(
        &game,
        "tests/autoplay.inputs.json",
        pretty(&template::autoplay_inputs())?,
        &mut files,
    )?;

    // The person's own answers, so later agents know what was asked for and why.
    if !input.answers.is_empty() {
        write(
            &game,
            INTERVIEW_FILE,
            pretty(&json!({
                "format": "marionette.interview/1",
                "prompt": input.prompt,
                "answers": input.answers,
            }))?,
            &mut files,
        )?;
    }

    // The brief goes in first, so a project whose level could not be won can still be
    // fixed and refreshed.
    write(&game, BRIEF_FILE, pretty(&brief)?, &mut files)?;

    // Level: the brief's seed first; if the autopilot cannot win it, the next seeds.
    let tries = input.tries.unwrap_or(8).clamp(1, 64);
    let mut outcome = None;
    let mut final_map: Vec<String> = Vec::new();
    let mut seed = brief.seed;
    for attempt in 0..tries {
        seed = brief.seed + attempt as u64;
        let map = template::level(&brief, seed);
        build(
            &mut game,
            &brief,
            &art,
            &map,
            input.refresh,
            input.overwrite_config,
        )?;
        let limit = max_ticks(&brief, &map);
        if let Some(t) = autoplay_win(&game, limit)? {
            outcome = Some((t, attempt + 1));
            final_map = map;
            break;
        }
    }
    let Some((won_tick, tried)) = outcome else {
        return Err(Diagnostic::error(
            starter_codes::UNWINNABLE,
            format!(
                "the autopilot could not win any of {tries} levels (seeds {} to {seed})",
                brief.seed
            ),
        )
        .fix("try another seed or an easier difficulty in the brief, or raise `tries`")
        .into());
    };
    let mut brief = brief;
    brief.seed = seed;

    // Pin the tests to what this build does.
    let auto_ticks = won_tick + 30;
    let auto_hash = run_hash(&game, auto_ticks, Some("tests/autoplay.inputs.json"))?;
    write(
        &game,
        "tests/autoplay.test.json",
        pretty(&template::autoplay_test(auto_ticks, &auto_hash))?,
        &mut files,
    )?;
    let idle_hash = run_hash(&game, template::IDLE_TICKS, None)?;
    write(
        &game,
        "tests/idle.test.json",
        pretty(&template::idle_test(template::IDLE_TICKS, &idle_hash))?,
        &mut files,
    )?;
    write(&game, BRIEF_FILE, pretty(&brief)?, &mut files)?;
    write(
        &game,
        "README.md",
        template::readme(&brief, &final_map),
        &mut files,
    )?;
    files.extend([
        template::SCRIPT.to_string(),
        "build.ops.json".into(),
        template::SCENE.to_string(),
    ]);
    files.sort();
    files.dedup();

    // AI handoff files and skills for agents working on the game (the handoff operations of
    // the skills work, decisions 0160 and 0161). A build without them reports it; that is not
    // an error. Existing handoff files are never overwritten.
    let mut notes = vec![
        "`brief.json`: the whole design (template, words, colors, one line per asset slot, seed). Change it, then run game_make with `refresh`.".to_string(),
        "`scenes/scripts/game.ts`: the game. game_make rewrites only its CONFIG block; the rest is yours.".to_string(),
        "`build.ops.json`: the scene recipe, rebuilt by game_make. Own scene operations go in `custom.ops.json`.".to_string(),
        "`assets/generated/*.recipe.json`: art still to generate (`marionette generate run`, then game_make `refresh`).".to_string(),
        "`tests/autoplay.test.json` (the autopilot wins) and `tests/idle.test.json` (doing nothing does not win); refresh re-pins them.".to_string(),
    ];
    if !input.answers.is_empty() {
        notes.push("`interview.json`: the person's prompt and answers to the game interview; check changes against it.".into());
    }
    if let Some(w) = &wishes {
        notes.push(format!("TODO from the interview, not built by the template: {w}"));
    }
    let mut handoff = Vec::new();
    for (name, input_json) in [
        (
            "handoff_init",
            json!({"path": input.path, "title": brief.title, "summary": brief.pitch, "notes": notes}),
        ),
        ("skills_install", json!({"path": input.path})),
    ] {
        handoff.push(match agent.call(name, input_json) {
            Ok(_) => format!("{name}: written"),
            Err(e) if e.0.iter().any(|d| d.code == codes::UNKNOWN_OPERATION) => {
                format!("{name}: not available in this build")
            }
            Err(e) => format!(
                "{name}: {}",
                e.0.iter()
                    .map(|d| format!("{}: {}", d.code, d.message))
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
        });
    }

    // The handoff template's STATE.md is generic; say what this game really is. A fresh
    // STATE.md gets the template's rows replaced; a refresh only rewrites the Game row it
    // wrote before, so the counts never go stale and hand edits elsewhere stay.
    let state = game.root().join("docs/ai/STATE.md");
    if let Ok(text) = std::fs::read_to_string(&state) {
        let count = |marks: &[char]| {
            final_map
                .iter()
                .flat_map(|r| r.chars())
                .filter(|c| marks.contains(c))
                .count()
        };
        let enemy_mark = if brief.template == Template::Maze {
            'G'
        } else {
            'e'
        };
        let enemies = count(&[enemy_mark]);
        let game_row = format!(
            "| Game | `{}` ({} {}): {} {}, {} {}, seed {seed}; the autopilot wins in {:.1} s. |",
            brief.title,
            brief.template.name(),
            if brief.view == View::ThreeD { "3d" } else { "2d" },
            count(&['*']),
            brief.words.pickups,
            enemies,
            if enemies == 1 { "enemy" } else { "enemies" },
            autoplay_seconds(won_tick),
        );
        let mut text = if input.refresh {
            text.lines()
                .map(|l| {
                    if l.starts_with("| Game |") {
                        game_row.as_str()
                    } else {
                        l
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
                + "\n"
        } else {
            text.replace(
                "| Scenes | `scenes/main.scene.json` from the project template. |",
                &game_row,
            )
            .replace(
                "| Tests | `tests/main.test.json` passes. |",
                "| Tests | `tests/autoplay.test.json` (the autopilot wins) and `tests/idle.test.json` (doing nothing does not win); `game_make` with `refresh` re-pins both. |",
            )
        };
        if !input.refresh && !pending.is_empty() {
            text = text.replacen(
                "- Nothing yet.",
                "- Some art is still a placeholder shape (`assets/generated/*.recipe.json`); check by running `marionette generate run` and then `game_make` with `refresh`.",
                1,
            );
        }
        // A failed write leaves the older text, which is only stale, not wrong.
        let _ = std::fs::write(&state, text);
    }

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
            seconds: autoplay_seconds(won_tick),
            seed,
            tries: tried,
        },
        tests_passed: passed,
        tests_failed: failed,
        handoff,
        wishes,
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
    let ext = if matches!(kind, SlotKind::Model { .. }) {
        "glb"
    } else {
        "png"
    };
    let rel = format!("assets/generated/{slot}.{ext}");
    dir.join(&rel).is_file().then_some(rel)
}

/// Art for every slot through the lookup order: the brief's `use`, a built recipe output,
/// then project files and the CC0 pool (with the slot's `search` words); otherwise a
/// placeholder and the slot's generation recipes.
fn resolve_art(
    game: &mut Agent,
    brief: &Brief,
    files: &mut Vec<String>,
) -> Result<(ArtMap, Vec<SlotResult>, Vec<String>), Errors> {
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
                let main = out["main"].as_str().ok_or_else(|| {
                    bad(
                        format!("pool asset `{id}` has no main file"),
                        "pick another asset",
                    )
                })?;
                found = Some(("brief".into(), main.to_string()));
            } else if dir.join(u).is_file() {
                found = Some(("brief".into(), u.to_string()));
            } else {
                return Err(Diagnostic::error(
                    starter_codes::BAD_BRIEF,
                    format!("assets.{}.use: `{u}` not found", slot.name),
                )
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
                    let Some(hit) = step["results"].as_array().and_then(|a| a.first()) else {
                        continue;
                    };
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
                    std::fs::read(dir.join(file))
                        .ok()
                        .and_then(|bytes| marionette_starter::sprite_art(&bytes, from_scene))
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
                    game.write_file(&path, pretty(&recipe)?.as_bytes())?;
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
fn build(
    game: &mut Agent,
    brief: &Brief,
    art: &ArtMap,
    map: &[String],
    refresh: bool,
    overwrite_config: bool,
) -> Result<(), Errors> {
    let config = template::config(brief, art, map);
    let script_path = game.root().join(template::SCRIPT);
    let script = if refresh && script_path.is_file() {
        let old = std::fs::read_to_string(&script_path)
            .map_err(|e| bad(format!("cannot read the script: {e}"), "check the file"))?;
        // The block the last build wrote is kept beside the project; a different block now
        // means someone edited it, and this refresh would silently erase that.
        if let (Some(block), Ok(written)) = (
            template::current_config(&old),
            std::fs::read_to_string(game.root().join(GENERATED_CONFIG)),
        ) && block != written
        {
            if !overwrite_config {
                return Err(Diagnostic::error(
                    starter_codes::CONFIG_EDITED,
                    format!("the CONFIG block in {} was edited by hand; a refresh would erase the edit", template::SCRIPT),
                )
                .file(template::SCRIPT)
                .fix("put the change in brief.json instead (pickup_count, enemy_count, words, size, seed), or move the edit outside the CONFIG block; or pass `overwrite_config: true` to replace the block and keep the old one in .config.backup")
                .into());
            }
            game.write_file(CONFIG_BACKUP, block.as_bytes())?;
        }
        template::replace_config(&old, &template::config_block(&config)).ok_or_else(|| {
            Errors::from(
                Diagnostic::error(
                    starter_codes::NO_CONFIG_BLOCK,
                    format!(
                        "{} has no `{}` ... `{}` block",
                        template::SCRIPT,
                        template::CONFIG_BEGIN,
                        template::CONFIG_END
                    ),
                )
                .file(template::SCRIPT)
                .fix("put the markers back around `const CONFIG`, or delete the script to get a fresh one"),
            )
        })?
    } else {
        template::script(brief, &config)
    };
    game.write_file(template::SCRIPT, script.as_bytes())?;
    game.write_file(GENERATED_CONFIG, template::config_block(&config).as_bytes())?;
    let mut ops = template::build_ops(brief, map);
    // A person's own scene operations run after the template's.
    let custom = game.root().join("custom.ops.json");
    if custom.is_file() {
        let text = std::fs::read_to_string(&custom)
            .map_err(|e| bad(format!("custom.ops.json: {e}"), "check the file"))?;
        let extra: Value = serde_json::from_str(&text)
            .map_err(|e| bad(format!("custom.ops.json: {e}"), "format marionette.ops"))?;
        if let Some(list) = ops["ops"].as_array_mut() {
            let validate = list.pop();
            list.extend(extra["ops"].as_array().cloned().unwrap_or_default());
            list.extend(validate);
        }
    }
    game.write_file("build.ops.json", pretty(&ops)?.as_bytes())?;
    let scene = game.root().join(template::SCENE);
    if scene.exists() {
        std::fs::remove_file(&scene)
            .map_err(|e| bad(format!("cannot replace the scene: {e}"), "check the file"))?;
    }
    game.call("batch_run", json!({"file": "build.ops.json"}))?;
    Ok(())
}

/// Seconds the autopilot needed, to one decimal (the same number in the output and STATE.md).
fn autoplay_seconds(won_tick: u64) -> f64 {
    (won_tick as f64 / 60.0 * 10.0).round() / 10.0
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
