//! Navigation operations (decision 0231): ask for a route over a `NavSurface` and see how
//! the agents are doing, as numbers.

use crate::observe::{WorldTarget, with_world};
use crate::{Agent, Op, codes, op};
use marionette_core::scene::SceneId;
use marionette_core::{Diagnostic, Errors};
use marionette_nav::{NavAgent, NavAgentState, NavStatus};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub(crate) fn ops() -> Vec<Op> {
    vec![
        op(
            "nav_path",
            "A route over a NavSurface between two world points (session, or a scene after \
             `ticks`): corner points on the ground, length in metres, and whether it reaches \
             `to` (false: it ends at the closest reachable place).",
            false,
            path,
        ),
        op(
            "nav_agents",
            "How the NavAgents are doing: a count per status (Idle, Waiting, Moving, Partial, \
             Arrived, Unreachable), then up to `limit` agents (not arrived first) with \
             status, speed and corners left.",
            false,
            agents,
        ),
    ]
}

#[derive(Deserialize, JsonSchema)]
pub struct PathInput {
    #[serde(flatten)]
    pub target: WorldTarget,
    /// The entity with the NavSurface.
    pub surface: u64,
    pub from: [f32; 3],
    pub to: [f32; 3],
}

#[derive(Serialize, JsonSchema)]
pub struct PathOutput {
    pub corners: Vec<[f32; 3]>,
    pub length: f32,
    pub complete: bool,
}

fn path(agent: &mut Agent, input: PathInput) -> Result<PathOutput, Errors> {
    with_world(agent, &input.target, |_, sim, _| {
        let Some((corners, complete)) =
            marionette_nav::find_path(&sim.world, input.surface, input.from, input.to)
        else {
            return Err(Diagnostic::error(
                codes::BAD_INPUT,
                format!(
                    "entity {} has no built NavSurface (or no open cell near `to`)",
                    input.surface
                ),
            )
            .entity(input.surface)
            .fix("add a NavSurface to the world's entity")
            .into());
        };
        let length = corners
            .windows(2)
            .map(|w| {
                let d = [w[1][0] - w[0][0], w[1][1] - w[0][1], w[1][2] - w[0][2]];
                (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
            })
            .sum();
        Ok(PathOutput {
            corners,
            length,
            complete,
        })
    })
}

#[derive(Deserialize, JsonSchema)]
pub struct AgentsInput {
    #[serde(flatten)]
    pub target: WorldTarget,
    /// Most agents to list. Default 10.
    pub limit: Option<usize>,
}

#[derive(Serialize, JsonSchema)]
pub struct AgentRow {
    pub id: u64,
    pub status: String,
    pub speed: f32,
    pub corners: u32,
}

#[derive(Serialize, JsonSchema)]
pub struct AgentsOutput {
    pub tick: u64,
    /// Status name -> agent count.
    pub counts: std::collections::BTreeMap<String, usize>,
    pub agents: Vec<AgentRow>,
    pub more: usize,
}

fn agents(agent: &mut Agent, input: AgentsInput) -> Result<AgentsOutput, Errors> {
    with_world(agent, &input.target, |_, sim, _| {
        let tick = sim.tick();
        let world = &mut sim.world;
        let mut q = world.query::<(&SceneId, &NavAgent, Option<&NavAgentState>)>();
        let mut rows: Vec<(bool, AgentRow)> = q
            .iter(world)
            .map(|(id, _, st)| {
                let st = st.copied().unwrap_or_default();
                let v = st.velocity;
                (
                    st.status == NavStatus::Arrived,
                    AgentRow {
                        id: id.0,
                        status: format!("{:?}", st.status),
                        speed: (v.x * v.x + v.y * v.y + v.z * v.z).sqrt(),
                        corners: st.corners,
                    },
                )
            })
            .collect();
        rows.sort_by_key(|(arrived, r)| (*arrived, r.id));
        let mut counts = std::collections::BTreeMap::new();
        for (_, r) in &rows {
            *counts.entry(r.status.clone()).or_insert(0) += 1;
        }
        let limit = input.limit.unwrap_or(10);
        let more = rows.len().saturating_sub(limit);
        rows.truncate(limit);
        Ok(AgentsOutput {
            tick,
            counts,
            agents: rows.into_iter().map(|(_, r)| r).collect(),
            more,
        })
    })
}
