//! `providers.json`: which provider (and model) each kind uses, at the project root.
//!
//! ```json
//! {
//!   "format": "marionette.providers/1",
//!   "language": { "provider": "claude", "model": "claude-opus-5-5" },
//!   "image": { "provider": "mock" }
//! }
//! ```
//!
//! A missing file, or a missing kind, means `mock`. Unknown fields are refused, so a key can
//! never be stored here: keys come from environment variables only.

use crate::{Kind, codes};
use marionette_core::{Diagnostic, Errors};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const FILE_NAME: &str = "providers.json";
pub const FORMAT: &str = "marionette.providers/1";

/// One kind's provider and model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Choice {
    /// Provider name (see `provider_list`).
    pub provider: String,
    /// Model name; the provider's default model when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

impl Choice {
    pub fn mock() -> Self {
        Self {
            provider: "mock".into(),
            model: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProvidersConfig {
    /// Always `marionette.providers/1`.
    #[serde(default = "format")]
    pub format: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<Choice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<Choice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model3d: Option<Choice>,
}

fn format() -> String {
    FORMAT.into()
}

impl ProvidersConfig {
    pub fn path(project_root: &Path) -> PathBuf {
        project_root.join(FILE_NAME)
    }

    /// Reads the project's config; a missing file gives the all-mock default.
    pub fn load(project_root: &Path) -> Result<(Self, bool), Errors> {
        let path = Self::path(project_root);
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((Self::new(), false)),
            Err(e) => {
                return Err(Diagnostic::error(
                    marionette_core::error::codes::IO_READ,
                    format!("cannot read {FILE_NAME}: {e}"),
                )
                .file(FILE_NAME)
                .into());
            }
        };
        Self::parse(&text).map(|c| (c, true))
    }

    pub fn new() -> Self {
        Self {
            format: FORMAT.into(),
            ..Default::default()
        }
    }

    pub fn parse(text: &str) -> Result<Self, Errors> {
        let config: Self = serde_json::from_str(text).map_err(|e| {
            Errors::from(
                Diagnostic::error(codes::BAD_CONFIG, format!("{FILE_NAME} is invalid: {e}"))
                    .file(FILE_NAME)
                    .at(Some(e.line()), Some(e.column()))
                    .fix("fields are format, language, image, model3d, each {provider, model}; keys belong in environment variables, never in this file"),
            )
        })?;
        if config.format != FORMAT {
            return Err(Diagnostic::error(
                codes::BAD_CONFIG,
                format!("{FILE_NAME} has format `{}`, expected `{FORMAT}`", config.format),
            )
            .file(FILE_NAME)
            .into());
        }
        Ok(config)
    }

    /// Canonical text: fixed field order, two-space indent, LF, trailing newline.
    pub fn to_text(&self) -> String {
        let mut s = serde_json::to_string_pretty(self).expect("config serializes");
        s.push('\n');
        s
    }

    /// The choice for a kind (`mock` when unset).
    pub fn choice(&self, kind: Kind) -> Choice {
        self.slot(kind).clone().unwrap_or_else(Choice::mock)
    }

    pub fn set(&mut self, kind: Kind, choice: Choice) {
        *self.slot_mut(kind) = Some(choice);
    }

    fn slot(&self, kind: Kind) -> &Option<Choice> {
        match kind {
            Kind::Language => &self.language,
            Kind::Image => &self.image,
            Kind::Model3d => &self.model3d,
        }
    }

    fn slot_mut(&mut self, kind: Kind) -> &mut Option<Choice> {
        match kind {
            Kind::Language => &mut self.language,
            Kind::Image => &mut self.image,
            Kind::Model3d => &mut self.model3d,
        }
    }
}
