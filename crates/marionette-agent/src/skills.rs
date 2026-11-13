//! Agent skills shipped with the engine (decision 0160): one folder per task under
//! `.claude/skills/` with a `SKILL.md` (front matter `name` and `description`, then the
//! procedure). A skill-aware agent always sees the descriptions and loads a body only when
//! the task matches. The files are compiled in, so a game's binary can list them, hand one
//! out over MCP, and install them into the game's own repository.

use crate::{Agent, Op, codes, op};
use marionette_core::{Diagnostic, Errors};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One shipped skill.
pub struct Skill {
    /// Folder name and front matter `name`.
    pub name: &'static str,
    /// The whole `SKILL.md`.
    pub text: &'static str,
    /// Cargo feature the skill's operations need; skills of parts left out of a build are
    /// not listed or installed.
    pub feature: Option<&'static str>,
}

macro_rules! skill {
    ($name:literal) => {
        skill!($name, None)
    };
    ($name:literal, $feature:expr) => {
        Skill {
            name: $name,
            text: include_str!(concat!("../../../.claude/skills/", $name, "/SKILL.md")),
            feature: $feature,
        }
    };
}

/// Every skill in the repository, sorted by name. `marionette-*` skills are for games and
/// are installed into game projects; `engine-*` skills are for work on the engine itself.
pub const ALL: &[Skill] = &[
    skill!("engine-add-operation"),
    skill!("engine-add-scene-or-mode"),
    skill!("engine-add-skill"),
    skill!("engine-finish-change"),
    skill!("marionette-add-rust-code"),
    skill!("marionette-animate-model"),
    skill!("marionette-build-scene"),
    skill!("marionette-find-assets", Some("pool")),
    skill!("marionette-find-slowdown"),
    skill!("marionette-generate-assets", Some("generate")),
    skill!("marionette-handoff"),
    skill!("marionette-light-scene"),
    skill!("marionette-multiplayer", Some("netplay")),
    skill!("marionette-package-game", Some("package")),
    skill!("marionette-port-unity-unreal", Some("port")),
    skill!("marionette-synth-audio"),
    skill!("marionette-test-game"),
    skill!("marionette-tune-render-cost"),
    skill!("marionette-world-ui", Some("ui3d")),
    skill!("marionette-write-job", Some("jobs-ts")),
    skill!("marionette-write-script", Some("script")),
];

/// Folders a skill-aware agent reads project skills from: `.claude/skills` (Claude Code and
/// agents that follow it) and `.agents/skills` (the shared folder of the open format).
pub const TARGETS: &[(&str, &str)] = &[("claude", ".claude/skills"), ("agents", ".agents/skills")];

impl Skill {
    /// `game` for skills installed into game projects, `engine` for engine work.
    pub fn audience(&self) -> &'static str {
        if self.name.starts_with("engine-") {
            "engine"
        } else {
            "game"
        }
    }

    /// The front matter `description` (one line).
    pub fn description(&self) -> &'static str {
        front_matter(self.text, "description").unwrap_or("")
    }

    pub fn compiled_in(&self) -> bool {
        self.feature.is_none_or(|f| crate::FEATURES.contains(&f))
    }
}

/// A one-line front matter value of a `SKILL.md`.
pub fn front_matter<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let mut lines = text.lines();
    if lines.next()?.trim_end() != "---" {
        return None;
    }
    for line in lines {
        let line = line.trim_end();
        if line == "---" {
            break;
        }
        if let Some(value) = line.strip_prefix(key).and_then(|r| r.strip_prefix(':')) {
            return Some(value.trim());
        }
    }
    None
}

/// Game skills of this build, in name order.
fn game_skills() -> impl Iterator<Item = &'static Skill> {
    ALL.iter().filter(|s| s.audience() == "game" && s.compiled_in())
}

pub(crate) fn ops() -> Vec<Op> {
    vec![
        op(
            "skills_list",
            "List the task skills: name and when to use each. Read the matching one with skills_get \
             before a task.",
            false,
            list,
        ),
        op(
            "skills_get",
            "One skill's instructions: the tested procedure, traps and checks for a kind of task.",
            false,
            get,
        ),
        op(
            "skills_install",
            "Write or refresh the game skills in a project (`.claude/skills`, `.agents/skills`) so \
             skill-aware agents load them by themselves. The project's own skills are left alone.",
            true,
            install,
        ),
    ]
}

#[derive(Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct ListInput {
    /// Also list the skills for work on the engine itself (default: game skills only).
    #[serde(default)]
    pub engine: bool,
}

#[derive(Serialize, JsonSchema)]
pub struct SkillSummary {
    pub name: String,
    /// What the skill does and when to use it.
    pub description: String,
    /// `game` or `engine`.
    pub audience: String,
}

#[derive(Serialize, JsonSchema)]
pub struct ListOutput {
    pub skills: Vec<SkillSummary>,
}

fn list(_: &mut Agent, input: ListInput) -> Result<ListOutput, Errors> {
    let skills = ALL
        .iter()
        .filter(|s| s.compiled_in() && (input.engine || s.audience() == "game"))
        .map(|s| SkillSummary {
            name: s.name.into(),
            description: s.description().into(),
            audience: s.audience().into(),
        })
        .collect();
    Ok(ListOutput { skills })
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetInput {
    /// Skill name from skills_list.
    pub name: String,
}

#[derive(Serialize, JsonSchema)]
pub struct GetOutput {
    pub name: String,
    /// The instructions, without the front matter.
    pub instructions: String,
}

fn get(_: &mut Agent, input: GetInput) -> Result<GetOutput, Errors> {
    let Some(skill) = ALL.iter().find(|s| s.name == input.name) else {
        return Err(
            Diagnostic::error(codes::NOT_FOUND, format!("no skill named `{}`", input.name))
                .fix("list the names with skills_list (`engine: true` for engine skills)")
                .into(),
        );
    };
    Ok(GetOutput {
        name: skill.name.into(),
        instructions: body(skill.text).into(),
    })
}

/// A `SKILL.md` without its front matter.
pub fn body(text: &str) -> &str {
    let Some(rest) = text.strip_prefix("---") else {
        return text;
    };
    match rest.find("\n---") {
        Some(end) => rest[end + 4..].trim_start_matches(['\r', '\n']),
        None => text,
    }
}

#[derive(Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct InstallInput {
    /// Project folder to install into (default: the project root).
    pub path: Option<String>,
    /// Which skill folders to write: `claude` (`.claude/skills`) and `agents`
    /// (`.agents/skills`). Default: both.
    pub targets: Option<Vec<String>>,
}

#[derive(Serialize, JsonSchema)]
pub struct InstallOutput {
    /// Skill names installed.
    pub skills: Vec<String>,
    /// Skill folders written, relative to the project root.
    pub folders: Vec<String>,
    /// Files written.
    pub files: usize,
}

fn install(agent: &mut Agent, input: InstallInput) -> Result<InstallOutput, Errors> {
    let base = input.path.unwrap_or_default();
    let base = base.trim_end_matches(['/', '\\']);
    let names = input
        .targets
        .unwrap_or_else(|| TARGETS.iter().map(|(n, _)| n.to_string()).collect());
    let mut folders = Vec::new();
    for name in &names {
        let Some((_, folder)) = TARGETS.iter().find(|(n, _)| n == name) else {
            return Err(
                Diagnostic::error(codes::BAD_INPUT, format!("unknown target `{name}`"))
                    .fix("use `claude`, `agents` or both")
                    .into(),
            );
        };
        folders.push(if base.is_empty() || base == "." {
            folder.to_string()
        } else {
            format!("{base}/{folder}")
        });
    }
    let files = install_into(agent, &folders)?;
    Ok(InstallOutput {
        skills: game_skills().map(|s| s.name.to_string()).collect(),
        folders,
        files,
    })
}

/// Writes every game skill of this build under each folder. Returns the file count.
pub(crate) fn install_into(agent: &Agent, folders: &[String]) -> Result<usize, Errors> {
    let mut files = 0;
    for folder in folders {
        for skill in game_skills() {
            agent.write_file(
                &format!("{folder}/{}/SKILL.md", skill.name),
                skill.text.as_bytes(),
            )?;
            files += 1;
        }
    }
    Ok(files)
}
