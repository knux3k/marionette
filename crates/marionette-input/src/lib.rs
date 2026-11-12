//! Marionette input: named actions bound to devices in a text file, applied once per tick
//! in [`Stage::Input`] into the `ActionState` resource.
//!
//! Three sources feed one queue of [`InputEvent`]s keyed by tick:
//! - live devices (the window pushes key/mouse events at the current tick),
//! - a scripted event file (`*.inputs.json`),
//! - a recording, which is the same file format: every change of an action's value, at the
//!   tick it happened. Replaying a recording reproduces the run exactly, whatever produced it.
//!
//! Action map file (`*.actions.json`, path set by the `InputConfig` scene resource):
//! ```json
//! { "format": "marionette.input-map", "version": 1, "actions": {
//!     "move_x": { "bindings": [ { "input": "key:KeyD", "scale": 1 }, { "input": "key:KeyA", "scale": -1 },
//!                               { "input": "axis:LeftStickX", "scale": 1 } ] },
//!     "jump":   { "bindings": [ { "input": "key:Space", "scale": 1 }, { "input": "pad:South", "scale": 1 } ] } } }
//! ```
//! An action's value is the sum of `device value * scale` over its bindings, clamped to -1..1.
//! Input names: `key:<KeyCode>` (winit names such as `KeyW`, `Space`, `ArrowLeft`),
//! `mouse:Left|Right|Middle`, `pad:<Button>`, `axis:<Axis>`.

use bevy_ecs::prelude::*;
use bevy_reflect::Reflect;
use marionette_core::actions::ActionState;
use marionette_core::builtin::SimClock;
use marionette_core::module::{Module, SceneDir, Stage};
use marionette_core::{Diagnostic, Errors, Registry, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

pub mod codes {
    pub const MAP: &str = "input.map";
    pub const EVENTS: &str = "input.events";
}

pub const MAP_FORMAT: &str = "marionette.input-map";
pub const EVENTS_FORMAT: &str = "marionette.input-events";

/// Turns input on for a scene: the action map file, relative to the scene file.
#[derive(Resource, Reflect, Debug, Clone, PartialEq)]
pub struct InputConfig {
    pub action_map: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub input: String,
    #[serde(default = "one")]
    pub scale: f32,
}

fn one() -> f32 {
    1.0
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct ActionDef {
    pub bindings: Vec<Binding>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Resource)]
#[serde(deny_unknown_fields)]
pub struct ActionMap {
    pub format: String,
    pub version: u32,
    pub actions: BTreeMap<String, ActionDef>,
}

/// One input change at a tick: either a device input (`input`) or an action set directly
/// (`action`). Exactly one of the two.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InputEvent {
    pub tick: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    pub value: f32,
}

/// The scripted/recorded event file.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EventsFile {
    pub format: String,
    pub version: u32,
    pub events: Vec<InputEvent>,
}

impl EventsFile {
    pub fn new(events: Vec<InputEvent>) -> Self {
        Self {
            format: EVENTS_FORMAT.into(),
            version: 1,
            events,
        }
    }
    /// Canonical text: one event per line, stable order (by tick, then as recorded).
    pub fn to_text(&self) -> String {
        let mut out = format!(
            "{{\n  \"format\": \"{}\",\n  \"version\": {},\n  \"events\": [\n",
            self.format, self.version
        );
        for (i, e) in self.events.iter().enumerate() {
            let line = serde_json::to_string(e).expect("events serialize");
            out.push_str("    ");
            out.push_str(&line);
            out.push_str(if i + 1 < self.events.len() { ",\n" } else { "\n" });
        }
        out.push_str("  ]\n}\n");
        out
    }
}

/// Pending events, by tick. Future events survive snapshots.
#[derive(Resource, Serialize, Deserialize, Debug, Clone, Default)]
pub struct InputQueue {
    pub events: BTreeMap<u64, Vec<InputEvent>>,
}

/// Raw device values and directly set actions (not part of the hash: a replay sets actions
/// directly where the original run pressed keys, and both must hash the same).
#[derive(Resource, Serialize, Deserialize, Debug, Clone, Default)]
pub struct DeviceState {
    pub devices: BTreeMap<String, f32>,
    pub direct: BTreeMap<String, f32>,
}

/// Every action change so far; written out as a replayable events file.
#[derive(Resource, Serialize, Deserialize, Debug, Clone, Default)]
pub struct InputRecording {
    pub events: Vec<InputEvent>,
}

fn bad(code: &'static str, file: &str, msg: String, fix: &str) -> Errors {
    Diagnostic::error(code, msg)
        .file(file)
        .fix(fix.to_string())
        .into()
}

pub fn parse_map(text: &str, file: &str) -> Result<ActionMap> {
    let map: ActionMap = serde_json::from_str(text).map_err(|e| {
        Errors::from(
            Diagnostic::error(codes::MAP, format!("invalid action map: {e}"))
                .file(file)
                .at(Some(e.line()), Some(e.column()))
                .fix("see the action map format in marionette-input's documentation"),
        )
    })?;
    if map.format != MAP_FORMAT || map.version != 1 {
        return Err(bad(
            codes::MAP,
            file,
            "not a version 1 action map".into(),
            "set \"format\": \"marionette.input-map\", \"version\": 1",
        ));
    }
    for (name, def) in &map.actions {
        for b in &def.bindings {
            if !["key:", "mouse:", "pad:", "axis:"]
                .iter()
                .any(|p| b.input.starts_with(p))
            {
                return Err(bad(
                    codes::MAP,
                    file,
                    format!("action `{name}`: unknown input `{}`", b.input),
                    "inputs start with key:, mouse:, pad: or axis:",
                ));
            }
        }
    }
    Ok(map)
}

/// Reads and checks an events file (script or recording).
pub fn parse_events(text: &str, file: &str) -> Result<Vec<InputEvent>> {
    let f: EventsFile = serde_json::from_str(text).map_err(|e| {
        Errors::from(
            Diagnostic::error(codes::EVENTS, format!("invalid input events: {e}"))
                .file(file)
                .at(Some(e.line()), Some(e.column())),
        )
    })?;
    if f.format != EVENTS_FORMAT || f.version != 1 {
        return Err(bad(
            codes::EVENTS,
            file,
            "not a version 1 events file".into(),
            "set \"format\": \"marionette.input-events\", \"version\": 1",
        ));
    }
    check_events(&f.events).map_err(|e| e.with_file(file))?;
    Ok(f.events)
}

pub fn check_events(events: &[InputEvent]) -> Result<()> {
    for (i, e) in events.iter().enumerate() {
        if e.input.is_some() == e.action.is_some() {
            return Err(Diagnostic::error(
                codes::EVENTS,
                format!("event {i}: needs exactly one of `input` and `action`"),
            )
            .pointer(format!("/events/{i}"))
            .into());
        }
        if !e.value.is_finite() {
            return Err(Diagnostic::error(
                codes::EVENTS,
                format!("event {i}: value must be a finite number"),
            )
            .pointer(format!("/events/{i}/value"))
            .into());
        }
    }
    Ok(())
}

/// Queues events to be applied at their ticks. Events for ticks already simulated are an error.
pub fn feed(world: &mut World, events: &[InputEvent]) -> Result<()> {
    check_events(events)?;
    let now = world.resource::<SimClock>().tick;
    if let Some(late) = events.iter().find(|e| e.tick < now) {
        return Err(Diagnostic::error(
            codes::EVENTS,
            format!("event for tick {} but the simulation is at tick {now}", late.tick),
        )
        .fix("events must be at the current tick or later")
        .into());
    }
    let mut q = world.get_resource_or_insert_with(InputQueue::default);
    for e in events {
        q.events.entry(e.tick).or_default().push(e.clone());
    }
    if world.get_resource::<ActionState>().is_none() {
        world.insert_resource(ActionState::default());
    }
    Ok(())
}

/// Every action change so far, as a replayable events file.
pub fn recording(world: &World) -> EventsFile {
    EventsFile::new(
        world
            .get_resource::<InputRecording>()
            .map(|r| r.events.clone())
            .unwrap_or_default(),
    )
}

pub struct InputModule;

#[derive(Serialize, Deserialize)]
struct Saved {
    queue: InputQueue,
    devices: DeviceState,
    recording: InputRecording,
}

impl Module for InputModule {
    fn name(&self) -> &'static str {
        "input"
    }

    fn register(&self, r: &mut Registry) {
        r.register_resource::<InputConfig>();
        r.register_resource::<ActionState>();
    }

    fn add_systems(&self, schedule: &mut Schedule) {
        schedule.add_systems(apply_input.in_set(Stage::Input));
    }

    fn init(&self, world: &mut World) -> Result<()> {
        let Some(cfg) = world.get_resource::<InputConfig>().cloned() else {
            return Ok(());
        };
        let dir = world
            .get_resource::<SceneDir>()
            .map(|d| d.0.clone())
            .unwrap_or_default();
        let path = dir.join(&cfg.action_map);
        let text = std::fs::read_to_string(&path).map_err(|e| {
            bad(
                codes::MAP,
                &cfg.action_map,
                format!("cannot read action map {}: {e}", cfg.action_map),
                "the path is relative to the scene file",
            )
        })?;
        let map = parse_map(&text, &cfg.action_map)?;
        world.insert_resource(map);
        if world.get_resource::<ActionState>().is_none() {
            world.insert_resource(ActionState::default());
        }
        Ok(())
    }

    fn save(&self, world: &World) -> Option<Vec<u8>> {
        let saved = Saved {
            queue: world.get_resource::<InputQueue>().cloned().unwrap_or_default(),
            devices: world.get_resource::<DeviceState>().cloned().unwrap_or_default(),
            recording: world
                .get_resource::<InputRecording>()
                .cloned()
                .unwrap_or_default(),
        };
        Some(serde_json::to_vec(&saved).expect("input state serializes"))
    }

    fn load(&self, world: &mut World, bytes: &[u8]) -> Result<()> {
        let s: Saved = serde_json::from_slice(bytes).map_err(|e| {
            Errors::from(Diagnostic::error(
                codes::EVENTS,
                format!("cannot restore input state: {e}"),
            ))
        })?;
        world.insert_resource(s.queue);
        world.insert_resource(s.devices);
        world.insert_resource(s.recording);
        Ok(())
    }
}

/// Applies this tick's events and recomputes every action value.
fn apply_input(world: &mut World) {
    if world.get_resource::<ActionState>().is_none() {
        return;
    }
    let tick = world.resource::<SimClock>().tick;
    let due = world
        .get_resource_mut::<InputQueue>()
        .and_then(|mut q| q.events.remove(&tick))
        .unwrap_or_default();
    let mut dev = world.remove_resource::<DeviceState>().unwrap_or_default();
    for e in &due {
        match (&e.input, &e.action) {
            (Some(i), _) => {
                dev.devices.insert(i.clone(), e.value);
            }
            (_, Some(a)) => {
                dev.direct.insert(a.clone(), e.value);
            }
            _ => {}
        }
    }
    let mut values = BTreeMap::new();
    if let Some(map) = world.get_resource::<ActionMap>() {
        for (name, def) in &map.actions {
            let sum: f32 = def
                .bindings
                .iter()
                .map(|b| dev.devices.get(&b.input).copied().unwrap_or(0.0) * b.scale)
                .sum();
            values.insert(name.clone(), sum.clamp(-1.0, 1.0));
        }
    }
    for (a, v) in &dev.direct {
        values.insert(a.clone(), *v);
    }
    let mut state = world.resource_mut::<ActionState>();
    let old = std::mem::take(&mut state.values);
    let mut changes = Vec::new();
    for (a, v) in &values {
        if old.get(a).copied().unwrap_or(0.0) != *v {
            changes.push(InputEvent {
                tick,
                input: None,
                action: Some(a.clone()),
                value: *v,
            });
        }
    }
    state.previous = old;
    state.values = values;
    world.insert_resource(dev);
    if !changes.is_empty() {
        world
            .get_resource_or_insert_with(InputRecording::default)
            .events
            .extend(changes);
    }
}

/// Reads an events file from disk.
pub fn load_events(path: &Path, shown: &str) -> Result<Vec<InputEvent>> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        bad(
            codes::EVENTS,
            shown,
            format!("cannot read {shown}: {e}"),
            "check the path",
        )
    })?;
    parse_events(&text, shown)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map_error(text: &str) -> marionette_core::Diagnostic {
        let e = parse_map(text, "a.actions.json").expect_err("must be refused");
        assert_eq!(e.0.len(), 1);
        e.0[0].clone()
    }

    #[test]
    fn a_valid_map_parses_with_default_scale() {
        let m = parse_map(
            r#"{"format":"marionette.input-map","version":1,"actions":{"jump":{"bindings":[{"input":"key:Space"}]}}}"#,
            "a.actions.json",
        )
        .unwrap();
        assert_eq!(m.actions["jump"].bindings[0].scale, 1.0);
    }

    #[test]
    fn broken_json_reports_its_position() {
        let d =
            map_error("{\n  \"format\": \"marionette.input-map\",\n  \"version\": 1,\n  \"actions\": {,}\n}");
        assert_eq!(d.code, codes::MAP);
        assert_eq!(d.file.as_deref(), Some("a.actions.json"));
        assert_eq!(d.line, Some(4));
    }

    #[test]
    fn wrong_format_or_version_is_refused() {
        let d = map_error(r#"{"format":"something-else","version":1,"actions":{}}"#);
        assert!(d.message.contains("not a version 1 action map"), "{}", d.message);
        let d = map_error(r#"{"format":"marionette.input-map","version":2,"actions":{}}"#);
        assert!(d.message.contains("not a version 1 action map"), "{}", d.message);
    }

    #[test]
    fn unknown_fields_and_inputs_are_refused() {
        let d = map_error(r#"{"format":"marionette.input-map","version":1,"actions":{},"extra":1}"#);
        assert!(d.message.contains("unknown field"), "{}", d.message);
        let d = map_error(
            r#"{"format":"marionette.input-map","version":1,"actions":{"fire":{"bindings":[{"input":"joystick:Trigger"}]}}}"#,
        );
        assert!(
            d.message
                .contains("action `fire`: unknown input `joystick:Trigger`"),
            "{}",
            d.message
        );
    }

    #[test]
    fn events_need_exactly_one_target_and_a_finite_value() {
        let both = InputEvent {
            tick: 0,
            input: Some("key:KeyA".into()),
            action: Some("left".into()),
            value: 1.0,
        };
        let e = check_events(&[both]).unwrap_err();
        assert_eq!(e.0[0].pointer.as_deref(), Some("/events/0"));
        let nan = InputEvent {
            tick: 0,
            input: Some("key:KeyA".into()),
            action: None,
            value: f32::NAN,
        };
        let e = check_events(&[nan]).unwrap_err();
        assert_eq!(e.0[0].pointer.as_deref(), Some("/events/0/value"));
        let e = parse_events(
            r#"{"format":"marionette.input-events","version":3,"events":[]}"#,
            "x.inputs.json",
        )
        .unwrap_err();
        assert_eq!(e.0[0].code, codes::EVENTS);
    }
}
