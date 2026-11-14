//! Save game operations (decision 0076): write and load slots of a live session, list,
//! check and delete slots. Slots live in `dir` (default `saves`, relative to the project
//! root); a shipped game uses its own folder through `marionette-save` directly.

use crate::session::get_mut;
use crate::{Agent, Op, op};
use marionette_core::{Diagnostic, Errors};
use marionette_save::{Pending, SaveInfo, SaveStore, format_of};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::time::Instant;

pub(crate) fn ops() -> Vec<Op> {
    vec![
        op(
            "save_write",
            "Save a live session to a slot (crash-safe, keeps a backup).",
            true,
            write,
        ),
        op(
            "save_load",
            "Load a slot into a live session, migrating old saves.",
            true,
            load,
        ),
        op("save_list", "List save slots (headers only).", false, list),
        op(
            "save_check",
            "Check that a slot would load into a session.",
            false,
            check,
        ),
        op("save_delete", "Delete a slot and its backup.", true, delete),
    ]
}

fn default_dir() -> String {
    "saves".into()
}

fn store(agent: &Agent, dir: &str) -> Result<SaveStore, Errors> {
    SaveStore::open(agent.resolve(dir)?)
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WriteInput {
    pub session: String,
    /// Slot name: 1 to 48 of a-z, 0-9, `_`, `-` (autosaves use `auto-N`).
    pub slot: String,
    /// Folder of slots, relative to the project root (default `saves`).
    #[serde(default = "default_dir")]
    pub dir: String,
    /// Text for a slot menu.
    #[serde(default)]
    pub label: String,
    /// Small JSON for a slot menu (at most 16 KB): play time, chapter.
    #[serde(default)]
    pub meta: serde_json::Value,
}

#[derive(Serialize, JsonSchema)]
pub struct WriteOutput {
    pub slot: String,
    pub path: String,
    pub bytes: u64,
    pub tick: u64,
    pub version: u32,
    /// Main-thread part: copying the state.
    pub capture_ms: f64,
    /// The rest: scene text, checksum, file (a game does this on its save thread).
    pub write_ms: f64,
}

fn write(agent: &mut Agent, input: WriteInput) -> Result<WriteOutput, Errors> {
    let store = store(agent, &input.dir)?;
    let s = get_mut(agent, &input.session)?;
    let t = Instant::now();
    let pending = Pending::capture(
        &mut s.sim,
        SaveInfo {
            label: input.label,
            meta: input.meta,
        },
    );
    let capture_ms = t.elapsed().as_secs_f64() * 1000.0;
    let version = pending.format.version;
    let tick = pending.state.tick;
    let t = Instant::now();
    let file = pending.into_file(s.sim.registry(), marionette_save::writer::unix_now())?;
    let bytes = store.write(&input.slot, &file)?;
    let write_ms = t.elapsed().as_secs_f64() * 1000.0;
    s.log("save", format!("saved tick {tick} to slot {}", input.slot));
    Ok(WriteOutput {
        path: format!("{}/{}.msave", input.dir.trim_end_matches('/'), input.slot),
        slot: input.slot,
        bytes,
        tick,
        version,
        capture_ms,
        write_ms,
    })
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LoadInput {
    pub session: String,
    pub slot: String,
    #[serde(default = "default_dir")]
    pub dir: String,
}

#[derive(Serialize, JsonSchema)]
pub struct LoadOutput {
    pub session: String,
    pub slot: String,
    pub tick: u64,
    pub state_hash: String,
    pub from_version: u32,
    pub to_version: u32,
    /// Module state (physics internals, held input) restored too: the game continues
    /// exactly as it would have. False after a migration.
    pub exact: bool,
    /// Why the slot's backup was loaded instead, if it was.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovered: Option<String>,
}

fn load(agent: &mut Agent, input: LoadInput) -> Result<LoadOutput, Errors> {
    let store = store(agent, &input.dir)?;
    let s = get_mut(agent, &input.session)?;
    let loaded = marionette_save::load(&mut s.sim, &store, &input.slot)?;
    s.log(
        "load",
        format!("loaded slot {} (tick {})", input.slot, loaded.tick),
    );
    Ok(LoadOutput {
        session: input.session,
        slot: input.slot,
        tick: loaded.tick,
        state_hash: s.sim.state_hash(),
        from_version: loaded.from_version,
        to_version: loaded.to_version,
        exact: loaded.exact,
        recovered: loaded.recovered,
    })
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListInput {
    #[serde(default = "default_dir")]
    pub dir: String,
}

#[derive(Serialize, JsonSchema)]
pub struct SlotRow {
    pub slot: String,
    pub bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tick: Option<u64>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub saved_unix: Option<u64>,
    pub backup: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct ListOutput {
    pub dir: String,
    pub slots: Vec<SlotRow>,
}

fn list(agent: &mut Agent, input: ListInput) -> Result<ListOutput, Errors> {
    let path = agent.resolve(&input.dir)?;
    if !path.exists() {
        return Ok(ListOutput {
            dir: input.dir,
            slots: Vec::new(),
        });
    }
    let slots = SaveStore::open(path)?
        .list()?
        .into_iter()
        .map(|i| {
            let (game, version, tick, label, saved_unix) = match i.header {
                Some(h) => (
                    Some(h.game),
                    Some(h.version),
                    Some(h.tick),
                    h.label,
                    Some(h.saved_unix),
                ),
                None => (None, None, None, String::new(), None),
            };
            SlotRow {
                slot: i.slot,
                bytes: i.bytes,
                game,
                version,
                tick,
                label,
                saved_unix,
                backup: i.has_backup,
                problem: i.problem,
            }
        })
        .collect();
    Ok(ListOutput {
        dir: input.dir,
        slots,
    })
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckInput {
    /// A live session of the game that would load the save (its format and migrations).
    pub session: String,
    pub slot: String,
    #[serde(default = "default_dir")]
    pub dir: String,
}

#[derive(Serialize, JsonSchema)]
pub struct CheckOutput {
    pub slot: String,
    /// Loading would succeed.
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_version: Option<u32>,
    pub to_version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovered: Option<String>,
    /// What would stop the load (empty when `ok`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[schemars(with = "Vec<serde_json::Value>")]
    pub problems: Vec<Diagnostic>,
}

fn check(agent: &mut Agent, input: CheckInput) -> Result<CheckOutput, Errors> {
    let store = store(agent, &input.dir)?;
    let s = get_mut(agent, &input.session)?;
    let current = format_of(&s.sim.world);
    let migrations = s
        .sim
        .world
        .get_resource::<marionette_save::SaveMigrations>()
        .map(|m| std::sync::Arc::clone(&m.0)) // a refcount bump; the session is used below
        .unwrap_or_default();
    let names: Vec<&'static str> = s.sim.modules().iter().map(|m| m.name()).collect();
    let read = store.read(&input.slot);
    let mut out = CheckOutput {
        slot: input.slot,
        ok: false,
        from_version: None,
        to_version: current.version,
        recovered: None,
        problems: Vec::new(),
    };
    let (file, recovered) = match read {
        Ok(x) => x,
        Err(e) => {
            out.problems = e.0;
            return Ok(out);
        }
    };
    out.recovered = recovered;
    out.from_version = Some(file.header.version);
    let result = marionette_save::prepare(file, &current, &migrations, &names).and_then(|(snap, _, _)| {
        let scene = marionette_core::scene::parse(&snap.scene)?;
        marionette_core::scene::validate(&scene, s.sim.registry(), Some(&snap.scene)).map(|_| ())
    });
    match result {
        Ok(()) => out.ok = true,
        Err(e) => out.problems = e.0,
    }
    Ok(out)
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeleteInput {
    pub slot: String,
    #[serde(default = "default_dir")]
    pub dir: String,
}

#[derive(Serialize, JsonSchema)]
pub struct DeleteOutput {
    pub slot: String,
    /// False when the slot did not exist.
    pub deleted: bool,
}

fn delete(agent: &mut Agent, input: DeleteInput) -> Result<DeleteOutput, Errors> {
    let deleted = store(agent, &input.dir)?.delete(&input.slot)?;
    Ok(DeleteOutput {
        slot: input.slot,
        deleted,
    })
}
