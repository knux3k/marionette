//! Save games (decision 0076).
//!
//! - [`SaveFormat`]: a scene resource naming the game, its save version, autosave settings
//!   and an optional migrations file. [`SaveModule`] registers it.
//! - [`format`]: the `.msave` file: a magic line, one header line of JSON (readable without
//!   the rest), then the scene text and module bytes, checked by SHA-256.
//! - [`SaveStore`]: named slots in one folder; writes go to a temporary file, the old save
//!   becomes `<slot>.msave.bak`, then a rename; reads fall back to the backup.
//! - [`migrate`]: data steps (rename, remove, set fields) and Rust steps that take an old
//!   save's scene up one version at a time.
//! - [`SaveWriter`]: a background thread that turns a [`StateCapture`] into a file, so the
//!   tick pays only for the copy (decision 0073).
//! - [`Autosaver`]: autosave every N ticks and on `save_request` events, rotating through
//!   `auto-0..auto-(n-1)`.
//!
//! Loading: [`load`] reads a slot, migrates, and restores a [`Sim`].

pub mod autosave;
pub mod format;
pub mod migrate;
pub mod store;
pub mod writer;

pub use autosave::Autosaver;
pub use format::{SaveFile, SaveHeader};
pub use migrate::{Change, Migrations, MigrationsFile};
pub use store::{SaveStore, SlotInfo};
pub use writer::{SaveWriter, WriteResult};

use bevy_ecs::prelude::*;
use bevy_reflect::Reflect;
use marionette_core::module::{Module, SceneDir};
use marionette_core::{Diagnostic, Errors, Registry, Sim, SimSnapshot, StateCapture};
use std::sync::Arc;

/// Stable error codes of this crate (never change what one means).
pub mod codes {
    pub const NOT_FOUND: &str = "save.not_found";
    pub const CORRUPT: &str = "save.corrupt";
    pub const NEWER: &str = "save.newer";
    pub const OTHER_GAME: &str = "save.other_game";
    pub const BAD_SLOT: &str = "save.bad_slot";
    pub const IO: &str = "save.io";
    pub const MIGRATION: &str = "save.migration";
    pub const FORMAT: &str = "save.format";
    pub const META: &str = "save.meta";
}

/// How a game's saves are named and versioned. Put it in the game's scene; a world without
/// it saves as game `""`, version 1, with autosave off.
#[derive(Resource, Reflect, Debug, Clone, PartialEq)]
pub struct SaveFormat {
    /// Game id written in every save; a save of another game is refused on load.
    pub game: String,
    /// Save version, starting at 1. Raise it when saved data changes shape and add a
    /// migration step from the old version.
    pub version: u32,
    /// Autosave every this many ticks (0: only on `save_request` events).
    pub autosave_every: u64,
    /// Autosave slots `auto-0` .. `auto-(n-1)`, overwritten oldest first (at least 1).
    pub autosave_slots: u32,
    /// Data migrations file (JSON, relative to the scene folder); empty for none.
    pub migrations: String,
}

impl Default for SaveFormat {
    fn default() -> Self {
        Self {
            game: String::new(),
            version: 1,
            autosave_every: 0,
            autosave_slots: 3,
            migrations: String::new(),
        }
    }
}

/// Migrations available to the running world: the scene's migrations file plus Rust steps
/// given to [`SaveModule::with`]. Inserted by the module; not saved, not hashed.
#[derive(Resource, Clone, Default)]
pub struct SaveMigrations(pub Arc<Migrations>);

/// Registers [`SaveFormat`] and builds [`SaveMigrations`] when a scene loads.
#[derive(Default)]
pub struct SaveModule {
    rust: Arc<Migrations>,
}

impl SaveModule {
    /// A module with a game's own Rust migration steps (merged after the data steps of the
    /// same version).
    pub fn with(migrations: Migrations) -> Self {
        Self {
            rust: Arc::new(migrations),
        }
    }
}

impl Module for SaveModule {
    fn name(&self) -> &'static str {
        "save"
    }

    fn register(&self, registry: &mut Registry) {
        registry.register_resource::<SaveFormat>();
    }

    fn init(&self, world: &mut World) -> marionette_core::Result<()> {
        // Once per session start: the module keeps its Rust steps for the next init.
        let mut all = (*self.rust).clone();
        if let Some(file) = world
            .get_resource::<SaveFormat>()
            .map(|f| f.migrations.clone())
            .filter(|f| !f.is_empty())
        {
            let dir = world
                .get_resource::<SceneDir>()
                .map(|d| d.0.clone())
                .unwrap_or_default();
            let data = MigrationsFile::read(&dir.join(&file), &file)?;
            all = data.into_migrations().merged(all);
        }
        world.insert_resource(SaveMigrations(Arc::new(all)));
        Ok(())
    }
}

/// The world's [`SaveFormat`], or the default.
pub fn format_of(world: &World) -> SaveFormat {
    world.get_resource::<SaveFormat>().cloned().unwrap_or_default()
}

fn format_json(f: &SaveFormat) -> serde_json::Value {
    serde_json::json!({
        "game": f.game,
        "version": f.version,
        "autosave_every": f.autosave_every,
        "autosave_slots": f.autosave_slots,
        "migrations": f.migrations,
    })
}

/// What a save carries besides the state: a label and small game data for a slot menu.
#[derive(Debug, Clone, Default)]
pub struct SaveInfo {
    pub label: String,
    /// Small JSON (at most [`format::META_MAX`] bytes as text): play time, chapter, money.
    pub meta: serde_json::Value,
}

/// A save ready to write: the copied state plus the header fields known on the main thread.
pub struct Pending {
    pub state: StateCapture,
    pub format: SaveFormat,
    pub info: SaveInfo,
}

impl Pending {
    /// Copies `sim`'s state (the only part that runs on the tick's thread).
    pub fn capture(sim: &mut Sim, info: SaveInfo) -> Self {
        Self {
            format: format_of(&sim.world),
            state: sim.capture_state(),
            info,
        }
    }

    /// Builds the file (scene text and checksum): the slow part, any thread.
    pub fn into_file(self, registry: &Registry, saved_unix: u64) -> Result<SaveFile, Errors> {
        let snap = self.state.into_snapshot(registry);
        SaveFile::new(&snap, &self.format, self.info, saved_unix)
    }
}

/// What [`load`] did.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Loaded {
    pub tick: u64,
    /// Save version found in the file.
    pub from_version: u32,
    /// Version after migration (the game's current version).
    pub to_version: u32,
    /// True when module state (physics internals, held input) was restored too, so the game
    /// continues exactly as it would have. Migrated saves rebuild that state from
    /// components instead.
    pub exact: bool,
    /// Set when the slot file was missing or damaged and its backup was loaded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovered: Option<String>,
}

/// Reads `slot` from `store`, migrates it to the running game's version and restores `sim`.
pub fn load(sim: &mut Sim, store: &SaveStore, slot: &str) -> Result<Loaded, Errors> {
    let (file, recovered) = store.read(slot)?;
    let mut loaded = restore(sim, file)?;
    loaded.recovered = recovered;
    Ok(loaded)
}

/// Restores `sim` from a decoded save (see [`load`]).
pub fn restore(sim: &mut Sim, file: SaveFile) -> Result<Loaded, Errors> {
    let current = format_of(&sim.world);
    let migrations = sim
        .world
        .get_resource::<SaveMigrations>()
        .map(|m| Arc::clone(&m.0)) // a refcount bump, so `sim` can be borrowed mutably below
        .unwrap_or_default();
    let names: Vec<&'static str> = sim.modules().iter().map(|m| m.name()).collect();
    let (snap, from, exact) = prepare(file, &current, &migrations, &names)?;
    sim.restore(&snap)?;
    Ok(Loaded {
        tick: snap.tick,
        from_version: from,
        to_version: current.version,
        exact,
        recovered: None,
    })
}

/// Checks game and version, migrates, and gives the snapshot to restore, the version the
/// file had, and whether module state is kept (`exact`). Module bytes are dropped when a
/// migration ran or a module is no longer present (`modules`: the running module names).
pub fn prepare(
    file: SaveFile,
    current: &SaveFormat,
    migrations: &Migrations,
    modules: &[&'static str],
) -> Result<(SimSnapshot, u32, bool), Errors> {
    let h = &file.header;
    if h.game != current.game {
        return Err(Diagnostic::error(
            codes::OTHER_GAME,
            format!("save is from game `{}`, this is `{}`", h.game, current.game),
        )
        .fix("load it with the game that wrote it")
        .into());
    }
    let from = h.version;
    if from > current.version {
        return Err(Diagnostic::error(
            codes::NEWER,
            format!(
                "save version {from} is newer than this game's version {}",
                current.version
            ),
        )
        .fix("update the game; old builds cannot read newer saves")
        .into());
    }
    let (mut snap, all_modules) = file.into_snapshot(modules)?;
    let mut exact = all_modules;
    let mut scene = marionette_core::scene::parse(&snap.scene)?;
    if from < current.version {
        migrations.apply(&mut scene, from, current.version)?;
        snap.modules.clear();
        exact = false;
    }
    // The running game's format wins over the one the save carried (its migrations file
    // or autosave settings may have changed since).
    if let Some(saved) = scene.resources.get_mut("SaveFormat") {
        *saved = format_json(current);
    }
    snap.scene = scene.to_text();
    Ok((snap, from, exact))
}
