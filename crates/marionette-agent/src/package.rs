//! Packaging operations (decision 0140): list package targets and what this machine can build,
//! and build a game into a folder and zip per target (`marionette-package`).

use crate::{Agent, Op, op};
use marionette_core::Errors;
use marionette_package as pkg;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub(crate) fn ops() -> Vec<Op> {
    vec![
        op(
            "package_targets",
            "List package targets (windows-x64, linux-x64, steamdeck, steam-frame) and whether this \
             machine can build each; with `game`, also the files a package of it would hold.",
            false,
            targets,
        ),
        op(
            "package_build",
            "Package a game: build its binary for each target (release, glibc 2.31 floor on Linux), copy \
             its runtime files, licenses and a game.package.json, then write <out>/<game>-<target>/ and \
             a zip. The binary plays the entry scene when started with no arguments. Takes minutes.",
            true,
            build,
        ),
    ]
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TargetsInput {
    /// Game folder (holds marionette.json), relative to the project root.
    #[serde(default)]
    pub game: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct TargetRow {
    pub name: &'static str,
    pub triple: &'static str,
    pub summary: &'static str,
    /// `cargo`, `zigbuild`, or null when this machine cannot build it.
    pub builder: Option<pkg::Builder>,
    /// Why not, and the fix.
    pub missing: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct TargetsOutput {
    pub host: &'static str,
    pub targets: Vec<TargetRow>,
    /// With `game`: what a package would hold.
    pub game: Option<pkg::Game>,
}

fn targets(agent: &mut Agent, input: TargetsInput) -> Result<TargetsOutput, Errors> {
    let game = match &input.game {
        Some(g) => Some(pkg::read_game(&agent.resolve(g)?)?),
        None => None,
    };
    let targets = pkg::TARGETS
        .iter()
        .map(|t| {
            let r = pkg::can_build(t);
            TargetRow {
                name: t.name,
                triple: t.triple,
                summary: t.summary,
                missing: r.as_ref().err().map(|e| {
                    e.0.iter()
                        .map(|d| format!("{} Fix: {}", d.message, d.fix.clone().unwrap_or_default()))
                        .collect::<Vec<_>>()
                        .join("; ")
                }),
                builder: r.ok(),
            }
        })
        .collect();
    Ok(TargetsOutput {
        host: pkg::host_triple(),
        targets,
        game,
    })
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BuildInput {
    /// Game folder (holds marionette.json), relative to the project root.
    pub game: String,
    /// Targets, or `["all"]` for every target this machine can build. Default: this machine's own.
    #[serde(default)]
    pub targets: Vec<String>,
    /// Output folder, relative to the project root.
    #[serde(default = "default_out")]
    pub out: String,
    /// Write a zip next to each folder (default true).
    #[serde(default = "yes")]
    pub archive: bool,
    /// Run the packaged binary headless for a few ticks when this machine can (default true).
    #[serde(default = "yes")]
    pub smoke: bool,
    /// Debug build instead of release (faster to build, slow to play).
    #[serde(default)]
    pub debug: bool,
    /// Link-time sysroots by architecture (`x86_64`, `aarch64`) for zigbuild builds (default:
    /// MARIONETTE_SYSROOT_X86_64 / MARIONETTE_SYSROOT_AARCH64; scripts/package-sysroots.sh makes them).
    #[serde(default)]
    pub sysroots: std::collections::BTreeMap<String, String>,
}

fn default_out() -> String {
    "dist".into()
}

fn yes() -> bool {
    true
}

#[derive(Serialize, JsonSchema)]
pub struct BuildOutput {
    pub packages: Vec<pkg::Built>,
}

fn build(agent: &mut Agent, input: BuildInput) -> Result<BuildOutput, Errors> {
    let names = if input.targets.is_empty() {
        let host = pkg::host_triple();
        let own = pkg::TARGETS
            .iter()
            .find(|t| t.triple == host)
            .map(|t| t.name)
            .unwrap_or("linux-x64");
        vec![own.to_string()]
    } else {
        input.targets.clone()
    };
    let opts = pkg::Options {
        workspace: agent.root().to_path_buf(),
        game: agent.resolve(&input.game)?,
        out: agent.resolve(&input.out)?,
        targets: pkg::targets(&names)?,
        release: !input.debug,
        archive: input.archive,
        smoke: input.smoke,
        sysroots: input.sysroots.into_iter().map(|(k, v)| (k, PathBuf::from(v))).collect(),
    };
    Ok(BuildOutput {
        packages: pkg::package(&opts)?,
    })
}
