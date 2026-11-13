//! The editor view as an operation (decision 0130): the picture the scene editor window would
//! show, rendered offscreen, plus what is under given pixels and the inspector for one
//! entity. Lets an agent check what a person sees in `marionette edit` without a screen.

use crate::observe::{WorldTarget, parse_size, with_world};
use crate::{Agent, Op, op};
use glam::Vec3;
use marionette_assets::encode_png;
use marionette_core::Errors;
use marionette_editor::view::{self, ViewRequest};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::Path;

/// Most hits listed per picked pixel.
const HITS_KEEP: usize = 8;

pub(crate) fn ops() -> Vec<Op> {
    vec![op(
        "editor_view",
        "The scene editor's view without a window: render it (panels, selection box, grid) to a \
         PNG, list the entities under given pixels nearest first, and return the inspector's \
         components for `select`. Camera: `eye` and `look_at`, else the scene's first camera; \
         `frame` fits the selection (or the scene).",
        false,
        editor_view,
    )]
}

#[derive(Deserialize, JsonSchema)]
pub struct EditorViewInput {
    #[serde(flatten)]
    pub target: WorldTarget,
    /// Image size `WIDTHxHEIGHT` (default 1280x720; pick pixels use the same size).
    pub size: Option<String>,
    /// MSAA samples, 1 or 4 (default 4).
    pub msaa: Option<u32>,
    /// Editor camera position.
    pub eye: Option<[f32; 3]>,
    /// Point the editor camera looks at (with `eye`; default: 1 m ahead along -Z).
    pub look_at: Option<[f32; 3]>,
    /// Move the camera so the selection (or the whole scene) fills the view.
    #[serde(default)]
    pub frame: bool,
    /// Entity to select (highlighted, boxed and shown in the inspector).
    pub select: Option<u64>,
    /// Show the entity list and inspector panels (default true).
    pub panels: Option<bool>,
    /// Show the ground grid (default true).
    pub grid: Option<bool>,
    /// Pixels to pick at, `[x, y]` with y down.
    #[serde(default)]
    pub pick: Vec<[f32; 2]>,
    /// Write the image (default true). With false only picks and the inspector come back.
    pub image: Option<bool>,
    /// Output PNG path (default `.marionette/screenshots/<name>-editor.png`).
    pub out: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct PickedEntity {
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Distance from the camera along the pick ray, in meters.
    pub distance: f64,
}

#[derive(Serialize, JsonSchema)]
pub struct Pick {
    pub x: f32,
    pub y: f32,
    /// Nearest first; at most 8.
    pub hits: Vec<PickedEntity>,
}

#[derive(Serialize, JsonSchema)]
pub struct Selected {
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub components: Map<String, Value>,
}

#[derive(Serialize, JsonSchema)]
pub struct EditorViewOutput {
    /// PNG path, when an image was written.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub width: u32,
    pub height: u32,
    pub tick: u64,
    pub entities: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub picks: Vec<Pick>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected: Option<Selected>,
    /// Where the editor camera ended up (useful after `frame`): position and look direction.
    pub camera: [[f64; 3]; 2],
}

fn round3(v: Vec3) -> [f64; 3] {
    let r = |x: f32| (x as f64 * 1000.0).round() / 1000.0;
    [r(v.x), r(v.y), r(v.z)]
}

fn editor_view(agent: &mut Agent, input: EditorViewInput) -> Result<EditorViewOutput, Errors> {
    let (width, height) = parse_size(input.size.as_deref().unwrap_or("1280x720"))?;
    let msaa = input.msaa.unwrap_or(4);
    let image = input.image.unwrap_or(true);
    let read_only = agent.read_only();
    with_world(agent, &input.target, |agent, sim, scene| {
        let out = input.out.clone().unwrap_or_else(|| {
            let label = Path::new(scene)
                .file_name()
                .map_or("scene".into(), |n| n.to_string_lossy().replace(".scene.json", ""));
            format!(".marionette/screenshots/{label}-editor.png")
        });
        if image
            && read_only
            && (Path::new(&out).is_absolute() || !out.replace('\\', "/").starts_with(".marionette/"))
        {
            return Err(marionette_core::Diagnostic::error(
                crate::codes::READ_ONLY,
                "in read-only mode images may only be written under .marionette/",
            )
            .fix("omit `out` to use the default location")
            .into());
        }
        if !matches!(msaa, 1 | 4) {
            return Err(marionette_core::Diagnostic::error(
                "cli.bad_msaa",
                format!("unsupported MSAA sample count {msaa}"),
            )
            .fix("use 1 or 4")
            .into());
        }
        let base = agent
            .resolve(scene)?
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        // The editor owns its world: view a copy (same entities and components) of this one.
        let tick = sim.tick();
        let text = sim.snapshot().to_text();
        let copy = agent.engine().load_text(&text, Some(scene), 0, &base)?;
        let eye = input.eye.map(|e| {
            let e = Vec3::from_array(e);
            (e, input.look_at.map_or(e + Vec3::NEG_Z, Vec3::from_array))
        });
        let req = ViewRequest {
            width,
            height,
            msaa,
            eye,
            frame: input.frame,
            select: input.select,
            panels: input.panels.unwrap_or(true),
            grid: input.grid.unwrap_or(true),
            picks: input.pick.iter().map(|p| (p[0], p[1])).collect(),
            image,
        };
        let gpu = agent.gpu()?;
        let result = view::render(gpu, scene, copy, base, &req).map_err(|e| e.with_file(scene))?;
        let names: std::collections::BTreeMap<u64, String> = sim
            .snapshot()
            .entities
            .into_iter()
            .filter_map(|e| e.name.map(|n| (e.id, n)))
            .collect();
        let path = match &result.pixels {
            Some(pixels) => {
                let png = encode_png(width, height, pixels);
                crate::write_atomic(&agent.resolve(&out)?, &out, &png)?;
                Some(out)
            }
            None => None,
        };
        let picks = req
            .picks
            .iter()
            .zip(result.picks)
            .map(|(&(x, y), hits)| Pick {
                x,
                y,
                hits: hits
                    .into_iter()
                    .take(HITS_KEEP)
                    .map(|h| PickedEntity {
                        id: h.entity,
                        name: names.get(&h.entity).cloned(),
                        distance: (h.distance as f64 * 1000.0).round() / 1000.0,
                    })
                    .collect(),
            })
            .collect();
        Ok(EditorViewOutput {
            path,
            width,
            height,
            tick,
            entities: result.entities,
            picks,
            selected: result.inspected.map(|e| Selected {
                id: e.id,
                name: e.name,
                components: e.components.into_iter().collect(),
            }),
            camera: [round3(result.camera.position), round3(result.camera.forward())],
        })
    })
}
