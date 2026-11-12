//! Live sessions: a loaded world that stays in memory between calls (start, step, observe,
//! edit, step again). Sessions live as long as the agent process (the MCP server); the CLI
//! starts a fresh process per call, so from the CLI use `scene_run` or the `scene`+`ticks`
//! form of the observe operations instead.

use crate::{Agent, Op, codes, op, write_atomic};
use marionette_core::{Diagnostic, Errors, Sim};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::time::Instant;

pub(crate) fn ops() -> Vec<Op> {
    vec![
        op(
            "session_start",
            "Load a scene into a live headless world (optionally pre-stepped). Returns the session id.",
            true,
            start,
        ),
        op(
            "session_step",
            "Advance a live session by `ticks` fixed ticks. Returns the state hash.",
            true,
            step,
        ),
        op("session_stop", "Discard a live session.", true, stop),
        op("session_list", "List live sessions.", false, list),
        op(
            "session_save",
            "Write a live session's current world to a scene file (default: the file it was loaded from).",
            true,
            save,
        ),
    ]
}

#[derive(Serialize, JsonSchema, Clone)]
pub struct LogEntry {
    pub seq: usize,
    pub tick: u64,
    pub kind: String,
    pub message: String,
}

/// Wall-clock timings of simulation steps. Measured around the simulation, never read by it.
#[derive(Default, Clone, Copy)]
pub struct Stats {
    pub steps: u64,
    pub total_ms: f64,
    pub max_ms: f64,
    pub last_ms: f64,
    pub last_render_ms: Option<f64>,
}

pub struct Session {
    pub sim: Sim,
    pub scene: String,
    pub seed: u64,
    pub logs: Vec<LogEntry>,
    pub stats: Stats,
}

impl Session {
    pub fn log(&mut self, kind: &str, message: String) {
        let seq = self.logs.len();
        let tick = self.sim.tick();
        self.logs.push(LogEntry {
            seq,
            tick,
            kind: kind.into(),
            message,
        });
    }

    pub fn step(&mut self, ticks: u64) {
        for _ in 0..ticks {
            let t = Instant::now();
            self.sim.step();
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            self.stats.steps += 1;
            self.stats.total_ms += ms;
            self.stats.max_ms = self.stats.max_ms.max(ms);
            self.stats.last_ms = ms;
        }
    }
}

pub(crate) fn get_mut<'a>(agent: &'a mut Agent, id: &str) -> Result<&'a mut Session, Errors> {
    let known: Vec<String> = agent.sessions.keys().cloned().collect();
    agent.sessions.get_mut(id).ok_or_else(|| {
        Errors::from(
            Diagnostic::error(codes::NO_SESSION, format!("no live session `{id}`")).fix(
                if known.is_empty() {
                    "start one with session_start".to_string()
                } else {
                    format!("live sessions: {}", known.join(", "))
                },
            ),
        )
    })
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StartInput {
    pub scene: String,
    #[serde(default)]
    pub seed: u64,
    /// Ticks to simulate before returning (default 0).
    #[serde(default)]
    pub ticks: u64,
}

#[derive(Serialize, JsonSchema)]
pub struct StepOutput {
    pub session: String,
    pub tick: u64,
    pub state_hash: String,
}

fn start(agent: &mut Agent, input: StartInput) -> Result<StepOutput, Errors> {
    let sim = crate::project::load_sim(agent, &input.scene, input.seed)?;
    let id = format!("s{}", agent.next_session);
    agent.next_session += 1;
    let mut session = Session {
        sim,
        scene: input.scene.clone(),
        seed: input.seed,
        logs: Vec::new(),
        stats: Stats::default(),
    };
    session.log(
        "start",
        format!("loaded {} with seed {}", input.scene, input.seed),
    );
    session.step(input.ticks);
    let out = StepOutput {
        session: id.clone(),
        tick: session.sim.tick(),
        state_hash: session.sim.state_hash(),
    };
    agent.sessions.insert(id, session);
    Ok(out)
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StepInput {
    pub session: String,
    pub ticks: u64,
}

fn step(agent: &mut Agent, input: StepInput) -> Result<StepOutput, Errors> {
    let s = get_mut(agent, &input.session)?;
    s.step(input.ticks);
    s.log("step", format!("stepped {} ticks", input.ticks));
    Ok(StepOutput {
        session: input.session,
        tick: s.sim.tick(),
        state_hash: s.sim.state_hash(),
    })
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionInput {
    pub session: String,
}

#[derive(Serialize, JsonSchema)]
pub struct StopOutput {
    pub session: String,
    pub stopped: bool,
}

fn stop(agent: &mut Agent, input: SessionInput) -> Result<StopOutput, Errors> {
    get_mut(agent, &input.session)?;
    agent.sessions.remove(&input.session);
    Ok(StopOutput {
        session: input.session,
        stopped: true,
    })
}

#[derive(Serialize, JsonSchema)]
pub struct SessionInfo {
    pub session: String,
    pub scene: String,
    pub seed: u64,
    pub tick: u64,
}

#[derive(Serialize, JsonSchema)]
pub struct ListOutput {
    pub sessions: Vec<SessionInfo>,
}

fn list(agent: &mut Agent, _: crate::project::Empty) -> Result<ListOutput, Errors> {
    Ok(ListOutput {
        sessions: agent
            .sessions
            .iter()
            .map(|(id, s)| SessionInfo {
                session: id.clone(),
                scene: s.scene.clone(),
                seed: s.seed,
                tick: s.sim.tick(),
            })
            .collect(),
    })
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveInput {
    pub session: String,
    /// Scene file to write (default: the file the session was loaded from).
    pub scene: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct SaveOutput {
    pub session: String,
    pub scene: String,
    pub tick: u64,
}

fn save(agent: &mut Agent, input: SaveInput) -> Result<SaveOutput, Errors> {
    let s = get_mut(agent, &input.session)?;
    let scene = input.scene.clone().unwrap_or_else(|| s.scene.clone());
    let text = s.sim.snapshot().to_text();
    let tick = s.sim.tick();
    s.log("save", format!("saved tick {tick} to {scene}"));
    let path = agent.resolve(&scene)?;
    write_atomic(&path, &scene, text.as_bytes())?;
    Ok(SaveOutput {
        session: input.session,
        scene,
        tick,
    })
}
