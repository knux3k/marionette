//! Which game files go into a package, and the files the package tool writes itself.

use crate::{Errors, Game, Target, codes, io};
use marionette_core::Diagnostic;
use schemars::JsonSchema;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::Path;

/// Paths left out of every package (globs relative to the game folder): tests, build and
/// generation recipes, provider settings, and the generation bookkeeping next to generated
/// assets. A game adds its own with `package.exclude`.
pub const DEFAULT_EXCLUDES: &[&str] = &[
    "tests/**",
    "build.ops.json",
    "providers.json",
    "README.md",
    "**/*.recipe.json",
    "**/*.approval.json",
    "**/.*",
    "**/.*/**",
];

/// Left out when the game has its own Rust package: its sources and build output.
const RUST_EXCLUDES: &[&str] = &["Cargo.toml", "Cargo.lock", "src/**", "target/**", "build.rs"];

/// Matches `path` (with `/` separators) against a glob: `*` matches within one path part,
/// `**` matches any number of parts (including none), `?` one character.
pub fn glob_match(pattern: &str, path: &str) -> bool {
    let pat: Vec<&str> = pattern.split('/').collect();
    let parts: Vec<&str> = path.split('/').collect();
    match_parts(&pat, &parts)
}

fn match_parts(pat: &[&str], parts: &[&str]) -> bool {
    match pat.split_first() {
        None => parts.is_empty(),
        Some((&"**", rest)) => (0..=parts.len()).any(|i| match_parts(rest, &parts[i..])),
        Some((p, rest)) => match parts.split_first() {
            Some((first, tail)) => match_one(p.as_bytes(), first.as_bytes()) && match_parts(rest, tail),
            None => false,
        },
    }
}

fn match_one(p: &[u8], s: &[u8]) -> bool {
    match p.split_first() {
        None => s.is_empty(),
        Some((b'*', rest)) => (0..=s.len()).any(|i| match_one(rest, &s[i..])),
        Some((b'?', rest)) => !s.is_empty() && match_one(rest, &s[1..]),
        Some((c, rest)) => s.first() == Some(c) && match_one(rest, &s[1..]),
    }
}

/// The game's runtime files, relative to `dir`, sorted.
pub(crate) fn runtime_files(dir: &Path, extra: &[String], rust: bool) -> Result<Vec<String>, Errors> {
    let mut patterns: Vec<&str> = DEFAULT_EXCLUDES.to_vec();
    if rust {
        patterns.extend_from_slice(RUST_EXCLUDES);
    }
    patterns.extend(extra.iter().map(String::as_str));
    let mut out = Vec::new();
    walk(dir, "", &mut out)?;
    out.retain(|f| !patterns.iter().any(|p| glob_match(p, f)));
    out.sort();
    Ok(out)
}

fn walk(root: &Path, rel: &str, out: &mut Vec<String>) -> Result<(), Errors> {
    let here = if rel.is_empty() { root.to_path_buf() } else { root.join(rel) };
    let entries = std::fs::read_dir(&here).map_err(|e| io(&here, e))?;
    for entry in entries {
        let entry = entry.map_err(|e| io(&here, e))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = if rel.is_empty() { name } else { format!("{rel}/{name}") };
        let kind = entry.file_type().map_err(|e| io(&entry.path(), e))?;
        if kind.is_dir() {
            walk(root, &path, out)?;
        } else if kind.is_file() {
            out.push(path);
        } else {
            return Err(Diagnostic::error(
                codes::GAME,
                format!("`{path}` is a link or special file; packages hold plain files only"),
            )
            .fix("replace it with a copy of the file, or exclude it with package.exclude")
            .into());
        }
    }
    Ok(())
}

/// Whether a file is text the engine reads (scenes, maps, scripts) and may name assets in.
pub(crate) fn is_text(path: &str) -> bool {
    [".json", ".ts", ".js", ".txt", ".md"].iter().any(|e| path.ends_with(e))
}

/// Removes from `files` every non-text file under `assets/` whose file name no text file
/// names, together with its `<stem>.provenance.json`; returns what was removed.
pub(crate) fn drop_unreferenced(dir: &Path, files: &mut Vec<String>) -> Vec<String> {
    let text: String = files
        .iter()
        .filter(|f| is_text(f) && !f.ends_with(".provenance.json"))
        .filter_map(|f| std::fs::read_to_string(dir.join(f)).ok())
        .collect::<Vec<_>>()
        .join("\n");
    let mut gone: Vec<String> = Vec::new();
    for f in files.iter() {
        if !f.starts_with("assets/") || is_text(f) {
            continue;
        }
        let name = f.rsplit('/').next().unwrap_or(f);
        if !text.contains(name) {
            gone.push(f.clone());
        }
    }
    let stems: Vec<String> = gone
        .iter()
        .filter_map(|f| f.rsplit_once('.').map(|(stem, _)| format!("{stem}.provenance.json")))
        .collect();
    let all_assets_of = |prov: &str| {
        // Keep a provenance file while another asset with the same stem stays.
        let stem = prov.trim_end_matches(".provenance.json");
        !files.iter().any(|f| !gone.contains(f) && f != prov && f.rsplit_once('.').is_some_and(|(s, _)| s == stem))
    };
    let extra: Vec<String> = stems.into_iter().filter(|p| files.contains(p) && all_assets_of(p)).collect();
    gone.extend(extra);
    files.retain(|f| !gone.contains(f));
    gone.sort();
    gone
}

/// One file in a package, as listed in the manifest.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PackageFile {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

/// Every file under `dir` with size and SHA-256, sorted by path.
pub(crate) fn list(dir: &Path) -> Result<Vec<PackageFile>, Errors> {
    let mut paths = Vec::new();
    walk(dir, "", &mut paths)?;
    paths.sort();
    paths
        .into_iter()
        .map(|p| {
            let bytes = std::fs::read(dir.join(&p)).map_err(|e| io(&dir.join(&p), e))?;
            let hash = Sha256::digest(&bytes);
            Ok(PackageFile {
                path: p,
                bytes: bytes.len() as u64,
                sha256: hash.iter().map(|b| format!("{b:02x}")).collect(),
            })
        })
        .collect()
}

/// Writes a file and marks it executable (Unix).
pub(crate) fn write_executable(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

/// Removes staging folders that a crashed package run left in `out`.
pub(crate) fn sweep_partials(out: &Path) {
    let Ok(entries) = std::fs::read_dir(out) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if let Some(pid) = name.rsplit_once(".partial-").map(|(_, p)| p.to_string())
            && name.starts_with('.')
            && pid.parse::<u32>().ok() != Some(std::process::id())
        {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

/// The player-facing README of a package.
pub(crate) fn readme(game: &Game, target: &Target, bin: &str) -> String {
    let title = game.settings.title.clone().unwrap_or_else(|| game.name.clone());
    let mut s = format!(
        "{title}\n\nMade with the Marionette engine. Package target: {} ({}).\n\n",
        target.name, target.summary
    );
    match target.name {
        "windows-x64" => s += &format!("Play: double-click {bin}.\n"),
        _ => s += &format!("Play: run ./{bin} (or double-click it in a file manager).\n"),
    }
    s += "Needs a GPU with Vulkan (Linux), or DirectX 12 or Vulkan (Windows).\n\
          Escape closes the game.\n\n";
    match target.name {
        "steamdeck" => {
            s += &format!(
                "Steam Deck\n\
                 - As a non-Steam game: copy this folder to the Deck, add {bin} with \"Add a Non-Steam Game\" in desktop mode.\n\
                 - Through the SteamOS Devkit Client: Start Command `{bin}`, Runtime \"Steam Linux Runtime 3.0 (sniper)\".\n\
                 - On Steam: Linux depot with this folder, launch option `{bin}`, Linux Runtime \"Steam Linux Runtime 3.0 (sniper)\".\n\
                 - Controls are keyboard and mouse: pick the Steam Input template \"Keyboard (WASD) and Mouse\".\n"
            )
        }
        "steam-frame" => {
            s += &format!(
                "Steam Frame (Linux ARM64)\n\
                 - Through the SteamOS Devkit Client: Title Upload with this folder, Start Command `{bin}`, Runtime \"Steam Linux Runtime 3.0 ARM64 (sniper)\".\n\
                 - The game runs as a flat window; VR needs the engine's OpenXR module, which does not exist yet.\n\
                 - Controls are keyboard and mouse: map the controllers with Steam Input.\n"
            )
        }
        _ => {}
    }
    s += "\nLicenses: see the licenses folder.\n";
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs_match_parts_and_any_depth() {
        assert!(glob_match("tests/**", "tests/a/b.json"));
        assert!(glob_match("**/*.recipe.json", "assets/generated/hero.recipe.json"));
        assert!(glob_match("**/*.recipe.json", "x.recipe.json"));
        assert!(!glob_match("*.json", "scenes/main.scene.json"));
        assert!(glob_match("scenes/*.json", "scenes/main.scene.json"));
        assert!(glob_match("**/.*", "assets/.cache"));
        assert!(!glob_match("**/.*", "assets/hero.glb"));
        assert!(glob_match("a?c", "abc"));
    }
}
