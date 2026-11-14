//! The editor's state and commands, with no window: the window only turns events into calls
//! here, so every command is testable headless.

use crate::camera::FlyCamera;
use crate::edit::{self, Call, Plan, UNDO_KEEP};
use crate::inspect::{self, Inspected};
use crate::overlay::{self, Layout, OverlayInput};
use crate::pick::{self, Pickable};
use glam::{Vec2, Vec3};
use marionette_core::{Errors, Sim};
use marionette_render::{FrameData, View, extract};
use serde_json::Value;
use std::collections::VecDeque;

/// What the editor needs from the program that opens it (the CLI passes the engine and the
/// operation table).
pub trait Host {
    /// A fresh simulation of the scene file as it is on disk now.
    fn load(&mut self) -> Result<Sim, Errors>;
    /// Runs one operation (`component_set`, `entity_create`, `entity_delete`).
    fn call(&mut self, op: &str, input: Value) -> Result<Value, Errors>;
    /// A device input for the game while it plays through its own camera (queue it for
    /// `sim.tick()`).
    fn device(&mut self, _sim: &mut Sim, _input: &str, _value: f32) {}
    /// Once per frame after the simulation caught up (play sounds).
    fn after_ticks(&mut self, _sim: &mut Sim) {}
}

/// Keys and mouse help for the bottom bar, most useful first.
pub const HELP: &str = "RMB+WASD FLY  CLICK SELECT  F FRAME  P PLAY  N STEP  R RESET  C GAME CAM  ARROWS MOVE  \
                        CTRL+D COPY  DEL  CTRL+Z UNDO  [ ] NEXT  G GRID  TAB PANELS";

pub struct Editor {
    /// Scene file, as shown and as passed to edit operations.
    pub scene: String,
    pub sim: Sim,
    pub camera: FlyCamera,
    pub selected: Option<u64>,
    pub panels: bool,
    pub grid: bool,
    /// Look through the scene's own camera (and send keys to the game while playing).
    pub game_view: bool,
    pub playing: bool,
    /// Last message for the status bar.
    pub message: String,
    pub list_top: usize,
    entities: Vec<(u64, Option<String>)>,
    undo: VecDeque<Vec<Call>>,
    /// Last click position and how many hits deep it went (clicking again cycles).
    cycle: Option<(Vec2, usize)>,
}

impl Editor {
    /// Starts at the scene's first camera, or looking at the whole scene when it has none.
    pub fn new(scene: impl Into<String>, sim: Sim) -> Self {
        let frame = extract(&sim.world);
        let camera = match frame.cameras.first() {
            Some(c) => FlyCamera::from_scene_camera(c),
            None => {
                let mut c = FlyCamera::default();
                let items = pick::pickables(&frame, &sim.world, &|_| None);
                if let Some((lo, hi)) = pick::world_bounds(&items) {
                    c.frame_box(lo, hi);
                }
                c
            }
        };
        let entities = inspect::entity_list(&sim.world);
        Self {
            scene: scene.into(),
            sim,
            camera,
            selected: None,
            panels: true,
            grid: true,
            game_view: false,
            playing: false,
            message: String::new(),
            list_top: 0,
            entities,
            undo: VecDeque::new(),
            cycle: None,
        }
    }

    pub fn entities(&self) -> &[(u64, Option<String>)] {
        &self.entities
    }

    /// Steps undo can take back.
    pub fn undo_depth(&self) -> usize {
        self.undo.len()
    }

    /// The view the window shows: the editor camera, or the scene camera in game view.
    pub fn view(&self, frame: &FrameData, aspect: f32) -> View {
        match (self.game_view, frame.cameras.first()) {
            (true, Some(c)) => c.view(aspect),
            _ => self.camera.view(aspect),
        }
    }

    /// Everything clickable this frame.
    pub fn items(&self, frame: &FrameData, bounds: &dyn Fn(&str) -> Option<(Vec3, Vec3)>) -> Vec<Pickable> {
        pick::pickables(frame, &self.sim.world, bounds)
    }

    pub fn inspected(&self) -> Option<Inspected> {
        inspect::inspect(&self.sim.world, self.sim.registry(), self.selected?)
    }

    /// Selects `id` (or nothing) and scrolls the entity list to show it.
    pub fn select(&mut self, id: Option<u64>, rows: usize) {
        self.selected = id;
        if let Some(i) = id.and_then(|id| self.entities.iter().position(|(e, _)| *e == id))
            && rows > 0
        {
            if i < self.list_top {
                self.list_top = i;
            } else if i >= self.list_top + rows {
                self.list_top = i + 1 - rows;
            }
        }
    }

    /// The next (`delta` 1) or previous (-1) entity in the list.
    pub fn select_step(&mut self, delta: i64, rows: usize) {
        if self.entities.is_empty() {
            return;
        }
        let n = self.entities.len() as i64;
        let i = match self
            .selected
            .and_then(|id| self.entities.iter().position(|(e, _)| *e == id))
        {
            Some(i) => (i as i64 + delta).rem_euclid(n),
            None if delta < 0 => n - 1,
            None => 0,
        };
        self.select(Some(self.entities[i as usize].0), rows);
    }

    /// Scrolls the entity list by `rows` (positive: down).
    pub fn scroll_list(&mut self, rows: i64, shown: usize) {
        let max = self.entities.len().saturating_sub(shown) as i64;
        self.list_top = (self.list_top as i64 + rows).clamp(0, max.max(0)) as usize;
    }

    /// A left click at (`x`, `y`): an entity list row, or whatever is under the cursor in
    /// the view. Clicking the same spot again selects the next thing behind.
    pub fn click(&mut self, items: &[Pickable], view: &View, layout: &Layout, x: f32, y: f32) {
        if let Some(row) = layout.list_row_at(x, y) {
            if let Some((id, _)) = self.entities.get(self.list_top + row) {
                self.selected = Some(*id);
            }
            self.cycle = None;
            return;
        }
        if layout.over_ui(x, y) {
            return;
        }
        let hits = pick::hits(items, view, x, y, layout.w, layout.h);
        let here = Vec2::new(x, y);
        let depth = match self.cycle {
            Some((p, d)) if p.distance(here) < 3.0 && !hits.is_empty() => (d + 1) % hits.len(),
            _ => 0,
        };
        self.cycle = Some((here, depth));
        let id = hits.get(depth).map(|h| h.entity);
        self.select(id, layout.list_rows());
    }

    /// Moves the camera so the selection (or the whole scene) fills the view.
    pub fn frame_selection(&mut self, items: &[Pickable]) {
        let chosen: Vec<&Pickable> = match self.selected {
            Some(id) => items.iter().filter(|i| i.entity == id).collect(),
            None => items.iter().collect(),
        };
        if let Some((lo, hi)) = pick::world_bounds(chosen) {
            self.camera.frame_box(lo, hi);
        }
    }

    /// Draws panels, markers, highlight and grid into the frame.
    pub fn overlay(&self, frame: &mut FrameData, items: &[Pickable], view: &View, layout: Layout) {
        let inspected = self.inspected();
        let status = self.status();
        overlay::draw(
            frame,
            &OverlayInput {
                layout,
                view,
                items,
                selected: self.selected,
                entities: &self.entities,
                list_top: self.list_top,
                inspected: inspected.as_ref(),
                status: &status,
                help: HELP,
                grid: self.grid && !self.game_view,
            },
        );
    }

    /// The top bar.
    pub fn status(&self) -> String {
        let mode = if self.playing { "PLAYING" } else { "PAUSED" };
        let cam = if self.game_view {
            "GAME CAMERA"
        } else {
            "EDITOR CAMERA"
        };
        let speed = match self.camera.ortho_height {
            Some(h) => format!("ZOOM {}", inspect::number(h as f64)),
            None => format!("SPEED {} M/S", inspect::number(self.camera.speed as f64)),
        };
        let mut s = format!(
            "{}  {mode}  TICK {}  {} ENTITIES  {cam}  {speed}",
            self.scene,
            self.sim.tick(),
            self.entities.len()
        );
        if !self.message.is_empty() {
            s.push_str("  - ");
            s.push_str(&self.message);
        }
        s
    }

    /// Refreshes the entity list after the world changed shape (spawns, removals, reload).
    pub fn refresh(&mut self) {
        self.entities = inspect::entity_list(&self.sim.world);
        if let Some(id) = self.selected
            && !self.entities.iter().any(|(e, _)| *e == id)
        {
            self.selected = None;
        }
        self.list_top = self.list_top.min(self.entities.len().saturating_sub(1));
    }

    pub fn toggle_play(&mut self) {
        self.playing = !self.playing;
        self.message.clear();
    }

    /// One tick while paused.
    pub fn step(&mut self, host: &mut dyn Host) {
        self.playing = false;
        self.sim.step();
        host.after_ticks(&mut self.sim);
        self.refresh();
    }

    /// Back to the scene file as it is on disk (tick 0), paused.
    pub fn reset(&mut self, host: &mut dyn Host) {
        match host.load() {
            Ok(sim) => {
                self.sim = sim;
                self.playing = false;
                self.message = "reset to the scene file".into();
                self.refresh();
            }
            Err(e) => self.message = first_line(&e),
        }
    }

    fn can_edit(&mut self) -> bool {
        if self.sim.tick() != 0 || self.playing {
            self.message = "press R to reset to the scene file before editing".into();
            return false;
        }
        true
    }

    fn apply(&mut self, host: &mut dyn Host, plan: Result<Plan, String>) {
        let plan = match plan {
            Ok(p) => p,
            Err(m) => return self.message = m,
        };
        match host.call(&plan.call.0, plan.call.1.clone()) {
            Ok(_) => {
                self.undo.push_back(plan.undo);
                while self.undo.len() > UNDO_KEEP {
                    self.undo.pop_front();
                }
                self.reset(host);
                self.message = plan.done;
            }
            Err(e) => self.message = first_line(&e),
        }
    }

    /// Moves the selected entity by `delta` meters (writes the scene file).
    pub fn nudge(&mut self, host: &mut dyn Host, delta: Vec3) {
        if !self.can_edit() {
            return;
        }
        let Some(e) = self.inspected() else { return };
        let plan = edit::nudge(&self.scene, &e, delta);
        self.apply(host, plan);
    }

    /// Copies the selected entity under the next free id and selects the copy.
    pub fn duplicate(&mut self, host: &mut dyn Host) {
        if !self.can_edit() {
            return;
        }
        let Some(e) = self.inspected() else { return };
        let new_id = self.entities.iter().map(|(id, _)| *id).max().unwrap_or(0) + 1;
        let plan = edit::duplicate(&self.scene, &e, new_id);
        self.apply(host, Ok(plan));
        if self.entities.iter().any(|(id, _)| *id == new_id) {
            self.selected = Some(new_id);
        }
    }

    /// Deletes the selected entity.
    pub fn delete(&mut self, host: &mut dyn Host) {
        if !self.can_edit() {
            return;
        }
        let Some(e) = self.inspected() else { return };
        let plan = edit::delete(&self.scene, &e);
        self.apply(host, Ok(plan));
    }

    /// Takes back the last edit.
    pub fn undo(&mut self, host: &mut dyn Host) {
        if !self.can_edit() {
            return;
        }
        let Some(calls) = self.undo.pop_back() else {
            self.message = "nothing to undo".into();
            return;
        };
        for (op, input) in calls {
            if let Err(e) = host.call(&op, input) {
                self.message = first_line(&e);
                return;
            }
        }
        self.reset(host);
        self.message = "undone".into();
    }
}

fn first_line(e: &Errors) -> String {
    e.to_string()
        .lines()
        .next()
        .unwrap_or("error")
        .chars()
        .take(120)
        .collect()
}
