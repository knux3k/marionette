//! What the editor draws on top of the scene: panels, markers, the selection box, the
//! highlight and the ground grid. Everything is added to a copy of the frame (UI draws and
//! a few extra meshes), so the renderer needs no editor-specific code and the scene is never
//! changed.

use crate::inspect::{Inspected, lines};
use crate::pick::{Pickable, Shape, project, screen_rect};
use glam::{Mat4, Vec2, Vec3};
use marionette_core::spatial::{Color, Material, Vec2 as CVec2};
use marionette_core::ui::{Anchor, UiNode};
use marionette_render::extract::{MeshDraw, UiDraw, UiLook};
use marionette_render::{FrameData, View};

/// UI layer of editor draws: above any game UI.
pub const LAYER: i32 = 1_000_000;
const ORANGE: [f32; 4] = [1.0, 0.62, 0.15, 1.0];
const PANEL: [f32; 4] = [0.008, 0.009, 0.012, 0.97];
const TEXT: [f32; 4] = [0.9, 0.92, 0.95, 1.0];
const DIM: [f32; 4] = [0.6, 0.64, 0.7, 1.0];
/// Characters per row of the entity list and of the inspector.
pub const LIST_CHARS: usize = 20;
pub const INSPECTOR_CHARS: usize = 32;

/// Where the panels sit on a `w` x `h` window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    pub w: f32,
    pub h: f32,
    /// Text size in pixels per row (16 on windows at least 720 tall, else 8; multiples of 8
    /// keep the bitmap font crisp).
    pub text: f32,
    pub panels: bool,
}

impl Layout {
    pub fn new(w: f32, h: f32, panels: bool) -> Self {
        Self {
            w,
            h,
            text: if h >= 720.0 { 16.0 } else { 8.0 },
            panels,
        }
    }
    pub fn char_w(&self) -> f32 {
        self.text * 0.75
    }
    pub fn row(&self) -> f32 {
        self.text + 2.0
    }
    /// Height of the top and bottom bars.
    pub fn bar(&self) -> f32 {
        self.row() + 6.0
    }
    /// Entity list rectangle (x, y, w, h), when panels are shown.
    pub fn list(&self) -> Option<(f32, f32, f32, f32)> {
        self.panels.then(|| {
            let w = LIST_CHARS as f32 * self.char_w() + 16.0;
            (0.0, self.bar(), w, self.h - 2.0 * self.bar())
        })
    }
    /// Inspector rectangle (x, y, w, h), when panels are shown.
    pub fn inspector(&self) -> Option<(f32, f32, f32, f32)> {
        self.panels.then(|| {
            let w = INSPECTOR_CHARS as f32 * self.char_w() + 16.0;
            (self.w - w, self.bar(), w, self.h - 2.0 * self.bar())
        })
    }
    /// Rows of the entity list that fit.
    pub fn list_rows(&self) -> usize {
        self.list().map_or(0, |(_, _, _, h)| {
            ((h - 12.0 - self.row()) / self.row()).max(0.0) as usize
        })
    }
    /// The entity list row at screen `y`, if any (0 = the first shown row).
    pub fn list_row_at(&self, x: f32, y: f32) -> Option<usize> {
        let (lx, ly, lw, lh) = self.list()?;
        if x < lx || x >= lx + lw || y < ly || y >= ly + lh {
            return None;
        }
        let first = ly + 6.0 + self.row();
        (y >= first).then(|| ((y - first) / self.row()) as usize)
    }
    /// The point is over a panel or bar (not the 3D view).
    pub fn over_ui(&self, x: f32, y: f32) -> bool {
        let inside = |r: Option<(f32, f32, f32, f32)>| {
            r.is_some_and(|(rx, ry, rw, rh)| x >= rx && x < rx + rw && y >= ry && y < ry + rh)
        };
        y < self.bar() || y >= self.h - self.bar() || inside(self.list()) || inside(self.inspector())
    }
}

/// Collects UI draws in order; each later draw is on top of earlier ones.
pub struct Ui {
    pub draws: Vec<UiDraw>,
}

impl Ui {
    fn node(&self, x: f32, y: f32, w: f32, h: f32) -> UiNode {
        UiNode {
            anchor: Anchor::TopLeft,
            offset: CVec2 { x, y },
            size: CVec2 {
                x: w.max(0.0),
                y: h.max(0.0),
            },
            layer: LAYER,
        }
    }
    fn push(&mut self, node: UiNode, look: UiLook) {
        // Entity ids above any scene's keep editor draws last within the layer.
        let entity = u64::MAX - 1_000_000 + self.draws.len() as u64;
        self.draws.push(UiDraw { entity, node, look });
    }
    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) {
        let n = self.node(x, y, w, h);
        self.push(n, UiLook::Rect { color });
    }
    pub fn text(&mut self, x: f32, y: f32, size: f32, text: impl Into<String>, color: [f32; 4]) {
        let text = text.into();
        let n = self.node(x, y, text.chars().count() as f32 * size * 0.75, size);
        self.push(n, UiLook::Text { text, size, color });
    }
    /// A rectangle outline `t` pixels thick.
    pub fn frame(&mut self, a: Vec2, b: Vec2, t: f32, color: [f32; 4]) {
        let (w, h) = (b.x - a.x, b.y - a.y);
        self.rect(a.x - t, a.y - t, w + 2.0 * t, t, color);
        self.rect(a.x - t, b.y, w + 2.0 * t, t, color);
        self.rect(a.x - t, a.y, t, h, color);
        self.rect(b.x, a.y, t, h, color);
    }
}

/// Everything the overlay needs to know about the editor's state this frame.
pub struct OverlayInput<'a> {
    pub layout: Layout,
    pub view: &'a View,
    pub items: &'a [Pickable],
    pub selected: Option<u64>,
    pub entities: &'a [(u64, Option<String>)],
    /// First entity list row shown.
    pub list_top: usize,
    pub inspected: Option<&'a Inspected>,
    /// Top bar text and bottom bar text.
    pub status: &'a str,
    pub help: &'a str,
    pub grid: bool,
}

/// Draws the overlay into `frame`: highlight, grid, markers, selection box, panels, bars.
pub fn draw(frame: &mut FrameData, o: &OverlayInput) {
    let l = o.layout;
    if let Some(id) = o.selected {
        for m in frame.meshes.iter_mut().filter(|m| m.entity == id) {
            m.emissive += Vec3::new(0.45, 0.25, 0.03);
        }
        for s in frame.sprites.iter_mut().filter(|s| s.entity == id) {
            s.color = [
                (s.color[0] * 0.6 + 0.4).min(1.0),
                (s.color[1] * 0.6 + 0.25).min(1.0),
                s.color[2] * 0.6,
                s.color[3],
            ];
        }
    }
    if o.grid && o.view.proj.w_axis.w == 0.0 {
        // Centered where the view direction meets the ground (or below the eye).
        let forward = -o.view.view.row(2).truncate();
        let eye = o.view.eye;
        let center = if forward.y < -0.05 && -eye.y / forward.y < 500.0 {
            eye + forward * (-eye.y / forward.y)
        } else {
            eye
        };
        grid(frame, center, eye.y.abs());
    }
    let mut ui = Ui { draws: Vec::new() };
    // Markers for cameras, lights and empty entities.
    for item in o.items {
        if let Shape::Marker { position, letter } = item.shape
            && let Some((p, _)) = project(o.view, position, l.w, l.h)
        {
            let selected = o.selected == Some(item.entity);
            let s = l.text.max(12.0);
            ui.rect(
                p.x - s * 0.5,
                p.y - s * 0.5,
                s,
                s,
                if selected { ORANGE } else { [0.2, 0.55, 0.9, 0.9] },
            );
            let t = (s - 4.0).max(8.0).min(l.text);
            ui.text(p.x - t * 0.375, p.y - t * 0.5, t, letter.to_string(), [1.0; 4]);
        }
    }
    // Selection box and label.
    if let Some(item) = o.selected.and_then(|id| o.items.iter().find(|i| i.entity == id))
        && let Some((a, b)) = screen_rect(&item.shape, o.view, l.w, l.h)
    {
        let pad = Vec2::splat(if matches!(item.shape, Shape::Marker { .. }) {
            l.text.max(12.0) * 0.5 + 3.0
        } else {
            3.0
        });
        let (a, b) = (a - pad, b + pad);
        ui.frame(a, b, 2.0, ORANGE);
        let label = label(item.entity, o.entities);
        let lw = label.chars().count() as f32 * l.char_w() + 8.0;
        let y = if a.y - l.row() - 4.0 > l.bar() {
            a.y - l.row() - 4.0
        } else {
            b.y + 4.0
        };
        ui.rect(a.x - 2.0, y, lw, l.row() + 2.0, ORANGE);
        ui.text(a.x + 2.0, y + 2.0, l.text, label, [0.05, 0.05, 0.05, 1.0]);
    }
    // Bars.
    ui.rect(0.0, 0.0, l.w, l.bar(), PANEL);
    ui.text(
        8.0,
        4.0,
        l.text,
        clip(o.status, (l.w / l.char_w()) as usize - 1),
        TEXT,
    );
    ui.rect(0.0, l.h - l.bar(), l.w, l.bar(), PANEL);
    ui.text(
        8.0,
        l.h - l.bar() + 4.0,
        l.text,
        clip(o.help, (l.w / l.char_w()) as usize - 1),
        DIM,
    );
    // Entity list.
    if let Some((x, y, w, h)) = l.list() {
        ui.rect(x, y, w, h, PANEL);
        ui.text(
            x + 8.0,
            y + 6.0,
            l.text,
            format!("ENTITIES {}", o.entities.len()),
            DIM,
        );
        for (row, (id, name)) in o.entities.iter().skip(o.list_top).take(l.list_rows()).enumerate() {
            let ry = y + 6.0 + l.row() * (row + 1) as f32;
            let selected = o.selected == Some(*id);
            if selected {
                ui.rect(x + 2.0, ry - 1.0, w - 4.0, l.row(), [0.55, 0.33, 0.08, 0.95]);
            }
            let text = match name {
                Some(n) => format!("{id} {n}"),
                None => format!("{id}"),
            };
            ui.text(
                x + 8.0,
                ry,
                l.text,
                clip(&text, LIST_CHARS),
                if selected { [1.0; 4] } else { TEXT },
            );
        }
    }
    // Inspector.
    if let Some((x, y, w, h)) = l.inspector() {
        ui.rect(x, y, w, h, PANEL);
        let rows = ((h - 12.0) / l.row()).max(1.0) as usize;
        let text = match o.inspected {
            Some(e) => lines(e, INSPECTOR_CHARS, rows),
            None => vec![
                "NOTHING SELECTED".into(),
                String::new(),
                "CLICK AN OBJECT OR".into(),
                "A LIST ENTRY".into(),
            ],
        };
        for (i, line) in text.iter().enumerate() {
            let color = if i == 0 || !line.starts_with(' ') {
                TEXT
            } else {
                DIM
            };
            ui.text(x + 8.0, y + 6.0 + l.row() * i as f32, l.text, line.clone(), color);
        }
    }
    frame.ui.extend(ui.draws);
    frame.ui.sort_by_key(|u| (u.node.layer, u.entity));
}

fn label(id: u64, entities: &[(u64, Option<String>)]) -> String {
    match entities
        .iter()
        .find(|(e, _)| *e == id)
        .and_then(|(_, n)| n.as_deref())
    {
        Some(n) => format!("#{id} {n}"),
        None => format!("#{id}"),
    }
}

fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n.saturating_sub(2)).chain(['.', '.']).collect()
    }
}

/// Grid lines on the ground plane y = 0 around `center`, 21 each way: 1 m apart, or 10 m
/// apart when the camera is more than 30 m above the ground (`height`). The X axis is red
/// and the Z axis blue. They glow a little so lighting does not hide them.
pub fn grid(frame: &mut FrameData, center: Vec3, height: f32) {
    const HALF: i32 = 10;
    let step = if height > 30.0 { 10.0 } else { 1.0 };
    let thick = 0.015 * step * (1.0 + height / 20.0).min(4.0);
    let asset: std::sync::Arc<str> = "primitive:cube".into();
    let (cx, cz) = ((center.x / step).round() * step, (center.z / step).round() * step);
    let black = Material {
        base_color: Color {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        },
        metallic: 0.0,
        roughness: 1.0,
    };
    let mut line = |center: Vec3, scale: Vec3, glow: Vec3| {
        frame.meshes.push(MeshDraw {
            entity: u64::MAX,
            asset: asset.clone(),
            world: Mat4::from_scale_rotation_translation(scale, glam::Quat::IDENTITY, center),
            material: Some(black),
            pose: None,
            emissive: glow,
            lamp: false,
        });
    };
    let len = 2.0 * HALF as f32 * step;
    for i in -HALF..=HALF {
        let (x, z) = (cx + i as f32 * step, cz + i as f32 * step);
        let glow = |axis: bool, c: Vec3| if axis { c } else { Vec3::splat(0.16) };
        line(
            Vec3::new(x, 0.002, cz),
            Vec3::new(thick, thick, len),
            glow(x == 0.0, Vec3::new(0.1, 0.25, 0.9)),
        );
        line(
            Vec3::new(cx, 0.002, z),
            Vec3::new(len, thick, thick),
            glow(z == 0.0, Vec3::new(0.9, 0.15, 0.1)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::FlyCamera;

    #[test]
    fn layout_rows_and_hit_regions() {
        let l = Layout::new(1280.0, 720.0, true);
        assert_eq!(l.text, 16.0);
        let (lx, ly, lw, _) = l.list().unwrap();
        assert_eq!((lx, ly), (0.0, l.bar()));
        assert!(l.list_rows() > 20);
        let first = ly + 6.0 + l.row() + 1.0;
        assert_eq!(l.list_row_at(10.0, first), Some(0));
        assert_eq!(l.list_row_at(10.0, first + l.row()), Some(1));
        assert_eq!(l.list_row_at(lw + 1.0, first), None);
        assert!(l.over_ui(1279.0, 300.0), "inspector");
        assert!(!l.over_ui(640.0, 360.0), "the 3D view");
        assert!(!Layout::new(1280.0, 720.0, false).over_ui(10.0, 300.0));
        assert_eq!(Layout::new(640.0, 360.0, true).text, 8.0);
    }

    #[test]
    fn overlay_adds_ui_on_top_and_never_changes_scene_draws_but_the_highlight() {
        let mut frame = FrameData::default();
        frame.meshes.push(MeshDraw {
            entity: 5,
            asset: "primitive:cube".into(),
            world: Mat4::IDENTITY,
            material: None,
            pose: None,
            emissive: Vec3::ZERO,
            lamp: false,
        });
        let view = FlyCamera::default().view(16.0 / 9.0);
        let items = [Pickable {
            entity: 5,
            shape: Shape::Box {
                world: Mat4::IDENTITY,
                min: Vec3::splat(-0.5),
                max: Vec3::splat(0.5),
            },
            layer: 0,
        }];
        let entities = vec![(5, Some("crate".to_string()))];
        let o = OverlayInput {
            layout: Layout::new(1280.0, 720.0, true),
            view: &view,
            items: &items,
            selected: Some(5),
            entities: &entities,
            list_top: 0,
            inspected: None,
            status: "MARIONETTE EDITOR",
            help: "HELP",
            grid: true,
        };
        draw(&mut frame, &o);
        assert!(frame.meshes[0].emissive.x > 0.0, "selected model glows");
        assert_eq!(frame.meshes.len(), 1 + 42, "21 grid lines each way");
        assert!(frame.ui.iter().all(|u| u.node.layer == LAYER));
        let texts: Vec<&str> = frame
            .ui
            .iter()
            .filter_map(|u| match &u.look {
                UiLook::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        for want in [
            "#5 crate",
            "5 crate",
            "ENTITIES 1",
            "MARIONETTE EDITOR",
            "NOTHING SELECTED",
        ] {
            assert!(texts.contains(&want), "{want} missing from {texts:?}");
        }
    }
}
