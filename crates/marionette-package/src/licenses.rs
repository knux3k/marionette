//! The licenses folder of a package: the engine's two licenses and every linked Rust crate
//! with its license and license files (from `cargo metadata`, for the target's platform and
//! the features the binary is built with). Identical license texts are written once.

use crate::{Errors, Game, Options, Target, build_error, io, toolchain};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;

/// Returns how many third-party crates were listed.
pub(crate) fn write(opts: &Options, game: &Game, target: &Target, dir: &Path) -> Result<usize, Errors> {
    std::fs::create_dir_all(dir).map_err(|e| io(dir, e))?;
    for name in ["LICENSE-MIT", "LICENSE-APACHE"] {
        let from = opts.workspace.join(name);
        if from.is_file() {
            std::fs::copy(&from, dir.join(format!("marionette-{name}"))).map_err(|e| io(&from, e))?;
        }
    }
    let mut cmd = Command::new("cargo");
    cmd.current_dir(&opts.workspace).args([
        "metadata",
        "--locked",
        "--format-version",
        "1",
        "--filter-platform",
        target.triple,
    ]);
    // Features unify across the workspace in `cargo metadata`, so the list can name a few
    // crates the binary leaves out: more attribution than needed, never less.
    let root_name = match &game.rust_package {
        Some(pkg) => {
            cmd.arg("--manifest-path").arg(opts.game.join("Cargo.toml"));
            pkg.clone()
        }
        None => "marionette-cli".to_string(),
    };
    let out = marionette_host::output(cmd).map_err(|e| toolchain(format!("cannot run cargo: {e}")))?;
    if !out.status.success() {
        return Err(build_error("cargo metadata", &out.stderr));
    }
    let meta: Value = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
    let packages: BTreeMap<&str, &Value> = meta["packages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| Some((p["id"].as_str()?, p)))
        .collect();
    let nodes: BTreeMap<&str, &Value> = meta["resolve"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|n| Some((n["id"].as_str()?, n)))
        .collect();
    let Some(root) = packages
        .iter()
        .find(|(_, p)| p["name"] == root_name.as_str() && p["source"].is_null())
        .map(|(id, _)| *id)
    else {
        return Err(toolchain(format!("cargo metadata did not list `{root_name}`")));
    };
    // Walk normal and build dependencies (not dev) from the root.
    let mut seen = BTreeSet::new();
    let mut todo = vec![root];
    while let Some(id) = todo.pop() {
        if !seen.insert(id) {
            continue;
        }
        let Some(node) = nodes.get(id) else { continue };
        for dep in node["deps"].as_array().into_iter().flatten() {
            let runtime = dep["dep_kinds"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|k| k["kind"].is_null() || k["kind"] == "build");
            if runtime && let Some(d) = dep["pkg"].as_str() {
                todo.push(d);
            }
        }
    }
    let mut list = String::from(
        "Rust crates linked into this game's binary, with their licenses.\n\
         License texts follow the list; identical texts appear once.\n\n",
    );
    let mut texts: BTreeMap<String, (String, Vec<String>)> = BTreeMap::new();
    let mut count = 0;
    for id in &seen {
        let Some(p) = packages.get(id) else { continue };
        if p["source"].is_null() {
            continue; // the engine's own crates: covered by the marionette license files
        }
        count += 1;
        let name = p["name"].as_str().unwrap_or("?");
        let version = p["version"].as_str().unwrap_or("?");
        let license = p["license"].as_str().unwrap_or("see license file");
        let _ = writeln!(list, "{name} {version}: {license}");
        let Some(manifest) = p["manifest_path"].as_str() else {
            continue;
        };
        let Some(crate_dir) = Path::new(manifest).parent() else {
            continue;
        };
        let Ok(entries) = std::fs::read_dir(crate_dir) else {
            continue;
        };
        let mut found: Vec<_> = entries
            .flatten()
            .filter(|e| {
                let n = e.file_name().to_string_lossy().to_ascii_uppercase();
                (n.starts_with("LICENSE")
                    || n.starts_with("LICENCE")
                    || n.starts_with("COPYING")
                    || n.starts_with("NOTICE"))
                    && e.file_type().is_ok_and(|t| t.is_file())
            })
            .map(|e| e.path())
            .collect();
        found.sort();
        for f in found {
            let Ok(text) = std::fs::read_to_string(&f) else {
                continue;
            };
            let key: String = Sha256::digest(text.trim().as_bytes())
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            let label = format!(
                "{name} {version} ({})",
                f.file_name().unwrap_or_default().to_string_lossy()
            );
            texts
                .entry(key)
                .or_insert_with(|| (text, Vec::new()))
                .1
                .push(label);
        }
    }
    for (text, users) in texts.values() {
        let _ = write!(
            list,
            "\n{}\nUsed by: {}\n{}\n\n{}\n",
            "=".repeat(72),
            users.join(", "),
            "-".repeat(72),
            text.trim_end()
        );
    }
    let path = dir.join("THIRD-PARTY.txt");
    std::fs::write(&path, list).map_err(|e| io(&path, e))?;
    Ok(count)
}
