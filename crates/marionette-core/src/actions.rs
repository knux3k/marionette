//! Action state: the value of every named input action this tick and last tick. Filled by
//! the input module (`marionette-input`) from live devices, scripts or recordings; read by
//! gameplay systems (for example the physics character controller). Part of the scene
//! state, so it is saved, hashed and restored like any component.

use bevy_ecs::prelude::*;
use bevy_reflect::Reflect;
use std::collections::BTreeMap;

/// Threshold above which a button-like action counts as pressed.
pub const PRESSED: f32 = 0.5;

#[derive(Resource, Reflect, Debug, Clone, Default, PartialEq)]
pub struct ActionState {
    /// Value of each action this tick (buttons 0 or 1, axes -1..1).
    pub values: BTreeMap<String, f32>,
    /// Values on the previous tick, for "just pressed" checks.
    pub previous: BTreeMap<String, f32>,
}

impl ActionState {
    pub fn value(&self, action: &str) -> f32 {
        self.values.get(action).copied().unwrap_or(0.0)
    }
    pub fn pressed(&self, action: &str) -> bool {
        self.value(action) > PRESSED
    }
    pub fn just_pressed(&self, action: &str) -> bool {
        self.pressed(action) && self.previous.get(action).copied().unwrap_or(0.0) <= PRESSED
    }
}
