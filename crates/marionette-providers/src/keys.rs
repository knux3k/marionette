//! Where keys come from: the environment first, then a person's keys file outside every
//! project (decision 0045). The keys file is named by the environment variable
//! `MARIONETTE_KEYS_FILE`, else by `keys_file` in the user config file (the same file that
//! holds `spend_cap_usd`). It holds `NAME=value` lines (`#` comments, optional `export`,
//! optional quotes) or one JSON object of strings.
//!
//! Only the provider key variables and `MARIONETTE_SPEND_CAP_USD` are read from it. Values are
//! never printed or returned by any operation. A keys file inside the project folder is
//! refused, so it cannot be committed by accident. No operation writes it.

use crate::{codes, spend};
use marionette_core::{Diagnostic, Errors};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const KEYS_FILE_ENV: &str = "MARIONETTE_KEYS_FILE";

/// Names the keys file may supply.
pub fn readable_names() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = crate::all().iter().filter_map(|p| p.info().key_env).collect();
    names.push(spend::CAP_ENV);
    names
}

/// The keys file path, if one is configured.
pub fn keys_file_path(var: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    if let Some(p) = var(KEYS_FILE_ENV).filter(|p| !p.trim().is_empty()) {
        return Some(PathBuf::from(p.trim()));
    }
    let cfg = spend::user_config_file(var)?;
    let text = std::fs::read_to_string(cfg).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    v.get("keys_file")
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
}

/// Parses `NAME=value` lines or a JSON object of strings.
pub fn parse(text: &str) -> BTreeMap<String, String> {
    let text = text.trim_start_matches('\u{feff}');
    if text.trim_start().starts_with('{') {
        return serde_json::from_str::<BTreeMap<String, serde_json::Value>>(text)
            .map(|m| {
                m.into_iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k, s.to_string())))
                    .collect()
            })
            .unwrap_or_default();
    }
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line).trim();
        let line = line.strip_prefix("set ").unwrap_or(line).trim();
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let v = v.trim();
        let v = v
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .or_else(|| v.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
            .unwrap_or(v);
        out.insert(k.trim().to_string(), v.to_string());
    }
    out
}

fn inside(path: &Path, root: &Path) -> bool {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    canon(path).starts_with(canon(root))
}

/// Refuses a keys file inside the project folder.
pub fn check_location(path: &Path, project_root: &Path) -> Result<(), Errors> {
    if inside(path, project_root) {
        return Err(Diagnostic::error(
            codes::KEYS_FILE_IN_PROJECT,
            "the keys file is inside the project folder, where it could be committed; it was not read",
        )
        .fix("move the keys file outside every project and point MARIONETTE_KEYS_FILE or `keys_file` at it")
        .into());
    }
    Ok(())
}

/// Looks `name` up in the environment, then in the keys file (only for [`readable_names`]).
/// A keys file inside `project_root` is ignored.
pub fn lookup(var: &dyn Fn(&str) -> Option<String>, project_root: &Path, name: &str) -> Option<String> {
    if let Some(v) = var(name).filter(|v| !v.trim().is_empty()) {
        return Some(v);
    }
    if !readable_names().contains(&name) {
        return None;
    }
    let path = keys_file_path(var)?;
    check_location(&path, project_root).ok()?;
    let text = std::fs::read_to_string(&path).ok()?;
    parse(&text).remove(name).filter(|v| !v.trim().is_empty())
}

/// Whether a keys file is configured, and whether it is usable, without reading values.
pub fn status(var: &dyn Fn(&str) -> Option<String>, project_root: &Path) -> Result<Option<bool>, Errors> {
    let Some(path) = keys_file_path(var) else {
        return Ok(None);
    };
    check_location(&path, project_root)?;
    Ok(Some(path.is_file()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dotenv_and_json() {
        let m = parse(
            "# keys\nexport DASHSCOPE_API_KEY=\"abc\"\nTRIPO_API_KEY = 'x y'\nset OTHER=1\nnot a line\n",
        );
        assert_eq!(m["DASHSCOPE_API_KEY"], "abc");
        assert_eq!(m["TRIPO_API_KEY"], "x y");
        assert_eq!(m["OTHER"], "1");
        let j = parse("\u{feff}{\"TRIPO_API_KEY\": \"t\", \"n\": 1}");
        assert_eq!(j.len(), 1);
        assert_eq!(j["TRIPO_API_KEY"], "t");
    }

    #[test]
    fn environment_wins_and_only_known_names_are_read() {
        let dir = std::env::temp_dir().join(format!("marionette-keys-{}", std::process::id()));
        let project = dir.join("project");
        std::fs::create_dir_all(&project).unwrap();
        let file = dir.join("keys.env");
        std::fs::write(
            &file,
            "TRIPO_API_KEY=from-file\nHOME=nope\nMARIONETTE_SPEND_CAP_USD=10\n",
        )
        .unwrap();
        let f = file.to_string_lossy().to_string();
        let env = |k: &str| (k == KEYS_FILE_ENV).then(|| f.clone());
        assert_eq!(
            lookup(&env, &project, "TRIPO_API_KEY").as_deref(),
            Some("from-file")
        );
        assert_eq!(lookup(&env, &project, spend::CAP_ENV).as_deref(), Some("10"));
        assert_eq!(lookup(&env, &project, "HOME"), None);
        let env2 = |k: &str| match k {
            KEYS_FILE_ENV => Some(f.clone()),
            "TRIPO_API_KEY" => Some("from-env".into()),
            _ => None,
        };
        assert_eq!(
            lookup(&env2, &project, "TRIPO_API_KEY").as_deref(),
            Some("from-env")
        );
        // Inside the project: refused and not read.
        let inner = project.join("keys.env");
        std::fs::write(&inner, "TRIPO_API_KEY=leak\n").unwrap();
        let i = inner.to_string_lossy().to_string();
        let env3 = |k: &str| (k == KEYS_FILE_ENV).then(|| i.clone());
        assert_eq!(lookup(&env3, &project, "TRIPO_API_KEY"), None);
        assert_eq!(code(status(&env3, &project)), codes::KEYS_FILE_IN_PROJECT);
        assert_eq!(status(&env, &project).unwrap(), Some(true));
        assert_eq!(status(&|_| None, &project).unwrap(), None);
    }

    fn code<T>(r: Result<T, Errors>) -> &'static str {
        r.err().unwrap().0[0].code
    }
}
