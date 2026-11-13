//! The game starter (decision 0150): one prompt to a playable project.
//!
//! The pipeline has three parts, all deterministic and offline:
//!
//! 1. A **brief** ([`Brief`], format `marionette.brief/1`): about 1 KB of JSON naming the
//!    genre template, the title, the art style and one line per asset slot. An agent writes it
//!    from the person's prompt (it is the language model already); [`prompt::brief_from_prompt`]
//!    is the keyword fallback for a prompt with no agent behind it.
//! 2. A **template** ([`template`]): a genre's TypeScript game script (with an autopilot that
//!    plays it), its input map, its scene recipe and its asset slots. The level is generated
//!    from the brief's seed ([`level`]).
//! 3. **Files** ([`template::render`]): the project as plain files (brief, build recipe,
//!    script with a config block, input map, tests, generation recipes, README). The
//!    operation layer (`game_make` in `marionette-agent`) resolves each asset slot through the
//!    asset lookup order, writes the files, builds the scene, and proves the level is winnable
//!    by letting the autopilot play it.

pub mod level;
pub mod prompt;
pub mod template;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Stable error codes of the starter.
pub mod codes {
    /// The brief is malformed or names something that does not exist.
    pub const BAD_BRIEF: &str = "starter.bad_brief";
    /// No generated level could be won by the autopilot within the retry budget.
    pub const UNWINNABLE: &str = "starter.unwinnable";
    /// A refresh found a script without its config markers.
    pub const NO_CONFIG_BLOCK: &str = "starter.no_config_block";
}

pub const BRIEF_FORMAT: &str = "marionette.brief/1";

/// The genre template a game starts from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Template {
    /// Walk a maze, collect every pickup while enemies wander, then reach the goal.
    /// 2D (top-down sprites) or 3D (models, shadows, glow).
    Maze,
    /// Side-scrolling 2D: run and jump over gaps, spikes and walking enemies (stomp them),
    /// collect pickups, reach the goal before time runs out.
    Platformer,
}

impl Template {
    pub const ALL: [Template; 2] = [Template::Maze, Template::Platformer];

    pub fn name(self) -> &'static str {
        match self {
            Template::Maze => "maze",
            Template::Platformer => "platformer",
        }
    }
}

/// 2D sprites or 3D models.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub enum View {
    #[default]
    #[serde(rename = "2d")]
    TwoD,
    #[serde(rename = "3d")]
    ThreeD,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Difficulty {
    Easy,
    #[default]
    Normal,
    Hard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Size {
    Small,
    #[default]
    Medium,
    Large,
}

/// Words shown in the HUD and banners.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Words {
    /// What the pickups are called in the HUD, plural (e.g. "lanterns").
    #[serde(default = "default_pickups")]
    pub pickups: String,
    /// Banner when the player wins.
    #[serde(default = "default_win")]
    pub win: String,
    /// Banner when the player loses.
    #[serde(default = "default_lose")]
    pub lose: String,
}

fn default_pickups() -> String {
    "gems".into()
}
fn default_win() -> String {
    "YOU MADE IT".into()
}
fn default_lose() -> String {
    "GAME OVER".into()
}

impl Default for Words {
    fn default() -> Self {
        Self {
            pickups: default_pickups(),
            win: default_win(),
            lose: default_lose(),
        }
    }
}

/// Colors as `#rrggbb` (sRGB).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Colors {
    /// Clear color behind everything, and the floor tint.
    #[serde(default = "default_background")]
    pub background: String,
    /// HUD text, glows and lights.
    #[serde(default = "default_accent")]
    pub accent: String,
}

fn default_background() -> String {
    "#141a2e".into()
}
fn default_accent() -> String {
    "#ffd166".into()
}

impl Default for Colors {
    fn default() -> Self {
        Self {
            background: default_background(),
            accent: default_accent(),
        }
    }
}

/// One asset slot of the template (player, enemy, pickup, goal, wall, floor, ground,
/// background).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SlotBrief {
    /// What it looks like, in a few words ("a small moth with glowing wings"). Becomes the
    /// generation prompt (with the brief's `style`) when nothing is found.
    #[serde(default)]
    pub describe: String,
    /// Use this asset instead of looking: a project-relative path, or `pool:<id>` for a CC0
    /// pool asset.
    #[serde(default, rename = "use", skip_serializing_if = "Option::is_none")]
    pub use_asset: Option<String>,
    /// Words for the asset lookup order (project files, CC0 pool). Without them the slot
    /// goes straight to generation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<String>,
}

/// A game brief: the whole design of a starter game in about 1 KB.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Brief {
    /// Always `marionette.brief/1`.
    #[serde(default = "brief_format")]
    pub format: String,
    pub title: String,
    /// One sentence: what the player does.
    #[serde(default)]
    pub pitch: String,
    pub template: Template,
    /// `2d` (default) or `3d` (maze only).
    #[serde(default)]
    pub view: View,
    /// Art style added to every generation prompt ("hand-painted, cozy, soft light").
    #[serde(default)]
    pub style: String,
    #[serde(default)]
    pub difficulty: Difficulty,
    #[serde(default)]
    pub size: Size,
    /// Level seed. The same brief always makes the same files.
    #[serde(default = "default_seed")]
    pub seed: u64,
    #[serde(default)]
    pub words: Words,
    #[serde(default)]
    pub colors: Colors,
    /// One entry per slot of the template (`game_brief` lists them); missing slots get a
    /// plain placeholder description.
    #[serde(default)]
    pub assets: BTreeMap<String, SlotBrief>,
}

fn brief_format() -> String {
    BRIEF_FORMAT.into()
}
fn default_seed() -> u64 {
    1
}

impl Brief {
    /// Checks the brief against its template. Returns every problem at once.
    pub fn check(&self) -> Result<(), marionette_core::Errors> {
        use marionette_core::Diagnostic;
        let mut problems = Vec::new();
        let bad = |msg: String, fix: &str| Diagnostic::error(codes::BAD_BRIEF, msg).fix(fix.to_string());
        if self.format != BRIEF_FORMAT {
            problems.push(bad(
                format!("brief format is `{}`", self.format),
                "set \"format\": \"marionette.brief/1\"",
            ));
        }
        if self.title.trim().is_empty() || self.title.chars().count() > 60 {
            problems.push(bad(
                "title must be 1 to 60 characters".into(),
                "give the game a short title",
            ));
        }
        if self.template == Template::Platformer && self.view == View::ThreeD {
            problems.push(bad(
                "the platformer template is 2D only".into(),
                "use \"view\": \"2d\", or the maze template for 3D",
            ));
        }
        let slots = template::slots(self.template, self.view);
        for name in self.assets.keys() {
            if !slots.iter().any(|s| s.name == name) {
                let names: Vec<&str> = slots.iter().map(|s| s.name).collect();
                problems.push(bad(
                    format!("`{name}` is not an asset slot of this template"),
                    &format!("slots: {}", names.join(", ")),
                ));
            }
        }
        for (name, c) in [("background", &self.colors.background), ("accent", &self.colors.accent)] {
            if parse_hex(c).is_none() {
                problems.push(bad(format!("colors.{name} `{c}` is not #rrggbb"), "write it like #ffd166"));
            }
        }
        if self.words.pickups.trim().is_empty() || self.words.pickups.chars().count() > 20 {
            problems.push(bad(
                "words.pickups must be 1 to 20 characters".into(),
                "for example \"lanterns\"",
            ));
        }
        if problems.is_empty() {
            Ok(())
        } else {
            Err(marionette_core::Errors(problems))
        }
    }
}

/// `#rrggbb` (sRGB) to linear RGB in 0..1.
pub fn parse_hex(s: &str) -> Option<[f32; 3]> {
    let h = s.strip_prefix('#')?;
    if h.len() != 6 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let ch = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok().map(|v| srgb_to_linear(v as f32 / 255.0));
    Some([ch(0)?, ch(2)?, ch(4)?])
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// How one slot is drawn, decided by the operation layer after the asset lookup.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Art {
    /// A 2D image. `image` is relative to the scene file. `region` crops it to its subject
    /// (pixels from the top-left: x, y, width, height); `aspect` is width / height of what is
    /// drawn.
    Sprite {
        image: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        region: Option<[u32; 4]>,
        aspect: f32,
    },
    /// A 3D model (glTF), relative to the scene file. Grounded at y = 0 and sized by its
    /// recipe; `scale` multiplies that.
    Model { asset: String, scale: f32 },
    /// A placeholder: a built-in shape in a flat color, until an asset exists.
    Shape { color: [f32; 3] },
}

/// What [`template::render`] needs besides the brief: how each slot is drawn.
pub type ArtMap = BTreeMap<String, Art>;

/// The art of a PNG sprite: crop to the pixels that are not (nearly) transparent, so a
/// generated subject in the middle of a square canvas fills its tile. An opaque image (a
/// tile or a backdrop) is used whole.
pub fn sprite_art(png: &[u8], image: String) -> Option<Art> {
    let img = image::load_from_memory_with_format(png, image::ImageFormat::Png).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return None;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0u32, 0u32);
    for (x, y, p) in img.enumerate_pixels() {
        if p[3] > 24 {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
    }
    if x1 < x0 {
        // Fully transparent: nothing to draw, but keep the file.
        return Some(Art::Sprite {
            image,
            region: None,
            aspect: w as f32 / h as f32,
        });
    }
    let (rw, rh) = (x1 - x0 + 1, y1 - y0 + 1);
    let whole = rw == w && rh == h;
    let aspect = (rw as f32 / rh as f32 * 1000.0).round() / 1000.0;
    Some(Art::Sprite {
        image,
        region: (!whole).then_some([x0, y0, rw, rh]),
        aspect,
    })
}
