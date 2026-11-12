//! The concept tables: every Unity and Unreal concept the porting tools know, with its
//! Marionette equivalent and a status. The tables are data (`data/unity.json`,
//! `data/unreal.json`), compiled into the binary, served by the `port_map` operation and
//! rendered into `docs/ai/reference/porting-map.md` by `marionette docs`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

/// The source engine a file or table belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    Unity,
    Unreal,
}

impl Engine {
    pub fn as_str(self) -> &'static str {
        match self {
            Engine::Unity => "unity",
            Engine::Unreal => "unreal",
        }
    }
}

/// How far Marionette covers a concept today.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Exists now as a component, an action map entry, a file or an operation. Port as data.
    Ready,
    /// Exists in the TypeScript script API (`marionette-script`, decision 0022): a direct
    /// call or hook with the same meaning.
    Script,
    /// Possible today, but the code must take a different shape (for example a coroutine
    /// becomes a tick counter in `self.state`). The note says how.
    Rewrite,
    /// Marionette has no equivalent yet. The note gives the best workaround; the gap list of
    /// a scan says which engine features ported games need most.
    Gap,
    /// Breaks determinism (wall clock, threads, platform math, unseeded randomness) and must
    /// be rewritten; the note says with what.
    Hazard,
    /// Belongs to the source engine but is not in the tables yet (scan results only).
    Unknown,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Ready => "ready",
            Status::Script => "script",
            Status::Rewrite => "rewrite",
            Status::Gap => "gap",
            Status::Hazard => "hazard",
            Status::Unknown => "unknown",
        }
    }
}

/// One mapped concept.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    /// The source name: a class, hook, component, type, or a call pattern with segments
    /// joined by `.` (C++ `::` and `->` are written as `.`).
    #[serde(alias = "pattern")]
    pub name: String,
    /// The Marionette equivalent (code, component or file). Empty for gaps without one.
    pub marionette: String,
    pub status: Status,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// Base classes only: what kind of class this is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<Role>,
}

/// What a source class is, from its base class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Unity MonoBehaviour: becomes a script.
    Behaviour,
    /// Unreal actor: becomes an entity with a script.
    Actor,
    /// Unreal actor component: becomes (part of) a script.
    Component,
    /// Data-only class (ScriptableObject, UDataAsset).
    Data,
    /// Editor-only code: not ported.
    Editor,
    /// A Blueprint graph exported as text.
    Blueprint,
    /// Other engine classes.
    Other,
    /// A plain class with no engine base: ported as ordinary TypeScript.
    Plain,
}

/// The section of a table an entry is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Section {
    Bases,
    Hooks,
    Components,
    Types,
    Api,
}

impl Section {
    pub const ALL: [Section; 5] = [
        Section::Bases,
        Section::Hooks,
        Section::Components,
        Section::Types,
        Section::Api,
    ];
    pub fn as_str(self) -> &'static str {
        match self {
            Section::Bases => "bases",
            Section::Hooks => "hooks",
            Section::Components => "components",
            Section::Types => "types",
            Section::Api => "api",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Table {
    pub engine: Engine,
    pub language: String,
    pub bases: Vec<Entry>,
    pub hooks: Vec<Entry>,
    pub components: Vec<Entry>,
    pub types: Vec<Entry>,
    pub api: Vec<Entry>,
    /// Names that belong to the engine API (class names, globals). A call chain that starts
    /// with one of them and matches no entry is reported as `unknown`.
    pub roots: Vec<String>,
}

impl Table {
    pub fn section(&self, s: Section) -> &[Entry] {
        match s {
            Section::Bases => &self.bases,
            Section::Hooks => &self.hooks,
            Section::Components => &self.components,
            Section::Types => &self.types,
            Section::Api => &self.api,
        }
    }

    fn find<'a>(list: &'a [Entry], name: &str) -> Option<&'a Entry> {
        list.iter().find(|e| e.name == name)
    }
    pub fn base(&self, name: &str) -> Option<&Entry> {
        Self::find(&self.bases, name)
    }
    pub fn hook(&self, name: &str) -> Option<&Entry> {
        Self::find(&self.hooks, name)
    }
    pub fn component(&self, name: &str) -> Option<&Entry> {
        Self::find(&self.components, name)
    }
    pub fn ty(&self, name: &str) -> Option<&Entry> {
        Self::find(&self.types, name)
    }
}

pub fn table(engine: Engine) -> &'static Table {
    static UNITY: OnceLock<Table> = OnceLock::new();
    static UNREAL: OnceLock<Table> = OnceLock::new();
    let (cell, text) = match engine {
        Engine::Unity => (&UNITY, include_str!("../data/unity.json")),
        Engine::Unreal => (&UNREAL, include_str!("../data/unreal.json")),
    };
    cell.get_or_init(|| serde_json::from_str(text).expect("built-in porting table is valid (tested)"))
}

/// The tables as Markdown (`docs/ai/reference/porting-map.md`).
pub fn reference() -> String {
    use std::fmt::Write;
    let esc = |s: &str| s.replace('|', "\\|").replace('\n', " ");
    let mut out = String::from(
        "# Porting map: Unity and Unreal to Marionette\n\nGENERATED by `marionette docs` from `crates/marionette-port/data/*.json`. Do not edit by hand; edit the tables. The recipe is `docs/ai/PORTING.md`; look entries up with the `port_map` operation.\n\nStatus: `ready` exists now as data (component, action map, file, operation); `script` is a direct call or hook in the TypeScript script API; `rewrite` needs a different code shape (the note says how); `gap` has no equivalent yet; `hazard` breaks determinism and must be rewritten.\n",
    );
    for engine in [Engine::Unity, Engine::Unreal] {
        let t = table(engine);
        let title = match engine {
            Engine::Unity => "Unity (C#)",
            Engine::Unreal => "Unreal (C++ and Blueprints)",
        };
        let _ = writeln!(out, "\n## {title}\n");
        for s in Section::ALL {
            let heading = match s {
                Section::Bases => "Base classes",
                Section::Hooks => "Lifecycle hooks and events",
                Section::Components => "Components",
                Section::Types => "Field and value types",
                Section::Api => "API calls",
            };
            let _ = writeln!(
                out,
                "### {heading}\n\n| {} | Marionette | Status | Note |\n| --- | --- | --- | --- |",
                t.language
            );
            let mut rows: Vec<&Entry> = t.section(s).iter().collect();
            rows.sort_by(|a, b| a.name.cmp(&b.name));
            for e in rows {
                let m = if e.marionette.is_empty() {
                    String::new()
                } else if e.marionette.ends_with('.') {
                    esc(&e.marionette)
                } else {
                    format!("`{}`", esc(&e.marionette).replace('`', "'"))
                };
                let _ = writeln!(
                    out,
                    "| `{}` | {m} | {} | {} |",
                    e.name,
                    e.status.as_str(),
                    esc(&e.note)
                );
            }
            out.push('\n');
        }
    }
    while out.ends_with("\n\n") {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn tables_parse_and_have_no_duplicates() {
        for engine in [Engine::Unity, Engine::Unreal] {
            let t = table(engine);
            assert_eq!(t.engine, engine);
            for s in Section::ALL {
                let mut seen = BTreeSet::new();
                for e in t.section(s) {
                    assert!(
                        seen.insert(e.name.clone()),
                        "{engine:?} {s:?}: duplicate {}",
                        e.name
                    );
                    assert!(
                        !e.name.contains("::") && !e.name.contains("->"),
                        "{}: use `.`",
                        e.name
                    );
                    // A hazard always says what to do instead.
                    if e.status == Status::Hazard {
                        assert!(!e.note.is_empty(), "{engine:?} {}: hazard without a note", e.name);
                    }
                    assert_ne!(e.status, Status::Unknown, "tables never use `unknown`");
                }
            }
            for b in &t.bases {
                assert!(b.role.is_some(), "base {} needs a role", b.name);
            }
        }
    }
}
