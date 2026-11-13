//! The editor view without a window: what the window would show (same camera, overlay and
//! picking), rendered offscreen. Used by the `editor_view` operation, so an agent can see
//! the editor's picture, ask what is under a pixel and read the inspector, and by tests.

use crate::camera::FlyCamera;
use crate::inspect::{self, Inspected};
use crate::overlay::Layout;
use crate::pick::{self, Hit};
use crate::state::Editor;
use glam::Vec3;
use marionette_core::{Errors, Sim};
use marionette_render::{Gpu, Offscreen, extract};
use std::path::PathBuf;

/// What to show.
#[derive(Debug, Clone, Default)]
pub struct ViewRequest {
    pub width: u32,
    pub height: u32,
    pub msaa: u32,
    /// Editor camera place and target; default: the scene's first camera (or the whole
    /// scene framed when it has none).
    pub eye: Option<(Vec3, Vec3)>,
    /// Frame the selection (or everything) instead of using the camera as given.
    pub frame: bool,
    pub select: Option<u64>,
    pub panels: bool,
    pub grid: bool,
    /// Pixels to pick at (x, y; y down).
    pub picks: Vec<(f32, f32)>,
    /// Render an image (otherwise only picks and the inspector).
    pub image: bool,
}

pub struct ViewResult {
    /// RGBA8 sRGB pixels when an image was asked for.
    pub pixels: Option<Vec<u8>>,
    /// Per pick: everything under the pixel, nearest first.
    pub picks: Vec<Vec<Hit>>,
    pub inspected: Option<Inspected>,
    pub camera: FlyCamera,
    pub entities: usize,
}

/// Renders the editor view of `sim`. `base_dir` resolves the scene's relative asset paths.
pub fn render(
    gpu: &Gpu,
    scene: &str,
    sim: Sim,
    base_dir: PathBuf,
    req: &ViewRequest,
) -> Result<ViewResult, Errors> {
    let (w, h) = (req.width.max(1), req.height.max(1));
    let mut off = Offscreen::new(gpu, w, h, req.msaa.max(1), base_dir);
    let mut editor = Editor::new(scene, sim);
    editor.panels = req.panels;
    editor.grid = req.grid;
    let frame = extract(&editor.sim.world);
    // Load models first so picking and framing see their real bounds.
    let problems = off.renderer.prepare(&frame);
    if !problems.is_empty() {
        return Err(Errors(problems));
    }
    let items = editor.items(&frame, &|a| off.renderer.model_bounds(a));
    if let Some((eye, target)) = req.eye {
        let f = (target - eye).normalize_or(Vec3::NEG_Z);
        editor.camera.position = eye;
        editor.camera.yaw = f.x.atan2(-f.z);
        editor.camera.pitch = f.y.clamp(-1.0, 1.0).asin().clamp(-1.553, 1.553);
    }
    let layout = Layout::new(w as f32, h as f32, req.panels);
    editor.select(req.select, layout.list_rows());
    if req.frame {
        editor.frame_selection(&items);
    }
    let view = editor.camera.view(w as f32 / h as f32);
    let picks = req
        .picks
        .iter()
        .map(|&(x, y)| pick::hits(&items, &view, x, y, w as f32, h as f32))
        .collect();
    let pixels = if req.image {
        let mut shown = frame.clone();
        editor.overlay(&mut shown, &items, &view, layout);
        off.render_view(gpu, &shown, view)?;
        Some(off.read_pixels(gpu)?)
    } else {
        None
    };
    Ok(ViewResult {
        pixels,
        picks,
        inspected: req
            .select
            .and_then(|id| inspect::inspect(&editor.sim.world, editor.sim.registry(), id)),
        camera: editor.camera,
        entities: editor.entities().len(),
    })
}
