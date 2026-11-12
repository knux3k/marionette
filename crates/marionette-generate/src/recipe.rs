//! Recipes: `<name>.recipe.json`, one per generated asset, in the generated-assets folder
//! (default `assets/generated/`). A recipe says what to ask for and how to post-process it;
//! the output, its provenance record and (for images) the person's approval sit beside it.
//!
//! ```json
//! {
//!   "format": "marionette.recipe/1",
//!   "name": "crate_ref",
//!   "output": "image",
//!   "prompt": "a wooden crate, three-quarter view, plain background",
//!   "size": [1024, 1024]
//! }
//! ```

use crate::codes;
use marionette_core::{Diagnostic, Errors};
use marionette_providers::{Choice, Kind, MAX_IMAGE_SIDE, MIN_IMAGE_SIDE};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const FORMAT: &str = "marionette.recipe/1";
pub const SUFFIX: &str = ".recipe.json";
/// Default folder for recipes and their outputs, relative to the project root.
pub const DEFAULT_DIR: &str = "assets/generated";
/// Default image request size when a recipe gives none.
pub const DEFAULT_SIZE: [u32; 2] = [1024, 1024];
/// Largest model height a recipe may ask for, in meters.
pub const MAX_HEIGHT_M: f64 = 1000.0;

/// What a recipe produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OutputKind {
    /// A PNG as generated (concept art, a reference image for a model, a sprite).
    Image,
    /// A PNG texture for materials or sprites: optionally resized, made tileable or given a
    /// transparent background.
    Texture,
    /// A GLB model: from a prompt, from an approved image recipe (`from`), or both.
    Model,
}

impl OutputKind {
    pub fn provider_kind(self) -> Kind {
        match self {
            OutputKind::Image | OutputKind::Texture => Kind::Image,
            OutputKind::Model => Kind::Model3d,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            OutputKind::Image | OutputKind::Texture => "png",
            OutputKind::Model => "glb",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            OutputKind::Image => "image",
            OutputKind::Texture => "texture",
            OutputKind::Model => "model",
        }
    }
}

/// Texture post-processing.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextureOptions {
    /// Final size `[width, height]`; each side a power of two from 16 to 4096. Default: each
    /// side of the generated image rounded down to a power of two.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resize: Option<[u32; 2]>,
    /// Blend the edges so the texture repeats without seams.
    #[serde(default, skip_serializing_if = "is_false")]
    pub tileable: bool,
    /// Make the flat background transparent (a sprite generated on one plain colour): the
    /// colour is read from the four corners, pixels near it become transparent with a soft
    /// edge, and the background tint is taken out of the edge pixels. Runs after `resize`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub remove_background: bool,
}

/// Model post-processing.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelOptions {
    /// Scale uniformly so the model is this tall, in meters (engine units). Default: keep the
    /// generated size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height_m: Option<f64>,
    /// Triangle budget. Exceeding it is reported in the provenance notes (no decimation yet).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_triangles: Option<u64>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn format() -> String {
    FORMAT.into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    /// Always `marionette.recipe/1`.
    #[serde(default = "format")]
    pub format: String,
    /// File stem of the recipe and its output: lowercase letters, digits, `_` and `-`.
    pub name: String,
    pub output: OutputKind,
    /// What to generate. Required for image and texture; for a model, a prompt, `from`, or both.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// Image and texture: requested size `[width, height]` (default 1024 x 1024).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<[u32; 2]>,
    /// Texture only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture: Option<TextureOptions>,
    /// Model only: the name of an image or texture recipe in the same folder whose output is
    /// the reference image. For a paid model provider, a person must approve that image first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// Model only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelOptions>,
    /// Pins the provider (and model) for this recipe. Default: the project's `providers.json`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<Choice>,
    /// Free text for people (not sent to the provider).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

fn bad(file: &str, msg: impl Into<String>, fix: &str) -> Errors {
    Diagnostic::error(codes::BAD_RECIPE, msg)
        .file(file)
        .fix(fix)
        .into()
}

/// A valid recipe name: 1 to 64 of `a-z 0-9 _ -`, starting with a letter or digit.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
        && name.as_bytes()[0].is_ascii_alphanumeric()
}

impl Recipe {
    pub fn file_name(&self) -> String {
        format!("{}{SUFFIX}", self.name)
    }

    pub fn output_file_name(&self) -> String {
        format!("{}.{}", self.name, self.output.extension())
    }

    /// Requested image size.
    pub fn size(&self) -> [u32; 2] {
        self.size.unwrap_or(DEFAULT_SIZE)
    }

    /// Checks fields that do not need other recipes. `file` is used in diagnostics.
    pub fn validate(&self, file: &str) -> Result<(), Errors> {
        if self.format != FORMAT {
            return Err(bad(
                file,
                format!("format is `{}`, expected `{FORMAT}`", self.format),
                "set \"format\": \"marionette.recipe/1\"",
            ));
        }
        if !valid_name(&self.name) {
            return Err(bad(
                file,
                format!("name `{}` is not a valid recipe name", self.name),
                "use 1 to 64 lowercase letters, digits, `_` or `-`, starting with a letter or digit",
            ));
        }
        let prompt = self.prompt.as_deref().map(str::trim).filter(|p| !p.is_empty());
        let only = |field: &str, kinds: &str| {
            bad(
                file,
                format!("`{field}` applies to {kinds} recipes only"),
                &format!("remove `{field}` or change `output`"),
            )
        };
        match self.output {
            OutputKind::Image | OutputKind::Texture => {
                if prompt.is_none() {
                    return Err(bad(
                        file,
                        "an image or texture recipe needs a prompt",
                        "add \"prompt\"",
                    ));
                }
                if self.from.is_some() {
                    return Err(only("from", "model"));
                }
                if self.model.is_some() {
                    return Err(only("model", "model"));
                }
                if self.output == OutputKind::Image && self.texture.is_some() {
                    return Err(only("texture", "texture"));
                }
                for v in self.size() {
                    if !(MIN_IMAGE_SIDE..=MAX_IMAGE_SIDE).contains(&v) {
                        return Err(bad(
                            file,
                            format!("size {v} is outside {MIN_IMAGE_SIDE}..={MAX_IMAGE_SIDE}"),
                            "use a size in range",
                        ));
                    }
                }
                if let Some([w, h]) = self.texture.as_ref().and_then(|t| t.resize) {
                    for v in [w, h] {
                        if !v.is_power_of_two() || !(16..=4096).contains(&v) {
                            return Err(bad(
                                file,
                                format!("texture.resize side {v} is not a power of two from 16 to 4096"),
                                "use sizes like 256, 512, 1024 or 2048",
                            ));
                        }
                    }
                }
            }
            OutputKind::Model => {
                if prompt.is_none() && self.from.is_none() {
                    return Err(bad(
                        file,
                        "a model recipe needs a prompt, `from` (an image recipe), or both",
                        "add \"prompt\" or \"from\"",
                    ));
                }
                if self.size.is_some() {
                    return Err(only("size", "image and texture"));
                }
                if self.texture.is_some() {
                    return Err(only("texture", "texture"));
                }
                if let Some(from) = &self.from
                    && (!valid_name(from) || from == &self.name)
                {
                    return Err(bad(
                        file,
                        format!("`from` names `{from}`, which is not another recipe name"),
                        "name an image or texture recipe in the same folder",
                    ));
                }
                if let Some(h) = self.model.as_ref().and_then(|m| m.height_m)
                    && !(h.is_finite() && h > 0.0 && h <= MAX_HEIGHT_M)
                {
                    return Err(bad(
                        file,
                        format!("model.height_m {h} is outside (0, {MAX_HEIGHT_M}]"),
                        "give a height in meters",
                    ));
                }
            }
        }
        if let Some(c) = &self.provider {
            let p = marionette_providers::find(&c.provider)?;
            if !p.info().supports(self.output.provider_kind()) {
                return Err(bad(
                    file,
                    format!(
                        "provider `{}` does not offer {} (needed for a {} recipe)",
                        c.provider,
                        self.output.provider_kind().as_str(),
                        self.output.as_str()
                    ),
                    "pick a provider from `provider_list` that offers this kind, or remove `provider`",
                ));
            }
        }
        Ok(())
    }

    /// Canonical file text: pretty JSON with a trailing newline.
    pub fn to_text(&self) -> String {
        let mut s = serde_json::to_string_pretty(self).expect("recipes serialize");
        s.push('\n');
        s
    }

    pub fn parse(text: &str, file: &str) -> Result<Recipe, Errors> {
        let r: Recipe = serde_json::from_str(text).map_err(|e| {
            Errors::from(
                Diagnostic::error(codes::BAD_RECIPE, format!("{file}: {e}"))
                    .file(file)
                    .at(Some(e.line()), Some(e.column()))
                    .fix("see the recipe format in docs/ai/GENERATION.md"),
            )
        })?;
        r.validate(file)?;
        Ok(r)
    }
}

/// A recipe and where it was read from.
#[derive(Debug, Clone)]
pub struct Loaded {
    pub recipe: Recipe,
    pub path: PathBuf,
    /// SHA-256 of the file bytes.
    pub sha256: String,
}

/// Reads every `*.recipe.json` in `dir`, sorted by name. A missing folder has no recipes. File
/// stems must match the `name` field. Every `from` must name an image or texture recipe.
pub fn load_dir(dir: &Path, shown_dir: &str) -> Result<Vec<Loaded>, Errors> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(Diagnostic::error(
                marionette_core::error::codes::IO_READ,
                format!("cannot read {shown_dir}: {e}"),
            )
            .into());
        }
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().ends_with(SUFFIX))
        })
        .collect();
    paths.sort();
    let mut out = Vec::new();
    let mut errors = Vec::new();
    for path in paths {
        let fname = path.file_name().unwrap().to_string_lossy().to_string();
        let shown = format!("{shown_dir}/{fname}");
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                errors.push(Diagnostic::error(
                    marionette_core::error::codes::IO_READ,
                    format!("cannot read {shown}: {e}"),
                ));
                continue;
            }
        };
        let text = String::from_utf8_lossy(&bytes);
        match Recipe::parse(&text, &shown) {
            Ok(recipe) => {
                if fname != recipe.file_name() {
                    errors.extend(
                        bad(
                            &shown,
                            format!(
                                "file is named `{fname}` but the recipe's name is `{}`",
                                recipe.name
                            ),
                            "rename the file to `<name>.recipe.json`",
                        )
                        .0,
                    );
                    continue;
                }
                out.push(Loaded {
                    recipe,
                    sha256: crate::sha256_hex(&bytes),
                    path,
                });
            }
            Err(e) => errors.extend(e.0),
        }
    }
    for l in &out {
        if let Some(from) = &l.recipe.from {
            match out.iter().find(|o| &o.recipe.name == from) {
                Some(o) if o.recipe.output != OutputKind::Model => {}
                Some(_) => errors.extend(
                    bad(
                        &l.recipe.file_name(),
                        format!("`from` names `{from}`, which is a model recipe"),
                        "name an image or texture recipe",
                    )
                    .0,
                ),
                None => errors.extend(
                    bad(
                        &l.recipe.file_name(),
                        format!("`from` names `{from}`, but there is no `{from}{SUFFIX}` in {shown_dir}"),
                        "write that image recipe first, or fix the name",
                    )
                    .0,
                ),
            }
        }
    }
    if errors.is_empty() {
        Ok(out)
    } else {
        Err(Errors(errors))
    }
}
