//! The module: scene types, the tick system (anchors, facing, pointer interaction) and the
//! draw hook that turns panels into world-space quads.

use crate::components::*;
use crate::font::FontData;
use crate::layout::{self, Rect};
use crate::runtime::*;
use bevy_ecs::prelude::*;
use glam::{Mat3, Quat, Vec3};
use marionette_core::actions::ActionState;
use marionette_core::builtin::{FixedStep, Position, DEFAULT_DT};
use marionette_core::module::{Module, SceneDir, Stage};
use marionette_core::scene::SceneId;
use marionette_core::spatial::{
    Camera, QuadContext, QuadDrawHooks, QuadVertex, Transform, WorldQuads,
};
use marionette_core::{Diagnostic, Errors, Registry, Result};
use std::collections::{BTreeMap, BTreeSet};

pub mod codes {
    pub const FONT: &str = "ui3d.font";
}

/// Offset in meters between coplanar layers of a panel, toward the viewer.
const LAYER_STEP: f32 = 0.0004;
/// A touch hovers up to this far in front of a panel and pushes through to press.
const TOUCH_HOVER: f32 = 0.08;
const TOUCH_BEHIND: f32 = 0.05;
const TOUCH_PRESS: f32 = 0.01;

pub struct Ui3dModule;

impl Module for Ui3dModule {
    fn name(&self) -> &'static str {
        "ui3d"
    }

    fn register(&self, r: &mut Registry) {
        r.register_component::<UiPanel>();
        r.register_component::<UiContent>();
        r.register_component::<UiAnchor>();
        r.register_resource::<UiFontConfig>();
    }

    fn add_systems(&self, schedule: &mut Schedule) {
        schedule.add_systems(ui_tick.in_set(Stage::Logic));
    }

    fn init(&self, world: &mut World) -> Result<()> {
        let mut errors = Vec::new();
        let config = world.get_resource::<UiFontConfig>().cloned().unwrap_or_default();
        let font = if config.font.is_empty() {
            FontData::builtin()
        } else {
            let dir = world.get_resource::<SceneDir>().map(|d| d.0.clone()).unwrap_or_default();
            let loaded = std::fs::read(dir.join(&config.font))
                .map_err(|e| format!("cannot read {}: {e}", config.font))
                .and_then(|bytes| FontData::from_ttf(&bytes));
            match loaded {
                Ok(f) => f,
                Err(msg) => {
                    errors.push(
                        Diagnostic::error(codes::FONT, msg)
                            .component("UiFontConfig")
                            .field("font")
                            .fix("give a TrueType or OpenType file relative to the scene, or \"\" for the built-in font"),
                    );
                    FontData::builtin()
                }
            }
        };
        world.insert_resource(UiRuntime::new(font));
        if world.get_resource::<UiPointers>().is_none() {
            world.insert_resource(UiPointers::default());
        }
        world.insert_resource(UiEvents::default());
        QuadDrawHooks::add(world, quad_hook);
        if errors.is_empty() { Ok(()) } else { Err(Errors(errors)) }
    }
}

/// A panel's world pose: centre and rotation (its +Z faces the front).
fn pose(t: Option<&Transform>, p: Option<&Position>) -> (Vec3, Quat) {
    let t = t.copied().unwrap_or_default();
    let pos = p.map_or(Vec3::new(t.translation.x, t.translation.y, t.translation.z), |p| Vec3::new(p.x, p.y, p.z));
    (pos, Quat::from_xyzw(t.rotation.x, t.rotation.y, t.rotation.z, t.rotation.w).normalize())
}

fn to_quat(q: Quat) -> marionette_core::spatial::Quat {
    marionette_core::spatial::Quat { x: q.x, y: q.y, z: q.z, w: q.w }
}

/// Rotation whose +Z points along `toward`, keeping +Y up as far as possible. Only
/// additions, multiplications and square roots, so results match on every platform.
fn facing(toward: Vec3, yaw_only: bool) -> Option<Quat> {
    let mut z = toward;
    if yaw_only {
        z.y = 0.0;
    }
    if z.length_squared() < 1e-10 {
        return None;
    }
    let z = z.normalize();
    let mut x = Vec3::Y.cross(z);
    if x.length_squared() < 1e-10 {
        x = Vec3::X;
    }
    let x = x.normalize();
    let y = z.cross(x);
    Some(Quat::from_mat3(&Mat3::from_cols(x, y, z)))
}

fn viewer(world: &mut World) -> Option<Vec3> {
    let mut q = world.query::<(&SceneId, &Camera, Option<&Transform>, Option<&Position>)>();
    q.iter(world)
        .min_by_key(|(id, ..)| id.0)
        .map(|(_, _, t, p)| pose(t, p).0)
}

fn set_translation(world: &mut World, e: Entity, v: Vec3) {
    if let Some(mut p) = world.get_mut::<Position>(e) {
        p.x = v.x;
        p.y = v.y;
        p.z = v.z;
    } else if let Some(mut t) = world.get_mut::<Transform>(e) {
        t.translation = marionette_core::spatial::Vec3::new(v.x, v.y, v.z);
    } else {
        world.entity_mut(e).insert(Transform {
            translation: marionette_core::spatial::Vec3::new(v.x, v.y, v.z),
            ..Default::default()
        });
    }
}

fn anchors_and_facing(world: &mut World, dt: f32) {
    let mut anchored: Vec<(Entity, UiAnchor)> = Vec::new();
    let mut q = world.query::<(Entity, &UiAnchor, &UiPanel)>();
    anchored.extend(q.iter(world).map(|(e, a, _)| (e, a.clone())));
    if !anchored.is_empty() {
        let needed: BTreeSet<u64> = anchored.iter().map(|(_, a)| a.target.0).collect();
        let mut targets: BTreeMap<u64, (Vec3, Quat)> = BTreeMap::new();
        let mut q = world.query::<(&SceneId, Option<&Transform>, Option<&Position>)>();
        for (id, t, p) in q.iter(world) {
            if needed.contains(&id.0) {
                targets.insert(id.0, pose(t, p));
            }
        }
        for (e, a) in anchored {
            let Some((tp, tr)) = targets.get(&a.target.0).copied() else { continue };
            let off = Vec3::new(a.offset.x, a.offset.y, a.offset.z);
            let want = tp + if a.relative { tr * off } else { off };
            let (now, _) = pose(world.get::<Transform>(e), world.get::<Position>(e));
            let k = if a.smoothing > 0.0 { dt / (a.smoothing + dt) } else { 1.0 };
            set_translation(world, e, now + (want - now) * k);
        }
    }
    let mut turning: Vec<(Entity, UiFace)> = Vec::new();
    let mut q = world.query::<(Entity, &UiPanel)>();
    turning.extend(q.iter(world).filter(|(_, p)| p.face != UiFace::Fixed).map(|(e, p)| (e, p.face)));
    if turning.is_empty() {
        return;
    }
    let Some(eye) = viewer(world) else { return };
    for (e, face) in turning {
        let (pos, _) = pose(world.get::<Transform>(e), world.get::<Position>(e));
        if let Some(rot) = facing(eye - pos, face == UiFace::Yaw) {
            if let Some(mut t) = world.get_mut::<Transform>(e) {
                t.rotation = to_quat(rot);
            } else {
                world.entity_mut(e).insert(Transform { rotation: to_quat(rot), ..Default::default() });
            }
        }
    }
}

struct PanelInfo {
    entity: Entity,
    id: u64,
    centre: Vec3,
    rot: Quat,
    panel: UiPanel,
}

struct Hit {
    /// Distance used to pick the nearest panel.
    dist: f32,
    /// Panel coordinates with the origin at the top-left corner, y down.
    x: f32,
    y: f32,
    inside: bool,
}

fn hit(info: &PanelInfo, p: &UiPointer) -> Option<Hit> {
    let n = info.rot * Vec3::Z;
    let o = Vec3::from(p.origin);
    let (point, dist) = match p.kind {
        PointerKind::Ray => {
            let d = Vec3::from(p.direction);
            let denom = d.dot(n);
            if denom >= -1e-6 {
                return None;
            }
            let t = (info.centre - o).dot(n) / denom;
            if t < 0.0 {
                return None;
            }
            (o + d * t, t * d.length())
        }
        PointerKind::Touch => {
            let d = (o - info.centre).dot(n);
            if !(-TOUCH_BEHIND..=TOUCH_HOVER).contains(&d) {
                return None;
            }
            (o - n * d, d.abs())
        }
    };
    let local = info.rot.inverse() * (point - info.centre);
    let (w, h) = (info.panel.size.x, info.panel.size.y);
    let (x, y) = (local.x + w / 2.0, h / 2.0 - local.y);
    Some(Hit { dist, x, y, inside: x >= 0.0 && x <= w && y >= 0.0 && y <= h })
}

fn pressed(p: &UiPointer, h: Option<&Hit>, info: Option<&PanelInfo>) -> bool {
    match p.kind {
        PointerKind::Ray => p.pressed,
        PointerKind::Touch => info.is_some_and(|i| {
            let n = i.rot * Vec3::Z;
            (Vec3::from(p.origin) - i.centre).dot(n) <= TOUCH_PRESS && h.is_some()
        }),
    }
}

/// Item under (x, y), if any (the last drawn wins; spacers and hidden items do not count).
fn item_at(info: &PanelInfo, content: &UiContent, lay: &layout::Layout, x: f32, y: f32) -> Option<usize> {
    let bottom = info.panel.size.y - info.panel.padding;
    (0..content.items.len()).rev().find(|&i| {
        let r: Rect = lay.rects[i];
        !matches!(content.items[i].widget, UiWidget::Spacer) && r.y + r.h <= bottom + 1e-4 && r.contains(x, y)
    })
}

fn ui_tick(world: &mut World) {
    let dt = world.get_resource::<FixedStep>().map_or(DEFAULT_DT, |f| f.dt);
    if world.get_resource::<UiRuntime>().is_none() {
        return;
    }
    anchors_and_facing(world, dt);
    let mut events: Vec<UiEvent> = Vec::new();

    let mut infos: Vec<PanelInfo> = Vec::new();
    {
        let mut q = world.query::<(Entity, &SceneId, &UiPanel, Option<&Transform>, Option<&Position>)>();
        for (e, id, panel, t, p) in q.iter(world) {
            let (centre, rot) = pose(t, p);
            infos.push(PanelInfo { entity: e, id: id.0, centre, rot, panel: panel.clone() });
        }
    }
    infos.sort_by_key(|i| i.id);
    let live: BTreeSet<u64> = infos.iter().map(|i| i.id).collect();
    let mut pointers = world.get_resource::<UiPointers>().cloned().unwrap_or_default().0;
    pointers.sort_by_key(|p| p.id);

    let mut rt = world.remove_resource::<UiRuntime>().expect("checked above");
    rt.retain(&live);
    let mut states: BTreeMap<u64, layout::PanelState> = BTreeMap::new();
    let mut clicks: Vec<String> = Vec::new();

    for p in &pointers {
        let was_down = rt.down.get(&p.id).copied().unwrap_or(false);
        let grab = rt.grabs.get(&p.id).map(|g| (g.panel, g.item));
        // The grabbed panel keeps the pointer even when it slides off it (slider drags).
        let target = match grab {
            Some((panel, _)) => infos.iter().find(|i| i.id == panel).and_then(|i| hit(i, p).map(|h| (i, h))),
            None => infos
                .iter()
                .filter(|i| i.panel.interactive)
                .filter_map(|i| hit(i, p).map(|h| (i, h)))
                .filter(|(_, h)| h.inside)
                .min_by(|a, b| a.1.dist.total_cmp(&b.1.dist)),
        };
        let down = pressed(p, target.as_ref().map(|t| &t.1), target.as_ref().map(|t| t.0));
        let Some((info, h)) = target else {
            rt.grabs.remove(&p.id);
            rt.down.insert(p.id, down);
            continue;
        };
        let Some(content) = world.get::<UiContent>(info.entity).cloned() else {
            rt.grabs.remove(&p.id);
            rt.down.insert(p.id, down);
            continue;
        };
        let lay = rt.layout(info.id, &info.panel, &content);
        let under = if h.inside { item_at(info, &content, &lay, h.x, h.y) } else { None };
        let s0 = info.panel.text_size;
        let st = states.entry(info.id).or_default();
        st.hover = under;
        if let Some(i) = under
            && let UiWidget::List { visible, scroll, .. } = &content.items[i].widget
        {
            let row = ((h.y - lay.rects[i].y) / layout::row_height(s0)).max(0.0) as usize;
            st.hover_row = Some(scroll.max(0.0) as usize + row.min((*visible).max(1) as usize - 1));
        }
        let mut changed: Option<UiContent> = None;
        let set = |changed: &mut Option<UiContent>| -> usize {
            if changed.is_none() {
                *changed = Some(content.clone());
            }
            0
        };
        // Press: start a grab; sliders react at once.
        if down && !was_down && under.is_some() && grab.is_none() {
            rt.grabs.insert(p.id, Grab { panel: info.id, item: under.expect("checked") });
        }
        let grab = rt.grabs.get(&p.id).map(|g| g.item);
        if let Some(i) = grab {
            st.pressed = Some(i);
            let item = &content.items[i];
            if down && let UiWidget::Slider { min, max, value, .. } = &item.widget {
                let (l, r, _) = layout::slider_track(lay.rects[i], s0);
                let f = if r > l { ((h.x - l) / (r - l)).clamp(0.0, 1.0) } else { 0.0 };
                let v = min + f * (max - min);
                if v != *value {
                    set(&mut changed);
                    if let Some(UiWidget::Slider { value, .. }) =
                        changed.as_mut().map(|c| &mut c.items[i].widget)
                    {
                        *value = v;
                    }
                    events.push(UiEvent { panel: info.id, item: item.id.clone(), kind: UiEventKind::Slide, value: v, pointer: p.id });
                }
            }
            if !down && was_down {
                if under == Some(i) {
                    match &item.widget {
                        UiWidget::Button { action, .. } => {
                            events.push(UiEvent { panel: info.id, item: item.id.clone(), kind: UiEventKind::Click, value: 1.0, pointer: p.id });
                            if !action.is_empty() {
                                clicks.push(action.clone());
                            }
                        }
                        UiWidget::Toggle { on, .. } => {
                            set(&mut changed);
                            if let Some(UiWidget::Toggle { on: o, .. }) = changed.as_mut().map(|c| &mut c.items[i].widget) {
                                *o = !*on;
                            }
                            events.push(UiEvent { panel: info.id, item: item.id.clone(), kind: UiEventKind::Toggle, value: if *on { 0.0 } else { 1.0 }, pointer: p.id });
                        }
                        UiWidget::List { rows, scroll, .. } => {
                            if let Some(row) = st.hover_row.filter(|r| *r < rows.len()) {
                                let _ = scroll;
                                set(&mut changed);
                                if let Some(UiWidget::List { selected, .. }) = changed.as_mut().map(|c| &mut c.items[i].widget) {
                                    *selected = row as i32;
                                }
                                events.push(UiEvent { panel: info.id, item: item.id.clone(), kind: UiEventKind::Select, value: row as f32, pointer: p.id });
                            }
                        }
                        _ => {}
                    }
                }
                rt.grabs.remove(&p.id);
            }
        } else if p.scroll != 0.0
            && let Some(i) = under
            && let UiWidget::List { rows, visible, scroll, .. } = &content.items[i].widget
        {
            let max = rows.len().saturating_sub((*visible).max(1) as usize) as f32;
            let v = (scroll + p.scroll).clamp(0.0, max);
            if v != *scroll {
                set(&mut changed);
                if let Some(UiWidget::List { scroll: s, .. }) = changed.as_mut().map(|c| &mut c.items[i].widget) {
                    *s = v;
                }
                events.push(UiEvent { panel: info.id, item: content.items[i].id.clone(), kind: UiEventKind::Scroll, value: v, pointer: p.id });
            }
        }
        rt.down.insert(p.id, down);
        if let Some(c) = changed
            && let Some(mut slot) = world.get_mut::<UiContent>(info.entity)
        {
            *slot = c;
        }
    }
    rt.down.retain(|id, _| pointers.iter().any(|p| p.id == *id));
    rt.states = states;
    events.truncate(MAX_EVENTS);
    world.insert_resource(rt);
    if !clicks.is_empty()
        && let Some(mut actions) = world.get_resource_mut::<ActionState>()
    {
        for a in clicks {
            actions.values.insert(a, 1.0);
        }
    }
    world.insert_resource(UiEvents(events));
}

/// Turns every visible panel into quads, far panels first so blending stacks correctly.
fn quad_hook(world: &World, ctx: &QuadContext, out: &mut Vec<WorldQuads>) {
    let Some(rt) = world.get_resource::<UiRuntime>() else { return };
    let Some(mut q) = world.try_query::<(&SceneId, &UiPanel, &UiContent, Option<&Transform>, Option<&Position>)>() else {
        return;
    };
    let eye = Vec3::from(ctx.eye);
    let mut visible: Vec<(f32, u64, &UiPanel, &UiContent, Vec3, Quat)> = Vec::new();
    for (id, panel, content, t, p) in q.iter(world) {
        let (centre, rot) = pose(t, p);
        let dist = eye.distance(centre);
        if panel.max_distance > 0.0 && dist > panel.max_distance {
            continue;
        }
        if (eye - centre).dot(rot * Vec3::Z) <= 0.0 {
            continue; // seen from behind
        }
        visible.push((dist, id.0, panel, content, centre, rot));
    }
    if visible.is_empty() {
        return;
    }
    visible.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut quads = WorldQuads { image: rt.font.image_name.clone(), vertices: Vec::new() };
    quads.vertices.reserve(visible.len() * 64);
    for (_, id, panel, content, centre, rot) in visible {
        let mesh = rt.quads(id, panel, content);
        let (hw, hh) = (panel.size.x / 2.0, panel.size.y / 2.0);
        for quad in mesh.iter() {
            let r = quad.rect;
            let (x0, x1) = (r.x - hw, r.x + r.w - hw);
            let (yt, yb) = (hh - r.y, hh - r.y - r.h);
            let z = quad.layer as f32 * LAYER_STEP;
            let [u0, v0, u1, v1] = quad.uv;
            for (x, y, u, v) in [(x0, yb, u0, v0), (x1, yb, u1, v0), (x1, yt, u1, v1), (x0, yt, u0, v1)] {
                let w = centre + rot * Vec3::new(x, y, z);
                quads.vertices.push(QuadVertex { position: w.to_array(), uv: [u, v], color: quad.color });
            }
        }
    }
    out.push(quads);
}
