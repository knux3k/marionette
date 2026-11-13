//! Scene components of world-space UI. A panel is ONE entity: [`UiPanel`] (where and how big,
//! colors) plus [`UiContent`] (its items). One entity per panel, not one per button, keeps
//! scenes small, layout cached and large amounts of text cheap.
//!
//! Conventions: sizes are meters. A panel's front faces its local +Z; its local x is right and
//! y is up; the entity's `Transform` (or `Position`) places its centre. Items flow left to
//! right, top to bottom, across `columns` equal columns.

use bevy_ecs::prelude::*;
use bevy_reflect::Reflect;
use marionette_core::builtin::SceneRef;
use marionette_core::spatial::{Color, Vec2, Vec3};

/// How a panel turns toward the viewer (the first camera). Done in the simulation each tick, so
/// pointer hits agree with what is drawn.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UiFace {
    /// Keep the entity's own rotation.
    #[default]
    Fixed,
    /// Turn around the world's up axis only (stays upright).
    Yaw,
    /// Face the camera fully.
    Camera,
}

#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UiAlign {
    #[default]
    Left,
    Center,
    Right,
}

/// A panel placed in the world.
#[derive(Component, Reflect, Debug, Clone, PartialEq)]
pub struct UiPanel {
    /// Width and height in meters.
    pub size: Vec2,
    /// Equal columns across the panel (at least 1).
    pub columns: u32,
    /// Space between the edge and the items, and between items, in meters.
    pub padding: f32,
    pub gap: f32,
    /// Text height (the "em") in meters for labels that give no size. 0.04 is readable at arm's
    /// length (0.6 m), about 3.8 degrees.
    pub text_size: f32,
    pub face: UiFace,
    /// Beyond this distance from the viewer the panel is not drawn (0: always drawn).
    pub max_distance: f32,
    /// Pointers can use it.
    pub interactive: bool,
    pub background: Color,
    pub text_color: Color,
    /// Fill of widgets (buttons, slider tracks, list rows) and their highlight colors.
    pub widget: Color,
    pub hover: Color,
    pub accent: Color,
}

impl Default for UiPanel {
    fn default() -> Self {
        Self {
            size: Vec2 { x: 0.5, y: 0.3 },
            columns: 1,
            padding: 0.015,
            gap: 0.008,
            text_size: 0.03,
            face: UiFace::Fixed,
            max_distance: 0.0,
            interactive: true,
            background: Color {
                r: 0.02,
                g: 0.03,
                b: 0.05,
                a: 0.85,
            },
            text_color: Color {
                r: 0.92,
                g: 0.95,
                b: 1.0,
                a: 1.0,
            },
            widget: Color {
                r: 0.12,
                g: 0.17,
                b: 0.24,
                a: 1.0,
            },
            hover: Color {
                r: 0.2,
                g: 0.3,
                b: 0.42,
                a: 1.0,
            },
            accent: Color {
                r: 0.1,
                g: 0.55,
                b: 0.95,
                a: 1.0,
            },
        }
    }
}

/// What an item is. Values live in the item, so they are part of the scene state: a slider
/// the pointer moved is saved, hashed and replayed like any other component data.
#[derive(Reflect, Debug, Clone, PartialEq)]
pub enum UiWidget {
    /// Text, wrapped to the item's width. `size` 0 uses the panel's `text_size`.
    Label { text: String, size: f32, align: UiAlign },
    /// A button. Clicking sets input action `action` (when not empty) for the tick.
    Button { text: String, action: String },
    /// A switch: `on` flips on click.
    Toggle { text: String, on: bool },
    /// A slider from `min` to `max`, dragged along its track.
    Slider {
        text: String,
        min: f32,
        max: f32,
        value: f32,
    },
    /// A bar filled `value` (0..1).
    Progress { text: String, value: f32 },
    /// Rows of text, `visible` of them at a time from row `scroll`; clicking a row selects it
    /// (`selected`, -1 for none); a pointer's scroll axis moves `scroll`.
    List {
        rows: Vec<String>,
        visible: u32,
        scroll: f32,
        selected: i32,
    },
    /// Empty space.
    Spacer,
}

/// One item of a panel.
#[derive(Reflect, Debug, Clone, PartialEq)]
pub struct UiItem {
    /// Name given in events (empty for items nobody listens to).
    pub id: String,
    /// Columns taken (at least 1).
    pub span: u32,
    /// Height in meters; 0 picks one from the content.
    pub height: f32,
    pub widget: UiWidget,
}

impl UiItem {
    pub fn new(id: &str, widget: UiWidget) -> Self {
        Self {
            id: id.to_string(),
            span: 1,
            height: 0.0,
            widget,
        }
    }
}

/// The items of a panel, in flow order.
#[derive(Component, Reflect, Debug, Clone, PartialEq, Default)]
pub struct UiContent {
    pub items: Vec<UiItem>,
}

/// Places the panel relative to another entity each tick (a wrist panel, a panel that
/// follows the head). `offset` is in the target's own axes when `relative`, else in world
/// axes. `smoothing` is a time constant in seconds (0: no lag).
#[derive(Component, Reflect, Debug, Clone, PartialEq)]
pub struct UiAnchor {
    pub target: SceneRef,
    pub offset: Vec3,
    pub relative: bool,
    pub smoothing: f32,
}

/// Which font draws panel text. A scene resource; without it the small built-in font is used
/// (capital letters only). `font` is a TrueType or OpenType file relative to the scene.
#[derive(Resource, Reflect, Debug, Clone, PartialEq, Default)]
pub struct UiFontConfig {
    pub font: String,
}
