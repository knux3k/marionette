//! Co-op networking and platform operations (decisions 0210 to 0213).
//!
//! `net_simulate` runs a whole lockstep session in one process over a simulated network and
//! says whether every peer stayed in the host's state: the check that a scene is safe to
//! network, with no second machine, no Steam and no real network.
//! `achievements_check` validates a game's achievements and stats definitions file.

use crate::{Agent, Op, codes, op};
use marionette_core::{Diagnostic, Errors};
use marionette_net::{LinkConditions, SimulateOptions, SimulateReport};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub(crate) fn ops() -> Vec<Op> {
    vec![
        op(
            "net_simulate",
            "Run a co-op lockstep session in one process: a host and `players`-1 clients, each with \
             its own copy of `scene`, over a simulated network (`link`: latency_ms, jitter_ms, \
             loss, duplicate), seeded random input on the scene's actions, optional `late_join_at` \
             and `drop_slot`/`drop_at`. Reports `all_equal` (every peer ended in the host's \
             state), desyncs the session caught by hash checks, stalls, lag in ticks and bytes.",
            false,
            simulate,
        ),
        op(
            "achievements_check",
            "Validate an achievements and stats definitions file (format \
             marionette.achievements): ids are store-safe and unique, stat-driven achievements \
             name a defined stat. Returns the counts and ids.",
            false,
            check,
        ),
    ]
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SimulateInput {
    /// Scene file, relative to the project root.
    pub scene: String,
    /// Players at the start, including the host (1 to 8). Default 2.
    pub players: Option<u8>,
    /// Ticks the host runs. Default 600.
    pub ticks: Option<u64>,
    /// Seed of the sims, the inputs and the network. Default 1.
    pub seed: Option<u64>,
    /// Simulated link, each direction. Default 40 ms, 10 ms jitter, 2% loss.
    pub link: Option<LinkConditions>,
    /// Compare state hashes every this many ticks (0 = never). Default 60.
    pub hash_interval: Option<u64>,
    /// Actions that travel. Default: the scene's action map.
    pub actions: Option<Vec<String>>,
    /// Chance per tick per action that a player changes it. Default 0.05.
    pub input_change: Option<f64>,
    /// A player who joins at this host tick (gets a snapshot).
    pub late_join_at: Option<u64>,
    /// A client slot whose game vanishes at `drop_at`.
    pub drop_slot: Option<u8>,
    pub drop_at: Option<u64>,
}

fn simulate(agent: &mut Agent, input: SimulateInput) -> Result<SimulateReport, Errors> {
    let path = agent.resolve(&input.scene)?;
    let d = SimulateOptions::default();
    let options = SimulateOptions {
        players: input.players.unwrap_or(d.players),
        ticks: input.ticks.unwrap_or(d.ticks),
        seed: input.seed.unwrap_or(d.seed),
        link: input.link.unwrap_or(d.link),
        hash_interval: input.hash_interval.unwrap_or(d.hash_interval),
        actions: input.actions.unwrap_or_default(),
        input_change: input.input_change.unwrap_or(d.input_change),
        late_join_at: input.late_join_at,
        drop_slot: input.drop_slot,
        drop_at: input.drop_at,
    };
    let engine = agent.engine().clone();
    let shown = input.scene.clone();
    let make = |seed: u64| engine.load_file(&path, &shown, seed).map_err(|e| e.to_string());
    marionette_net::simulate(&options, &make)
        .map_err(|m| Errors::from(Diagnostic::error(codes::BAD_INPUT, m).file(&input.scene)))
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckInput {
    /// Definitions file, relative to the project root.
    pub file: String,
}

#[derive(Serialize, JsonSchema)]
pub struct CheckOutput {
    pub achievements: Vec<String>,
    pub stats: Vec<String>,
    /// Achievements unlocked by a stat threshold.
    pub stat_driven: usize,
}

fn check(agent: &mut Agent, input: CheckInput) -> Result<CheckOutput, Errors> {
    let path = agent.resolve(&input.file)?;
    let text = std::fs::read_to_string(&path).map_err(|e| {
        Errors::from(
            Diagnostic::error(codes::BAD_INPUT, format!("cannot read {}: {e}", input.file))
                .file(&input.file),
        )
    })?;
    let defs = marionette_platform::parse_definitions(&text).map_err(|e| {
        Errors::from(Diagnostic::error(e.code, e.message).file(&input.file))
    })?;
    Ok(CheckOutput {
        stat_driven: defs.achievements.iter().filter(|a| a.stat.is_some()).count(),
        achievements: defs.achievements.into_iter().map(|a| a.id).collect(),
        stats: defs.stats.into_iter().map(|s| s.id).collect(),
    })
}
