//! Minimal UI for HUDs and menus, as plain components. Each UI entity has a [`UiNode`]
//! (where it sits on screen) plus one look: [`UiRect`], [`UiText`] or [`UiImage`].
//! A [`UiButton`] sets an input action while it is clicked. The renderer draws UI in
//! screen pixels on top of the world.

use crate::spatial::{Color, Vec2};
use bevy_ecs::prelude::*;
use bevy_reflect::Reflect;

/// The screen point a node is placed relative to.
#[derive(Reflect, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Anchor {
    #[default]
    TopLeft,
    Top,
    TopRight,
    Left,
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl Anchor {
    /// Fractions of the screen size: (0, 0) is top-left, (1, 1) bottom-right.
    pub fn fraction(self) -> (f32, f32) {
        match self {
            Anchor::TopLeft => (0.0, 0.0),
            Anchor::Top => (0.5, 0.0),
            Anchor::TopRight => (1.0, 0.0),
            Anchor::Left => (0.0, 0.5),
            Anchor::Center => (0.5, 0.5),
            Anchor::Right => (1.0, 0.5),
            Anchor::BottomLeft => (0.0, 1.0),
            Anchor::Bottom => (0.5, 1.0),
            Anchor::BottomRight => (1.0, 1.0),
        }
    }
}

/// Placement in screen pixels. The node's own anchor point (same `anchor`) is put at the
/// screen's anchor point plus `offset` (+x right, +y down). Higher `layer` draws on top.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq)]
pub struct UiNode {
    pub anchor: Anchor,
    pub offset: Vec2,
    pub size: Vec2,
    pub layer: i32,
}

/// A filled rectangle.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq)]
pub struct UiRect {
    pub color: Color,
}

/// Text in the built-in bitmap font, `size` pixels per character row (multiples of 8 are
/// crisp), starting at the node's top-left corner. Letters are shown in upper case.
#[derive(Component, Reflect, Debug, Clone, PartialEq)]
pub struct UiText {
    pub text: String,
    pub size: f32,
    pub color: Color,
}

/// An image (PNG relative to the scene, or a `builtin:` image) stretched over the node.
#[derive(Component, Reflect, Debug, Clone, PartialEq)]
pub struct UiImage {
    pub image: String,
    pub color: Color,
}

/// Clicking the node sets input `action` to 1 for one tick.
#[derive(Component, Reflect, Debug, Clone, PartialEq)]
pub struct UiButton {
    pub action: String,
}

/// Screen rectangle (x, y, width, height in pixels, y down) of a node on a `w` x `h` screen.
pub fn layout(node: &UiNode, w: f32, h: f32) -> (f32, f32, f32, f32) {
    let (fx, fy) = node.anchor.fraction();
    let x = fx * w + node.offset.x - fx * node.size.x;
    let y = fy * h + node.offset.y - fy * node.size.y;
    (x, y, node.size.x, node.size.y)
}
