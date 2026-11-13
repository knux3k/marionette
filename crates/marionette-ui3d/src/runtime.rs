//! Runtime (not scene) state of world-space UI: the font, a cache of layouts and meshes per
//! panel, what each pointer is doing, and the events produced. None of it is saved or hashed;
//! the scene's components hold everything that matters.

use crate::components::{UiContent, UiPanel};
use crate::font::FontData;
use crate::layout::{self, Layout, PanelState, Quad};
use bevy_ecs::prelude::*;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerKind {
    /// A ray (a controller laser, a gaze or mouse ray): `pressed` is the trigger.
    Ray,
    /// A fingertip or hand-held point at `origin`: it presses by pushing into the panel.
    Touch,
}

/// One pointing source. Whoever owns the input (an XR module, the mouse, a script, a test)
/// writes these into [`UiPointers`] each tick; panels never depend on where they come from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiPointer {
    /// Stable number per source (left hand 0, right hand 1, ...).
    pub id: u32,
    pub kind: PointerKind,
    /// World position of the ray start or the fingertip.
    pub origin: [f32; 3],
    /// World direction of the ray (need not be normalized; unused by touch).
    pub direction: [f32; 3],
    /// Trigger or click held (rays only).
    pub pressed: bool,
    /// Scroll amount this tick in list rows (positive moves down).
    pub scroll: f32,
}

/// The current pointers. Level state: whatever is here when a tick runs is used.
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct UiPointers(pub Vec<UiPointer>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiEventKind {
    Click,
    Toggle,
    Slide,
    Select,
    Scroll,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UiEvent {
    /// Scene id of the panel entity.
    pub panel: u64,
    /// The item's `id`.
    pub item: String,
    pub kind: UiEventKind,
    /// New value (slider value, 1/0 toggle, selected row, scroll row).
    pub value: f32,
    pub pointer: u32,
}

/// Events of the last tick (at most [`MAX_EVENTS`]), replaced every tick.
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct UiEvents(pub Vec<UiEvent>);

pub const MAX_EVENTS: usize = 256;

pub(crate) struct Grab {
    pub panel: u64,
    pub item: usize,
}

struct Cached {
    panel: UiPanel,
    content: UiContent,
    layout: Arc<Layout>,
    mesh: Option<(PanelState, Arc<Vec<Quad>>)>,
}

#[derive(Resource)]
pub struct UiRuntime {
    pub font: Arc<FontData>,
    pub(crate) states: BTreeMap<u64, PanelState>,
    pub(crate) grabs: BTreeMap<u32, Grab>,
    pub(crate) down: BTreeMap<u32, bool>,
    cache: Mutex<BTreeMap<u64, Cached>>,
}

impl UiRuntime {
    pub fn new(font: FontData) -> Self {
        Self {
            font: Arc::new(font),
            states: BTreeMap::new(),
            grabs: BTreeMap::new(),
            down: BTreeMap::new(),
            cache: Mutex::new(BTreeMap::new()),
        }
    }

    fn entry<R>(
        &self,
        id: u64,
        panel: &UiPanel,
        content: &UiContent,
        f: impl FnOnce(&mut Cached, &FontData) -> R,
    ) -> R {
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        let fresh = cache
            .get(&id)
            .is_none_or(|c| c.panel != *panel || c.content != *content);
        if fresh {
            cache.insert(
                id,
                Cached {
                    panel: panel.clone(),
                    content: content.clone(),
                    layout: Arc::new(layout::compute(panel, content, &self.font)),
                    mesh: None,
                },
            );
        }
        f(cache.get_mut(&id).expect("just inserted"), &self.font)
    }

    /// Layout of a panel, rebuilt only when its components changed.
    pub fn layout(&self, id: u64, panel: &UiPanel, content: &UiContent) -> Arc<Layout> {
        self.entry(id, panel, content, |c, _| c.layout.clone())
    }

    /// Mesh of a panel, rebuilt only when its components or pointer state changed.
    pub fn quads(&self, id: u64, panel: &UiPanel, content: &UiContent) -> Arc<Vec<Quad>> {
        let state = self.states.get(&id).copied().unwrap_or_default();
        self.entry(id, panel, content, |c, font| {
            if let Some((s, q)) = &c.mesh
                && *s == state
            {
                return q.clone();
            }
            let q = Arc::new(layout::mesh(panel, content, &c.layout, font, state));
            c.mesh = Some((state, q.clone()));
            q
        })
    }

    /// Forgets panels that no longer exist.
    pub(crate) fn retain(&mut self, live: &std::collections::BTreeSet<u64>) {
        let cache = self.cache.get_mut().unwrap_or_else(|e| e.into_inner());
        cache.retain(|id, _| live.contains(id));
        self.states.retain(|id, _| live.contains(id));
        self.grabs.retain(|_, g| live.contains(&g.panel));
    }

    /// Panels with a cached layout (for tests and the leak check).
    pub fn cached_panels(&self) -> usize {
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
}
