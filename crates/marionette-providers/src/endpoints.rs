//! Personal provider addresses and models (decision 0046).
//!
//! Some accounts reach a provider through an address of their own (for Qwen, a Model Studio
//! plan address such as `https://token-plan.<region>.maas.aliyuncs.com/compatible-mode/v1`) and
//! only the models that plan offers. Both are facts about one person's account, so they live in
//! the per-user config file, never in a project:
//!
//! ```json
//! "provider_endpoints": {
//!   "qwen": { "url": "C:\\path\\to\\qwen-endpoint.txt", "model": "qwen-image-3.0-pro" }
//! }
//! ```
//!
//! `url` is an `https://` address, or the path of a text file whose first non-empty line is
//! one. A plain string is the same as `{ "url": ... }`. The environment variables
//! `MARIONETTE_<NAME>_ENDPOINT` and `MARIONETTE_<NAME>_MODEL` win over the file.
//!
//! The personal model applies only where the project's `providers.json` names the provider and
//! pins no model, and it is applied before planning, so cache keys and provenance name the model
//! that really ran.

use crate::config::ProvidersConfig;
use crate::{Kind, spend};
use serde_json::Value;

/// One provider's personal address and model.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Personal {
    /// An `https://` address, as written (suffixes are the provider's business).
    pub url: Option<String>,
    pub model: Option<String>,
}

fn env_name(provider: &str, what: &str) -> String {
    format!("MARIONETTE_{}_{what}", provider.to_ascii_uppercase())
}

/// Reads `value` as an address or as a file holding one. Only `https://` addresses count.
fn url_from(value: &str) -> Option<String> {
    let value = value.trim();
    let text;
    let line = if value.starts_with("https://") {
        value
    } else {
        text = std::fs::read_to_string(value).ok()?;
        text.trim_start_matches('\u{feff}')
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty() && !l.starts_with('#'))?
    };
    (line.starts_with("https://") && !line.contains(char::is_whitespace)).then(|| line.to_string())
}

fn non_empty(s: Option<String>) -> Option<String> {
    s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// The personal address and model for `provider` (see the module docs).
pub fn personal(var: &dyn Fn(&str) -> Option<String>, provider: &str) -> Personal {
    let file: Option<Value> = spend::user_config_file(var)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str::<Value>(t.trim_start_matches('\u{feff}')).ok())
        .and_then(|v| v.pointer(&format!("/provider_endpoints/{provider}")).cloned());
    let (file_url, file_model) = match &file {
        Some(Value::String(s)) => (Some(s.clone()), None),
        Some(v) => (
            v.get("url").and_then(Value::as_str).map(str::to_string),
            v.get("model").and_then(Value::as_str).map(str::to_string),
        ),
        None => (None, None),
    };
    let url = non_empty(var(&env_name(provider, "ENDPOINT")))
        .or(non_empty(file_url))
        .and_then(|u| url_from(&u));
    let model = non_empty(var(&env_name(provider, "MODEL"))).or(non_empty(file_model));
    Personal { url, model }
}

/// `config` with each kind's personal model filled in where the project pins none.
pub fn apply_models(mut config: ProvidersConfig, var: &dyn Fn(&str) -> Option<String>) -> ProvidersConfig {
    for kind in [Kind::Language, Kind::Image, Kind::Model3d] {
        let mut choice = config.choice(kind);
        if choice.model.is_none()
            && choice.provider != "mock"
            && let Some(m) = personal(var, &choice.provider).model
        {
            choice.model = Some(m);
            config.set(kind, choice);
        }
    }
    config
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Choice;

    fn with_config(json: &str) -> (tempdir::Dir, impl Fn(&str) -> Option<String> + use<>) {
        let dir = tempdir::Dir::new();
        let cfg = dir.0.join("marionette");
        std::fs::create_dir_all(&cfg).unwrap();
        std::fs::write(cfg.join("config.json"), json).unwrap();
        let base = dir.0.to_string_lossy().to_string();
        let var = move |n: &str| match n {
            "APPDATA" | "XDG_CONFIG_HOME" => Some(base.clone()),
            _ => None,
        };
        (dir, var)
    }

    mod tempdir {
        pub struct Dir(pub std::path::PathBuf);
        impl Dir {
            pub fn new() -> Self {
                use std::sync::atomic::{AtomicU32, Ordering};
                static N: AtomicU32 = AtomicU32::new(0);
                let p = std::env::temp_dir().join(format!(
                    "marionette-endpoints-{}-{}",
                    std::process::id(),
                    N.fetch_add(1, Ordering::Relaxed)
                ));
                std::fs::create_dir_all(&p).unwrap();
                Self(p)
            }
        }
        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    #[test]
    fn an_address_can_come_from_a_file_next_to_the_key() {
        let (dir, _) = with_config("{}");
        let file = dir.0.join("qwen-endpoint.txt");
        std::fs::write(&file, "\u{feff}\n https://plan.example/compatible-mode/v1 \n").unwrap();
        let json = serde_json::json!({"provider_endpoints": {"qwen": {
            "url": file.to_string_lossy(), "model": "qwen-image-3.0-pro"}}});
        let (_d2, var) = with_config(&json.to_string());
        let p = personal(&var, "qwen");
        assert_eq!(p.url.as_deref(), Some("https://plan.example/compatible-mode/v1"));
        assert_eq!(p.model.as_deref(), Some("qwen-image-3.0-pro"));
        assert_eq!(personal(&var, "tripo"), Personal::default());
    }

    #[test]
    fn only_https_addresses_count_and_the_environment_wins() {
        let (_d, var) = with_config(r#"{"provider_endpoints": {"qwen": "http://plain.example"}}"#);
        assert_eq!(personal(&var, "qwen").url, None);
        let (_d, base) = with_config(r#"{"provider_endpoints": {"qwen": "https://file.example"}}"#);
        let var = |n: &str| match n {
            "MARIONETTE_QWEN_ENDPOINT" => Some("https://env.example".to_string()),
            other => base(other),
        };
        assert_eq!(personal(&var, "qwen").url.as_deref(), Some("https://env.example"));
    }

    #[test]
    fn a_personal_model_fills_only_an_unpinned_choice() {
        let (_d, var) = with_config(r#"{"provider_endpoints": {"qwen": {"model": "plan-model"}}}"#);
        let mut cfg = ProvidersConfig::new();
        cfg.set(
            Kind::Image,
            Choice {
                provider: "qwen".into(),
                model: None,
            },
        );
        let out = apply_models(cfg.clone(), &var);
        assert_eq!(out.choice(Kind::Image).model.as_deref(), Some("plan-model"));
        cfg.set(
            Kind::Image,
            Choice {
                provider: "qwen".into(),
                model: Some("pinned".into()),
            },
        );
        assert_eq!(
            apply_models(cfg, &var).choice(Kind::Image).model.as_deref(),
            Some("pinned")
        );
    }
}
