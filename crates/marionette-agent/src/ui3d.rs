//! World-space UI operations (cargo feature `ui3d`): point at panels from a script or an agent,
//! and read where panels and their items are, without a screenshot.

use crate::observe::{WorldTarget, with_world};
use crate::{Agent, Op, op};
use marionette_core::scene::SceneId;
use marionette_core::Errors;
use marionette_ui3d::{PointerKind, UiEvents, UiPointer, UiPointers, UiRuntime};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub(crate) fn ops() -> Vec<Op> {
    vec![
        op(
            "ui3d_point",
            "Set the world-space UI pointers of a live session, run `ticks` ticks (default 1), and \
             return the UI events of those ticks. Each pointer is a ray (`origin`, `direction`, \
             `pressed`, `scroll`) or a touch (`kind: touch`, `origin` = fingertip). An empty \
             `pointers` list lifts all pointers. Press, then release in a later call, to click.",
            true,
            point,
        ),
        op(
            "ui3d_panels",
            "List world-space panels: scene id, items with their id, kind, rectangle in panel \
             meters (top-left origin, y down) and current value, and whether the content overflows. \
             `session` or `scene` as for other observe operations.",
            false,
            panels,
        ),
    ]
}

#[derive(Deserialize, JsonSchema)]
pub struct PointInput {
    pub session: String,
    pub pointers: Vec<PointerIn>,
    #[serde(default = "one")]
    pub ticks: u64,
}

fn one() -> u64 {
    1
}

#[derive(Deserialize, JsonSchema)]
pub struct PointerIn {
    #[serde(default)]
    pub id: u32,
    /// `ray` (default) or `touch`.
    pub kind: Option<String>,
    pub origin: [f32; 3],
    #[serde(default)]
    pub direction: [f32; 3],
    #[serde(default)]
    pub pressed: bool,
    #[serde(default)]
    pub scroll: f32,
}

#[derive(Serialize, JsonSchema)]
pub struct EventOut {
    pub tick: u64,
    pub panel: u64,
    pub item: String,
    /// `click`, `toggle`, `slide`, `select` or `scroll`.
    pub kind: String,
    pub value: f32,
    pub pointer: u32,
}

#[derive(Serialize, JsonSchema)]
pub struct PointOutput {
    pub tick: u64,
    pub events: Vec<EventOut>,
}

fn point(agent: &mut Agent, input: PointInput) -> Result<PointOutput, Errors> {
    let target = WorldTarget { session: Some(input.session), ..Default::default() };
    with_world(agent, &target, |_, sim, _| {
        let pointers = input
            .pointers
            .iter()
            .map(|p| UiPointer {
                id: p.id,
                kind: if p.kind.as_deref() == Some("touch") { PointerKind::Touch } else { PointerKind::Ray },
                origin: p.origin,
                direction: p.direction,
                pressed: p.pressed,
                scroll: p.scroll,
            })
            .collect();
        sim.world.insert_resource(UiPointers(pointers));
        let mut events = Vec::new();
        for _ in 0..input.ticks.clamp(1, 600) {
            sim.step();
            let tick = sim.tick();
            if let Some(e) = sim.world.get_resource::<UiEvents>() {
                events.extend(e.0.iter().map(|e| EventOut {
                    tick,
                    panel: e.panel,
                    item: e.item.clone(),
                    kind: format!("{:?}", e.kind).to_lowercase(),
                    value: e.value,
                    pointer: e.pointer,
                }));
            }
        }
        // A scroll amount applies to one tick only.
        for p in &mut sim.world.resource_mut::<UiPointers>().0 {
            p.scroll = 0.0;
        }
        Ok(PointOutput { tick: sim.tick(), events })
    })
}

#[derive(Deserialize, JsonSchema)]
pub struct PanelsInput {
    #[serde(flatten)]
    pub target: WorldTarget,
}

#[derive(Serialize, JsonSchema)]
pub struct ItemOut {
    pub id: String,
    pub kind: String,
    /// `[x, y, width, height]` in meters from the panel's top-left corner.
    pub rect: [f32; 4],
    /// Slider value, toggle 0/1, progress, list selected row; absent for other kinds.
    pub value: Option<f32>,
}

#[derive(Serialize, JsonSchema)]
pub struct PanelOut {
    pub id: u64,
    pub size: [f32; 2],
    pub overflow: bool,
    pub items: Vec<ItemOut>,
}

#[derive(Serialize, JsonSchema)]
pub struct PanelsOutput {
    pub panels: Vec<PanelOut>,
}

fn panels(agent: &mut Agent, input: PanelsInput) -> Result<PanelsOutput, Errors> {
    with_world(agent, &input.target, |_, sim, _| {
        use marionette_ui3d::{UiContent, UiPanel, UiWidget};
        let mut found: Vec<(u64, UiPanel, UiContent)> = sim
            .world
            .query::<(&SceneId, &UiPanel, &UiContent)>()
            .iter(&sim.world)
            .map(|(i, p, c)| (i.0, p.clone(), c.clone()))
            .collect();
        found.sort_by_key(|f| f.0);
        let Some(rt) = sim.world.get_resource::<UiRuntime>() else {
            return Ok(PanelsOutput { panels: Vec::new() });
        };
        let panels = found
            .into_iter()
            .map(|(id, panel, content)| {
                let lay = rt.layout(id, &panel, &content);
                let items = content
                    .items
                    .iter()
                    .zip(lay.rects.iter())
                    .map(|(item, r)| {
                        let (kind, value) = match &item.widget {
                            UiWidget::Label { .. } => ("label", None),
                            UiWidget::Button { .. } => ("button", None),
                            UiWidget::Toggle { on, .. } => ("toggle", Some(f32::from(u8::from(*on)))),
                            UiWidget::Slider { value, .. } => ("slider", Some(*value)),
                            UiWidget::Progress { value, .. } => ("progress", Some(*value)),
                            UiWidget::List { selected, .. } => ("list", Some(*selected as f32)),
                            UiWidget::Spacer => ("spacer", None),
                        };
                        ItemOut { id: item.id.clone(), kind: kind.into(), rect: [r.x, r.y, r.w, r.h], value }
                    })
                    .collect();
                PanelOut { id, size: [panel.size.x, panel.size.y], overflow: lay.overflow, items }
            })
            .collect();
        Ok(PanelsOutput { panels })
    })
}
