//! Skills stay true (decision 0160). A skill is a procedure an agent follows without checking
//! it, so every name in one is checked here against the engine: operations, CLI commands and
//! flags, component and resource types, repository paths, quoted doc headings, other skills.
//! Sizes are capped because every description is in every agent's context all the time
//! (focus 2). When this fails, fix the skill (or the registry line in
//! `crates/marionette-agent/src/skills.rs`), not the test.

use marionette_agent::Agent;
use marionette_agent::skills::{self, Skill};
use serde_json::json;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Characters in one description (always in context).
const DESCRIPTION_LIMIT: usize = 400;
/// Bytes of all descriptions together (about 1,500 tokens).
const ALL_DESCRIPTIONS_LIMIT: usize = 6000;
/// Bytes of one body (loaded only when the task matches).
const BODY_LIMIT: usize = 4000;

/// Backticked words that look like an operation (`group_word`) but are not.
const NOT_OPERATIONS: &[&str] = &["script_error"];
/// Backticked capitalized words that are not scene types: Rust and TypeScript names the
/// skills use on purpose.
const NOT_TYPES: &[&str] = &[
    "Agent",
    "Clone",
    "Component",
    "Debug",
    "Entity",
    "Errors",
    "Float32Array",
    "Float64Array",
    "In",
    "Int32Array",
    "JsonSchema",
    "Math",
    "Out",
    "PartialEq",
    "Reflect",
    "Resource",
];
/// Paths a game project has too: its own stability suite at the default path, and the
/// handoff files `project_init` writes (checked in `games_get_the_skills_and_handoff_files`).
const GAME_PATHS: &[&str] = &[
    "stability/suite.json",
    "docs/ai/START-HERE.md",
    "docs/ai/STATE.md",
    "docs/ai/claims/",
    "docs/ai/sessions/",
    "docs/decisions/",
];
/// First path segments that mean "a path in the engine repository".
const REPO_ROOTS: &[&str] = &[
    "crates/",
    "docs/",
    "examples/",
    "stability/",
    "games/",
    "pool/",
    ".claude/",
    ".github/",
];

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// What the engine really has.
struct Engine {
    ops: BTreeSet<String>,
    groups: BTreeSet<String>,
    types: BTreeSet<String>,
    cli: clap::Command,
}

fn engine() -> Engine {
    let mut agent = Agent::new(repo(), true);
    let ops: BTreeSet<String> = agent.operations().map(|o| o.name.to_string()).collect();
    let groups = ops
        .iter()
        .map(|o| o.split('_').next().unwrap().to_string())
        .collect();
    let schema = agent.call("project_schema", json!({})).unwrap();
    let types = ["components", "resources"]
        .iter()
        .flat_map(|k| schema[k].as_array().unwrap().iter())
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    Engine {
        ops,
        groups,
        types,
        cli: marionette_cli::command(),
    }
}

/// Inline code spans and the lines of fenced code blocks of a Markdown text.
fn code(text: &str) -> (Vec<String>, Vec<String>) {
    let (mut spans, mut lines, mut fenced) = (Vec::new(), Vec::new(), false);
    let mut prose = String::new();
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        } else if fenced {
            lines.push(line.trim().to_string());
        } else {
            prose.push_str(line.trim());
            prose.push(' ');
        }
    }
    assert!(!fenced, "unclosed code fence");
    // Spans may wrap over lines, so they are cut from the joined prose.
    let parts: Vec<&str> = prose.split('`').collect();
    assert!(parts.len() % 2 == 1, "unpaired backtick");
    for span in parts.iter().skip(1).step_by(2) {
        spans.push(span.to_string());
    }
    (spans, lines)
}

/// Checks one command line (`marionette <command> ...`) against the CLI definition.
fn check_command(e: &Engine, text: &str, problems: &mut Vec<String>) {
    let rest = if let Some(r) = text.strip_prefix("marionette ") {
        r
    } else if let Some(i) = text.find("-p marionette-cli -- ") {
        &text[i + "-p marionette-cli -- ".len()..]
    } else {
        return;
    };
    let tokens: Vec<&str> = rest.split_whitespace().collect();
    let mut cmd = &e.cli;
    let mut i = 0;
    // Walk nested subcommands (`generate approve`, `scene fmt`).
    while let Some(tok) = tokens.get(i) {
        match cmd.find_subcommand(tok) {
            Some(sub) => {
                cmd = sub;
                i += 1;
            }
            None => break,
        }
    }
    if i == 0 {
        if tokens.first().is_some_and(|t| !t.starts_with('-')) {
            problems.push(format!("`{text}`: no CLI command `{}`", tokens[0]));
        }
        return;
    }
    if cmd.get_name() == "op"
        && let Some(name) = tokens.get(i)
        && !name.starts_with('<')
        && !e.ops.contains(*name)
    {
        problems.push(format!("`{text}`: no operation `{name}`"));
    }
    for flag in tokens[i..].iter().filter_map(|t| t.strip_prefix("--")) {
        let flag = flag.trim_end_matches(['.', ',', ';', ')', '\'', '"']);
        let known = cmd
            .get_arguments()
            .chain(e.cli.get_arguments().filter(|a| a.is_global_set()))
            .any(|a| a.get_long() == Some(flag));
        if !flag.is_empty() && !known {
            problems.push(format!(
                "`{text}`: `marionette {}` has no flag `--{flag}`",
                cmd.get_name()
            ));
        }
    }
}

fn is_op_shaped(e: &Engine, s: &str) -> bool {
    s.contains('_')
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && e.groups.contains(s.split('_').next().unwrap())
}

fn is_type_shaped(s: &str) -> bool {
    s.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        && s.chars().all(|c| c.is_ascii_alphanumeric())
        && s.chars().any(|c| c.is_ascii_lowercase())
}

fn problems_of(e: &Engine, skill: &Skill, names: &BTreeSet<&str>) -> Vec<String> {
    let mut problems = Vec::new();
    let root = repo();
    let text = skill.text.replace("\r\n", "\n");
    let body = skills::body(&text);

    // Front matter and sizes.
    if skills::front_matter(&text, "name") != Some(skill.name) {
        problems.push("front matter `name` must equal the folder name".into());
    }
    let d = skill.description();
    if d.is_empty() || d.chars().count() > DESCRIPTION_LIMIT {
        problems.push(format!(
            "description is {} characters (1 to {DESCRIPTION_LIMIT})",
            d.chars().count()
        ));
    }
    if !d.contains("Use when") {
        problems.push("description must say `Use when ...` (it decides when the skill loads)".into());
    }
    if d.contains('<') || d.contains('>') || d.contains(": ") || d.contains(" #") {
        problems
            .push("description must be a plain one-line YAML value: no angle brackets, `: ` or ` #`".into());
    }
    if body.len() > BODY_LIMIT {
        problems.push(format!(
            "body is {} bytes, limit {BODY_LIMIT}: split the skill",
            body.len()
        ));
    }
    let name_ok = skill.name.len() <= 64
        && !skill.name.starts_with('-')
        && !skill.name.ends_with('-')
        && !skill.name.contains("--")
        && skill
            .name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !name_ok {
        problems.push("name must be lowercase letters, digits and single hyphens, at most 64".into());
    }

    // Everything in backticks.
    let (spans, lines) = code(body);
    for span in &spans {
        check_command(e, span, &mut problems);
        let word = span.trim_end_matches(['.', ',']);
        if is_op_shaped(e, word) && !e.ops.contains(word) && !NOT_OPERATIONS.contains(&word) {
            problems.push(format!("`{word}` looks like an operation but none has that name"));
        }
        if is_type_shaped(word) && !e.types.contains(word) && !NOT_TYPES.contains(&word) {
            problems.push(format!("`{word}` is not a component or resource type"));
        }
        let is_repo_path =
            REPO_ROOTS.iter().any(|r| word.starts_with(r)) || ["CLAUDE.md", "AGENTS.md"].contains(&word);
        if is_repo_path && !word.contains(' ') {
            if skill.audience() == "game" {
                if GAME_PATHS.contains(&word) {
                    continue;
                }
                problems.push(format!(
                    "`{word}` is an engine repository path; a game skill is installed where it does not exist"
                ));
            } else if !word.contains('<') && !root.join(word).exists() {
                problems.push(format!("`{word}` does not exist"));
            }
        }
    }
    for line in &lines {
        check_command(e, line, &mut problems);
    }

    // `docs/ai/X.md` "Heading": the heading exists in that doc.
    let flat: String = body.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut rest = flat.as_str();
    while let Some(i) = rest.find(".md` \"") {
        let doc = rest[..i + 3].rsplit('`').next().unwrap();
        let after = &rest[i + 6..];
        let Some(end) = after.find('"') else { break };
        let heading = &after[..end];
        let found = std::fs::read_to_string(root.join(doc)).is_ok_and(|t| {
            t.lines()
                .any(|l| l.starts_with('#') && l.trim_start_matches('#').trim() == heading)
        });
        if !found {
            problems.push(format!("`{doc}` has no heading \"{heading}\""));
        }
        rest = &after[end..];
    }

    // skill `name`: the other skill exists.
    let mut rest = flat.as_str();
    while let Some(i) = rest.find("skill `") {
        let after = &rest[i + 7..];
        let Some(end) = after.find('`') else { break };
        if !names.contains(&after[..end]) {
            problems.push(format!("no skill named `{}`", &after[..end]));
        }
        rest = &after[end..];
    }
    problems
}

#[test]
fn skills_name_only_things_that_exist() {
    let e = engine();
    let names: BTreeSet<&str> = skills::ALL.iter().map(|s| s.name).collect();
    let mut all = Vec::new();
    for skill in skills::ALL.iter().filter(|s| s.compiled_in()) {
        for p in problems_of(&e, skill, &names) {
            all.push(format!("{}: {p}", skill.name));
        }
    }
    assert!(all.is_empty(), "stale or malformed skills:\n{}", all.join("\n"));
}

#[test]
fn the_checker_catches_stale_names() {
    let e = engine();
    let names: BTreeSet<&str> = skills::ALL.iter().map(|s| s.name).collect();
    let stale = |body: &str, name: &'static str| {
        let text =
            format!("---\nname: {name}\ndescription: Does a thing. Use when testing.\n---\n\n{body}\n");
        let skill = Skill {
            name,
            text: Box::leak(text.into_boxed_str()),
            feature: None,
        };
        problems_of(&e, &skill, &names)
    };
    for (body, name, want) in [
        ("Run `entity_destroy`.", "marionette-x", "looks like an operation"),
        (
            "Run `marionette op scene_explode '{}'`.",
            "marionette-x",
            "no operation",
        ),
        ("Run `marionette fly --fast`.", "marionette-x", "no CLI command"),
        (
            "Run `marionette run a.scene.json --turbo`.",
            "marionette-x",
            "no flag",
        ),
        (
            "```bash\ncargo run -q -p marionette-cli -- stability --warp\n```",
            "engine-x",
            "no flag",
        ),
        ("Add a `Hologram`.", "marionette-x", "not a component"),
        ("See `docs/ai/GONE.md`.", "engine-x", "does not exist"),
        (
            "See `docs/ai/HOWTO.md` \"Add a teleporter\".",
            "engine-x",
            "no heading",
        ),
        (
            "See `docs/ai/HOWTO.md`.",
            "marionette-x",
            "engine repository path",
        ),
        (
            "Then skill `marionette-nothing`.",
            "marionette-x",
            "no skill named",
        ),
    ] {
        let got = stale(body, name);
        assert!(
            got.iter().any(|p| p.contains(want)),
            "`{body}` should report `{want}`, got {got:?}"
        );
    }
    // And a true one passes.
    let fine = stale(
        "Run `marionette op scene_run '{}' --json`, set `Bloom`, see skill `engine-add-skill`.",
        "marionette-x",
    );
    assert!(fine.is_empty(), "{fine:?}");
}

#[test]
fn registry_index_and_folders_agree() {
    let root = repo();
    let mut folders: Vec<String> = std::fs::read_dir(root.join(".claude/skills"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    folders.sort();
    let registered: Vec<&str> = skills::ALL.iter().map(|s| s.name).collect();
    assert_eq!(
        folders, registered,
        "every folder in .claude/skills needs a line in crates/marionette-agent/src/skills.rs (sorted)"
    );
    let index = std::fs::read_to_string(root.join("AGENTS.md")).unwrap();
    for name in &registered {
        assert!(
            index.contains(&format!("`{name}`")),
            "AGENTS.md must list `{name}`"
        );
    }
    let total: usize = skills::ALL.iter().map(|s| s.description().len()).sum();
    assert!(
        total <= ALL_DESCRIPTIONS_LIMIT,
        "all descriptions are {total} bytes, limit {ALL_DESCRIPTIONS_LIMIT}: they are in every agent's context"
    );
}

#[test]
fn games_get_the_skills_and_handoff_files() {
    let dir = marionette_host::temp_dir("skills-test").unwrap();
    let mut agent = Agent::new(dir.path(), false);

    // A new project comes with them.
    agent.call("project_init", json!({ "path": "game" })).unwrap();
    for folder in [".claude/skills", ".agents/skills"] {
        let file = dir
            .path()
            .join("game")
            .join(folder)
            .join("marionette-build-scene/SKILL.md");
        assert!(file.exists(), "{}", file.display());
        assert!(
            !dir.path()
                .join("game")
                .join(folder)
                .join("engine-add-skill")
                .exists(),
            "engine skills are not installed into games"
        );
    }

    for path in GAME_PATHS.iter().filter(|p| p.starts_with("docs/")) {
        assert!(
            dir.path().join("game").join(path).exists(),
            "project_init must write {path}"
        );
    }

    // An existing project installs or refreshes them, and keeps its own skills.
    let own = dir.path().join(".claude/skills/my-game-rules/SKILL.md");
    std::fs::create_dir_all(own.parent().unwrap()).unwrap();
    std::fs::write(&own, "mine").unwrap();
    let out = agent
        .call("skills_install", json!({ "targets": ["claude"] }))
        .unwrap();
    assert_eq!(out["folders"], json!([".claude/skills"]));
    let installed = dir.path().join(".claude/skills/marionette-write-script/SKILL.md");
    let expected = skills::ALL
        .iter()
        .find(|s| s.name == "marionette-write-script")
        .unwrap();
    assert_eq!(std::fs::read_to_string(installed).unwrap(), expected.text);
    assert_eq!(std::fs::read_to_string(&own).unwrap(), "mine");
    assert!(!dir.path().join(".agents").exists());
    assert!(
        agent
            .call("skills_install", json!({ "targets": ["nobody"] }))
            .is_err()
    );

    // Agents without skill support read them through operations (CLI or MCP).
    let list = agent.call("skills_list", json!({})).unwrap();
    let listed = list["skills"].as_array().unwrap();
    assert!(listed.iter().all(|s| s["audience"] == "game"));
    assert_eq!(listed.len(), out["skills"].as_array().unwrap().len());
    let all = agent.call("skills_list", json!({ "engine": true })).unwrap();
    assert_eq!(
        all["skills"].as_array().unwrap().len(),
        skills::ALL.iter().filter(|s| s.compiled_in()).count()
    );
    let one = agent
        .call("skills_get", json!({ "name": "marionette-port-unity-unreal" }))
        .unwrap();
    let text = one["instructions"].as_str().unwrap();
    assert!(text.starts_with("# Port Unity or Unreal code"), "{text}");
    assert!(agent.call("skills_get", json!({ "name": "nope" })).is_err());
}

#[test]
fn handoffs_keep_parallel_agents_apart() {
    let dir = marionette_host::temp_dir("handoff-test").unwrap();
    let mut agent = Agent::new(dir.path(), false);
    let call = |agent: &mut Agent, op: &str, input: serde_json::Value| agent.call(op, input);

    // A project without handoff files is told so, then gets them without losing its own.
    let before = call(&mut agent, "handoff_status", json!({})).unwrap();
    assert_eq!(before["problems"].as_array().unwrap().len(), 2, "{before}");
    std::fs::create_dir_all(dir.path().join("docs/ai")).unwrap();
    std::fs::write(dir.path().join("docs/ai/STATE.md"), "mine").unwrap();
    let init = call(&mut agent, "handoff_init", json!({})).unwrap();
    assert!(
        !init["written"]
            .as_array()
            .unwrap()
            .contains(&json!("docs/ai/STATE.md"))
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("docs/ai/STATE.md")).unwrap(),
        "mine"
    );
    assert_eq!(
        call(&mut agent, "handoff_status", json!({})).unwrap()["problems"],
        json!([])
    );

    // A generated game in a subfolder gets its own lines in a new START-HERE.md.
    call(
        &mut agent,
        "handoff_init",
        json!({ "path": "made", "title": "Lamp Run", "summary": "Carry the lamp home.",
                "notes": ["`brief.json`: the design."] }),
    )
    .unwrap();
    let start = std::fs::read_to_string(dir.path().join("made/docs/ai/START-HERE.md")).unwrap();
    assert!(
        start.ends_with("## This game\n\nLamp Run: Carry the lamp home.\n\n- `brief.json`: the design.\n"),
        "{start}"
    );
    assert!(start.contains("## Every session ends by"));

    // Two agents claim different areas and get different decision blocks.
    std::fs::write(
        dir.path().join("docs/decisions/0003-earlier.md"),
        "# 0003: Earlier\n",
    )
    .unwrap();
    let a = call(
        &mut agent,
        "handoff_claim",
        json!({ "topic": "power-grid", "date": "2026-01-02", "summary": "Power lines carry load.",
                "areas": ["scripts/power", "scenes/power.scene.json"], "decisions": 5 }),
    )
    .unwrap();
    assert_eq!(a["decisions"], json!([4, 8]));
    let b = call(
        &mut agent,
        "handoff_claim",
        json!({ "topic": "drones", "date": "2026-01-02", "summary": "Drones carry items.",
                "areas": ["scripts/drones/"], "decisions": 2 }),
    )
    .unwrap();
    assert_eq!(b["decisions"], json!([9, 10]));

    // The same area, a file inside a claimed folder, and the same topic are refused.
    for (topic, area) in [("wires", "scripts/power/wires.ts"), ("all-scripts", "scripts")] {
        let err = call(
            &mut agent,
            "handoff_claim",
            json!({ "topic": topic, "date": "2026-01-02", "summary": "x", "areas": [area] }),
        )
        .unwrap_err();
        assert!(format!("{err:?}").contains("handoff.overlap"), "{err:?}");
    }
    let again = call(
        &mut agent,
        "handoff_claim",
        json!({ "topic": "drones", "date": "2026-01-03", "summary": "x", "areas": ["other"] }),
    );
    assert!(format!("{:?}", again.unwrap_err()).contains("agent.exists"));
    for bad in [
        json!({ "topic": "Bad Topic", "date": "2026-01-02", "summary": "x", "areas": ["a"] }),
        json!({ "topic": "ok", "date": "January", "summary": "x", "areas": ["a"] }),
        json!({ "topic": "ok", "date": "2026-01-02", "summary": "x", "areas": [] }),
    ] {
        assert!(call(&mut agent, "handoff_claim", bad).is_err());
    }

    // Claims merged from two branches that collide show up as problems.
    std::fs::write(
        dir.path().join("docs/ai/claims/merged.claim.json"),
        r#"{"topic":"merged","date":"2026-01-02","summary":"From another branch.","areas":["scripts/drones/ai.ts"],"decisions":[10,12]}"#,
    )
    .unwrap();
    let status = call(&mut agent, "handoff_status", json!({})).unwrap();
    let problems = status["problems"].to_string();
    assert!(
        problems.contains("both cover") && problems.contains("overlapping decision numbers"),
        "{problems}"
    );
    assert_eq!(status["next_decision"], 13);
    assert_eq!(status["claims"].as_array().unwrap().len(), 3);
    std::fs::remove_file(dir.path().join("docs/ai/claims/merged.claim.json")).unwrap();

    // A session note keeps the claim; a releasing one removes it. Same day, second file.
    let n1 = call(
        &mut agent,
        "handoff_note",
        json!({ "topic": "drones", "date": "2026-01-02", "done": ["Drone script moves items; scenario drones passes."],
                "unverified": ["Path cost on large maps; check with perf_sim."], "next": ["Charging pads."] }),
    )
    .unwrap();
    assert_eq!(n1["path"], "docs/ai/sessions/2026-01-02-drones.md");
    assert_eq!(n1["still_claimed"], true);
    let n2 = call(
        &mut agent,
        "handoff_note",
        json!({ "topic": "drones", "date": "2026-01-02", "done": ["Charging pads."], "release": true }),
    )
    .unwrap();
    assert_eq!(n2["path"], "docs/ai/sessions/2026-01-02-drones-2.md");
    assert_eq!(n2["released"], true);
    let text = std::fs::read_to_string(dir.path().join("docs/ai/sessions/2026-01-02-drones.md")).unwrap();
    assert!(
        text.contains("## Unverified") && text.contains("- Charging pads."),
        "{text}"
    );
    let status = call(&mut agent, "handoff_status", json!({})).unwrap();
    assert_eq!(status["claims"].as_array().unwrap().len(), 1);
    assert_eq!(status["sessions"], 2);
    assert_eq!(status["problems"], json!([]));

    // Always-read files that grow are reported.
    std::fs::write(dir.path().join("docs/ai/STATE.md"), "x".repeat(17 * 1024)).unwrap();
    let status = call(&mut agent, "handoff_status", json!({})).unwrap();
    assert!(status["problems"].to_string().contains("limit"), "{status}");

    // Read-only agents cannot claim or note.
    let mut reader = Agent::new(dir.path(), true);
    assert!(
        reader
            .call(
                "handoff_note",
                json!({ "topic": "power-grid", "date": "2026-01-02", "done": ["x"], "release": true })
            )
            .is_err()
    );
    assert!(dir.path().join("docs/ai/claims/power-grid.claim.json").exists());
}
