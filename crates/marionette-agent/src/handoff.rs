//! Handoffs for game projects (decision 0161): files that let the next agent, or several
//! agents at once, continue without asking. `docs/ai/START-HERE.md` and `STATE.md` are the
//! small always-read set; work in progress is one claim file per piece of work
//! (`docs/ai/claims/<topic>.claim.json`), history is one note per session
//! (`docs/ai/sessions/<date>-<topic>.md`), reasons are `docs/decisions/NNNN-title.md`.
//! One file per claim and per session means parallel branches never edit the same lines.

use crate::{Agent, Op, codes, op, read_text};
use marionette_core::{Diagnostic, Errors};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Two claims name the same area, or their decision numbers overlap.
pub const OVERLAP: &str = "handoff.overlap";

const START: &str = "docs/ai/START-HERE.md";
const STATE: &str = "docs/ai/STATE.md";
const CLAIMS: &str = "docs/ai/claims";
const SESSIONS: &str = "docs/ai/sessions";
const DECISIONS: &str = "docs/decisions";
/// Bytes START-HERE.md and STATE.md may take together before `handoff_status` complains.
pub const ALWAYS_READ_LIMIT: usize = 16 * 1024;

/// Heading of the section of START-HERE.md a game fills in.
const THIS_GAME: &str = "## This game";

const TEMPLATES: &[(&str, &str)] = &[
    (START, include_str!("../templates/handoff/START-HERE.md")),
    (STATE, include_str!("../templates/handoff/STATE.md")),
    (
        "docs/ai/claims/README.md",
        "# Claims\n\nOne `<topic>.claim.json` per piece of work in progress, written by `handoff_claim` and removed by\n`handoff_note` with `release: true`. An empty folder means nobody is working.\n",
    ),
    (
        "docs/ai/sessions/README.md",
        "# Session notes\n\nOne `<date>-<topic>.md` per session, written by `handoff_note`. Read the newest few\n(`handoff_status` lists them), not all.\n",
    ),
    (
        "docs/decisions/README.md",
        "# Decisions\n\nOne `NNNN-short-title.md` per choice between real alternatives: `# NNNN: Title`, then `Date`,\n`Status`, `Decision`, `Why`, and optional `Rejected` and `Not done yet`. Take numbers from your\nclaim's block (`handoff_claim` with `decisions`), or the next free one (`handoff_status`).\n",
    ),
];

pub(crate) fn ops() -> Vec<Op> {
    vec![
        op(
            "handoff_init",
            "Add the handoff files (docs/ai, docs/decisions) to a project, with the game's title, summary \
             and notes in a new START-HERE.md. Never overwrites.",
            true,
            init,
        ),
        op(
            "handoff_status",
            "Start of every session: what to read first, claimed work, latest session notes, next free \
             decision number, problems.",
            false,
            status,
        ),
        op(
            "handoff_claim",
            "Claim work before starting it: the paths it will change and a block of decision numbers. \
             Refused when a path overlaps another claim.",
            true,
            claim,
        ),
        op(
            "handoff_note",
            "End of every session: write the session note (done, not done, unverified, next). \
             `release: true` removes the finished claim.",
            true,
            note,
        ),
    ]
}

/// Writes the template files missing under `base` (a project folder relative to the agent's
/// root; empty for the root). Returns what was written.
pub(crate) fn write_templates(agent: &Agent, base: &str) -> Result<Vec<String>, Errors> {
    let mut written = Vec::new();
    for (rel, text) in TEMPLATES {
        let path = join(base, rel);
        if !agent.resolve(&path)?.exists() {
            agent.write_file(&path, text.as_bytes())?;
            written.push(rel.to_string());
        }
    }
    Ok(written)
}

fn join(base: &str, rel: &str) -> String {
    if base.is_empty() || base == "." {
        rel.to_string()
    } else {
        format!("{}/{rel}", base.trim_end_matches(['/', '\\']))
    }
}

#[derive(Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct Empty {}

#[derive(Serialize, JsonSchema)]
pub struct InitOutput {
    /// Files written; files that already existed are left alone.
    pub written: Vec<String>,
}

#[derive(Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct InitInput {
    /// Project folder, relative to the project root (default: the root itself).
    pub path: Option<String>,
    /// The game's title, for START-HERE.md.
    pub title: Option<String>,
    /// One line on what the game is.
    pub summary: Option<String>,
    /// Lines for START-HERE.md's "This game" section: the files that matter and what each is.
    #[serde(default)]
    pub notes: Vec<String>,
}

fn init(agent: &mut Agent, input: InitInput) -> Result<InitOutput, Errors> {
    let base = input.path.unwrap_or_default();
    let start = join(&base, START);
    let fresh = !agent.resolve(&start)?.exists();
    let written = write_templates(agent, &base)?;
    // A START-HERE.md written just now gets the game's own lines; an existing one is the
    // project's and is left alone.
    if fresh && (input.title.is_some() || input.summary.is_some() || !input.notes.is_empty()) {
        let template = TEMPLATES[0].1;
        let head = template.split(THIS_GAME).next().expect("split yields one part");
        let mut text = format!("{head}{THIS_GAME}\n");
        match (&input.title, &input.summary) {
            (Some(t), Some(s)) => text.push_str(&format!("\n{}: {}\n", t.trim(), s.trim())),
            (Some(one), None) | (None, Some(one)) => text.push_str(&format!("\n{}\n", one.trim())),
            (None, None) => {}
        }
        if !input.notes.is_empty() {
            text.push('\n');
            for line in &input.notes {
                text.push_str(&format!("- {}\n", line.trim()));
            }
        }
        agent.write_file(&start, text.as_bytes())?;
    }
    Ok(InitOutput { written })
}

/// A piece of work in progress.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Claim {
    /// Short name of the work, lowercase with hyphens; also the file name.
    pub topic: String,
    /// The day the claim was made, `YYYY-MM-DD`.
    pub date: String,
    /// One sentence: what will exist when this is done.
    pub summary: String,
    /// Paths (files or folders, relative to the project root) this work will change.
    pub areas: Vec<String>,
    /// Branch the work is on, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Reserved decision numbers, first and last (inclusive).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decisions: Option<(u32, u32)>,
}

fn bad(message: String, fix: &str) -> Errors {
    Diagnostic::error(codes::BAD_INPUT, message).fix(fix).into()
}

fn check_topic(topic: &str) -> Result<(), Errors> {
    let ok = !topic.is_empty()
        && topic.len() <= 48
        && !topic.starts_with('-')
        && !topic.ends_with('-')
        && topic
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if ok {
        Ok(())
    } else {
        Err(bad(
            format!("topic `{topic}` is not a short lowercase name"),
            "use lowercase letters, digits and hyphens, at most 48, e.g. `power-grid`",
        ))
    }
}

fn check_date(date: &str) -> Result<(), Errors> {
    let b = date.as_bytes();
    let ok = b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit());
    if ok {
        Ok(())
    } else {
        Err(bad(
            format!("date `{date}` is not YYYY-MM-DD"),
            "pass today's date as YYYY-MM-DD",
        ))
    }
}

/// File names in a project folder, sorted; empty when the folder is missing.
fn names(agent: &Agent, dir: &str) -> Vec<String> {
    let Ok(path) = agent.resolve(dir) else {
        return Vec::new();
    };
    let mut out: Vec<String> = std::fs::read_dir(path)
        .map(|d| {
            d.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

fn claims(agent: &Agent, problems: &mut Vec<String>) -> Vec<Claim> {
    let mut out = Vec::new();
    for name in names(agent, CLAIMS) {
        if !name.ends_with(".claim.json") {
            continue;
        }
        let shown = format!("{CLAIMS}/{name}");
        let parsed = agent
            .resolve(&shown)
            .and_then(|p| read_text(&p, &shown))
            .ok()
            .and_then(|t| serde_json::from_str::<Claim>(&t).ok());
        match parsed {
            Some(c) => out.push(c),
            None => problems.push(format!("`{shown}` is not a claim file; fix or remove it")),
        }
    }
    out
}

/// Numbers of the decision records present (`NNNN-title.md`).
fn decision_numbers(agent: &Agent) -> Vec<u32> {
    names(agent, DECISIONS)
        .iter()
        .filter(|n| n.ends_with(".md"))
        .filter_map(|n| n.split('-').next()?.parse().ok())
        .collect()
}

/// One more than the largest decision number recorded or reserved.
fn next_decision(agent: &Agent, claims: &[Claim]) -> u32 {
    let recorded = decision_numbers(agent).into_iter().max().unwrap_or(0);
    let reserved = claims
        .iter()
        .filter_map(|c| c.decisions.map(|d| d.1))
        .max()
        .unwrap_or(0);
    recorded.max(reserved) + 1
}

fn norm(area: &str) -> String {
    area.replace('\\', "/")
        .trim_matches('/')
        .trim_start_matches("./")
        .to_string()
}

/// True when one path is the other or lies inside it.
fn areas_overlap(a: &str, b: &str) -> bool {
    let (a, b) = (norm(a), norm(b));
    a == b
        || a.starts_with(&format!("{b}/"))
        || b.starts_with(&format!("{a}/"))
        || a.is_empty()
        || b.is_empty()
}

/// Every pair of claims that names the same area or the same decision numbers.
fn overlaps(claims: &[Claim]) -> Vec<String> {
    let mut out = Vec::new();
    for (i, a) in claims.iter().enumerate() {
        for b in &claims[i + 1..] {
            for x in &a.areas {
                for y in b.areas.iter().filter(|y| areas_overlap(x, y)) {
                    out.push(format!(
                        "claims `{}` and `{}` both cover `{x}` / `{y}`: agree who changes it, or narrow one claim",
                        a.topic, b.topic
                    ));
                }
            }
            if let (Some(p), Some(q)) = (a.decisions, b.decisions)
                && p.0 <= q.1
                && q.0 <= p.1
            {
                out.push(format!(
                    "claims `{}` and `{}` reserve overlapping decision numbers ({:04}-{:04}, {:04}-{:04}): the later \
                     claim takes a new block and renames its records",
                    a.topic, b.topic, p.0, p.1, q.0, q.1
                ));
            }
        }
    }
    out
}

#[derive(Serialize, JsonSchema)]
pub struct StatusOutput {
    /// Files to read before anything else, in order.
    pub read_first: Vec<String>,
    /// Work in progress.
    pub claims: Vec<Claim>,
    /// The newest session notes (at most 5, newest last); read these, not the whole folder.
    pub latest_sessions: Vec<String>,
    pub sessions: usize,
    pub decisions: usize,
    /// The next decision number nobody has recorded or reserved.
    pub next_decision: u32,
    /// Things to fix before starting work. Empty when the handoff is in order.
    pub problems: Vec<String>,
}

fn status(agent: &mut Agent, _: Empty) -> Result<StatusOutput, Errors> {
    let mut problems = Vec::new();
    let mut read_first = Vec::new();
    let mut always = 0;
    for file in ["PROTOCOL.md", START, STATE] {
        match agent.resolve(file).ok().and_then(|p| std::fs::metadata(p).ok()) {
            Some(m) => {
                read_first.push(file.to_string());
                if file != "PROTOCOL.md" {
                    always += m.len() as usize;
                }
            }
            None if file != "PROTOCOL.md" => problems.push(format!("`{file}` is missing: run handoff_init")),
            None => {}
        }
    }
    if always > ALWAYS_READ_LIMIT {
        problems.push(format!(
            "`{START}` and `{STATE}` are {always} bytes together, limit {ALWAYS_READ_LIMIT}: every session reads \
             them, so move history to session notes and detail to its own file"
        ));
    }
    let claims = claims(agent, &mut problems);
    problems.extend(overlaps(&claims));
    let mut numbers = decision_numbers(agent);
    numbers.sort_unstable();
    if let Some(w) = numbers.windows(2).find(|w| w[0] == w[1]) {
        problems.push(format!(
            "two decision records share number {:04}: renumber the newer one",
            w[0]
        ));
    }
    let sessions: Vec<String> = names(agent, SESSIONS)
        .into_iter()
        .filter(|n| n.ends_with(".md") && n != "README.md")
        .map(|n| format!("{SESSIONS}/{n}"))
        .collect();
    Ok(StatusOutput {
        read_first,
        next_decision: next_decision(agent, &claims),
        claims,
        latest_sessions: sessions[sessions.len().saturating_sub(5)..].to_vec(),
        sessions: sessions.len(),
        decisions: numbers.len(),
        problems,
    })
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClaimInput {
    /// Short name of the work, lowercase with hyphens, e.g. `power-grid`.
    pub topic: String,
    /// Today's date, `YYYY-MM-DD` (operations never read the clock).
    pub date: String,
    /// One sentence: what will exist when this is done.
    pub summary: String,
    /// Paths (files or folders, relative to the project root) this work will change. Keep them
    /// narrow: a claim on `scenes` blocks everyone else's scene work.
    pub areas: Vec<String>,
    /// Branch the work is on.
    pub branch: Option<String>,
    /// How many decision numbers to reserve (default 0). The block starts at the next free one.
    #[serde(default)]
    pub decisions: u32,
}

fn claim(agent: &mut Agent, input: ClaimInput) -> Result<Claim, Errors> {
    check_topic(&input.topic)?;
    check_date(&input.date)?;
    if input.areas.is_empty() {
        return Err(bad(
            "a claim needs at least one area".into(),
            "list the files or folders this work will change",
        ));
    }
    let path = format!("{CLAIMS}/{}.claim.json", input.topic);
    if agent.resolve(&path)?.exists() {
        return Err(
            Diagnostic::error(codes::EXISTS, format!("`{}` is already claimed", input.topic))
                .fix("pick up that work (read its claim and latest session note) or choose another topic")
                .into(),
        );
    }
    let mut ignored = Vec::new();
    let existing = claims(agent, &mut ignored);
    for other in &existing {
        for area in &input.areas {
            if let Some(theirs) = other.areas.iter().find(|a| areas_overlap(area, a)) {
                return Err(Diagnostic::error(
                    OVERLAP,
                    format!(
                        "`{area}` overlaps `{theirs}`, claimed by `{}` ({})",
                        other.topic, other.summary
                    ),
                )
                .fix(
                    "narrow the areas, wait for that work to finish, or agree the split with its owner first",
                )
                .into());
            }
        }
    }
    let decisions = (input.decisions > 0).then(|| {
        let first = next_decision(agent, &existing);
        (first, first + input.decisions - 1)
    });
    let claim = Claim {
        topic: input.topic,
        date: input.date,
        summary: input.summary,
        areas: input.areas.iter().map(|a| norm(a)).collect(),
        branch: input.branch,
        decisions,
    };
    let text = format!(
        "{}\n",
        serde_json::to_string_pretty(&claim).expect("claims serialize")
    );
    agent.write_file(&path, text.as_bytes())?;
    Ok(claim)
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoteInput {
    /// The work this session was about (the claim's topic when there is one).
    pub topic: String,
    /// Today's date, `YYYY-MM-DD`.
    pub date: String,
    /// What was finished and how it was checked, one line each.
    pub done: Vec<String>,
    /// What was started or planned but is not finished.
    #[serde(default)]
    pub not_done: Vec<String>,
    /// What was written or assumed but not checked, with what would check it.
    #[serde(default)]
    pub unverified: Vec<String>,
    /// The next step for whoever continues, most useful first.
    #[serde(default)]
    pub next: Vec<String>,
    /// Remove the claim: its work is finished (or given up, with `not_done` saying so).
    #[serde(default)]
    pub release: bool,
}

#[derive(Serialize, JsonSchema)]
pub struct NoteOutput {
    /// The session note written.
    pub path: String,
    /// True when a claim was removed.
    pub released: bool,
    /// True when the topic still has a claim (work continues).
    pub still_claimed: bool,
}

fn note(agent: &mut Agent, input: NoteInput) -> Result<NoteOutput, Errors> {
    check_topic(&input.topic)?;
    check_date(&input.date)?;
    if input.done.is_empty() && input.not_done.is_empty() {
        return Err(bad(
            "a session note needs `done` or `not_done`".into(),
            "say what the session did, even if nothing was finished",
        ));
    }
    let mut text = format!("# {} {}\n", input.date, input.topic);
    for (title, lines) in [
        ("Done", &input.done),
        ("Not done", &input.not_done),
        ("Unverified", &input.unverified),
        ("Next", &input.next),
    ] {
        if !lines.is_empty() {
            text.push_str(&format!("\n## {title}\n\n"));
            for line in lines {
                text.push_str(&format!("- {}\n", line.trim()));
            }
        }
    }
    // A second session on the same day and topic gets its own file.
    let mut path = format!("{SESSIONS}/{}-{}.md", input.date, input.topic);
    let mut n = 2;
    while agent.resolve(&path)?.exists() {
        path = format!("{SESSIONS}/{}-{}-{n}.md", input.date, input.topic);
        n += 1;
    }
    agent.write_file(&path, text.as_bytes())?;
    let claim_file = agent.resolve(&format!("{CLAIMS}/{}.claim.json", input.topic))?;
    let released = input.release && remove(&claim_file)?;
    Ok(NoteOutput {
        path,
        released,
        still_claimed: claim_file.exists(),
    })
}

fn remove(path: &Path) -> Result<bool, Errors> {
    if !path.exists() {
        return Ok(false);
    }
    std::fs::remove_file(path).map(|()| true).map_err(|e| {
        Diagnostic::error(
            marionette_core::error::codes::IO_WRITE,
            format!("cannot remove the claim file: {e}"),
        )
        .into()
    })
}
