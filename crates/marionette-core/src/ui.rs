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

/// Clips a node to a screen rectangle placed like a [`UiNode`] (decision 0175): the parts of
/// the node outside it are not drawn and take no clicks. A scrolling list or a panned map
/// puts the same clip on every node inside its view.
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq)]
pub struct UiClip {
    pub anchor: Anchor,
    pub offset: Vec2,
    pub size: Vec2,
}

impl UiClip {
    /// Screen rectangle (x, y, width, height in pixels, y down) on a `w` x `h` screen.
    pub fn rect(&self, w: f32, h: f32) -> (f32, f32, f32, f32) {
        let node = UiNode {
            anchor: self.anchor,
            offset: self.offset,
            size: self.size,
            layer: 0,
        };
        layout(&node, w, h)
    }
}

/// Cuts the quad `x0..x1` by `y0..y1` (y down) with texture corners `uv0`, `uv1` down to the
/// clip rectangle `(cx, cy, cw, ch)`, moving the texture corners in step so nothing stretches.
/// None when nothing of it is left.
pub fn clip_quad(
    quad: [f32; 4],
    uv0: [f32; 2],
    uv1: [f32; 2],
    clip: (f32, f32, f32, f32),
) -> Option<([f32; 4], [f32; 2], [f32; 2])> {
    let [x0, y0, x1, y1] = quad;
    let (cx, cy, cw, ch) = clip;
    let (nx0, ny0) = (x0.max(cx), y0.max(cy));
    let (nx1, ny1) = (x1.min(cx + cw), y1.min(cy + ch));
    if nx1 <= nx0 || ny1 <= ny0 {
        return None;
    }
    // Texture coordinate at a point along each side, as a fraction of the quad.
    let u = |x: f32| uv0[0] + (uv1[0] - uv0[0]) * if x1 > x0 { (x - x0) / (x1 - x0) } else { 0.0 };
    let v = |y: f32| uv0[1] + (uv1[1] - uv0[1]) * if y1 > y0 { (y - y0) / (y1 - y0) } else { 0.0 };
    Some(([nx0, ny0, nx1, ny1], [u(nx0), v(ny0)], [u(nx1), v(ny1)]))
}

/// Clicking the node sets input `action` to 1 for one tick.
#[derive(Component, Reflect, Debug, Clone, PartialEq)]
pub struct UiButton {
    pub action: String,
}

/// Draws the node on a world canvas instead of the screen (decision 0202): the canvas entity
/// with scene id `canvas` (a `UiCanvas` of the `ui3d` module) gives the pixel size the node's
/// anchor and offset are measured in, and a world pointer on the canvas presses its
/// [`UiButton`]. The screen renderer and the window's mouse leave such nodes alone, so the same
/// HUD or menu code can draw on a screen or, unchanged, in the world (a VR menu).
#[derive(Component, Reflect, Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiOnCanvas {
    pub canvas: u64,
}

/// What a game asks of the window it runs in (decision 0174). The window reads it after every
/// frame's ticks; without it the window's own options decide. Headless runs ignore it, so it
/// never changes what a tick computes.
#[derive(Resource, Reflect, Debug, Clone, Copy, PartialEq, Default)]
pub struct WindowControl {
    /// Hide and lock the cursor while the window has focus (mouse look). False frees it, for
    /// example while a pause menu is open.
    pub lock_cursor: bool,
    /// Close the window after this frame (a menu's Quit).
    pub close: bool,
}

/// Hides screen UI under a layer (decision 0287): with `on`, screen nodes whose `layer` is below
/// `min_layer` are neither drawn nor clicked. A photo mode hides the game's interface while its
/// own overlay (a higher layer) stays; nodes on world canvases are not affected.
#[derive(Resource, Reflect, Debug, Clone, Copy, PartialEq, Default)]
pub struct ScreenUiCutoff {
    pub on: bool,
    pub min_layer: i32,
}

impl ScreenUiCutoff {
    /// True when a screen node on `layer` is shown.
    pub fn shows(cutoff: Option<&Self>, layer: i32) -> bool {
        cutoff.is_none_or(|c| !c.on || layer >= c.min_layer)
    }
}

/// Whether screen point (`x`, `y`) is on `node`, and inside `clip` when it has one.
pub fn hit(node: &UiNode, clip: Option<&UiClip>, x: f32, y: f32, w: f32, h: f32) -> bool {
    let inside = |(bx, by, bw, bh): (f32, f32, f32, f32)| x >= bx && x < bx + bw && y >= by && y < by + bh;
    inside(layout(node, w, h)) && clip.is_none_or(|c| inside(c.rect(w, h)))
}

/// Screen rectangle (x, y, width, height in pixels, y down) of a node on a `w` x `h` screen.
pub fn layout(node: &UiNode, w: f32, h: f32) -> (f32, f32, f32, f32) {
    let (fx, fy) = node.anchor.fraction();
    let x = fx * w + node.offset.x - fx * node.size.x;
    let y = fy * h + node.offset.y - fy * node.size.y;
    (x, y, node.size.x, node.size.y)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(x: f32, y: f32, w: f32, h: f32) -> UiClip {
        UiClip {
            anchor: Anchor::TopLeft,
            offset: Vec2 { x, y },
            size: Vec2 { x: w, y: h },
        }
    }

    #[test]
    fn a_quad_is_cut_to_the_clip_and_its_texture_follows() {
        let c = clip(10.0, 0.0, 100.0, 50.0).rect(800.0, 600.0);
        // Half of it hangs out to the left: the left half of the texture goes with it.
        let (q, uv0, uv1) = clip_quad([0.0, 0.0, 20.0, 20.0], [0.0, 0.0], [1.0, 1.0], c).unwrap();
        assert_eq!(q, [10.0, 0.0, 20.0, 20.0]);
        assert_eq!(uv0, [0.5, 0.0]);
        assert_eq!(uv1, [1.0, 1.0]);
        // Wholly inside: unchanged.
        let inside = clip_quad([20.0, 10.0, 30.0, 20.0], [0.2, 0.2], [0.4, 0.4], c).unwrap();
        assert_eq!(inside, ([20.0, 10.0, 30.0, 20.0], [0.2, 0.2], [0.4, 0.4]));
        // Wholly outside: gone.
        assert!(clip_quad([200.0, 0.0, 220.0, 20.0], [0.0, 0.0], [1.0, 1.0], c).is_none());
    }

    #[test]
    fn clicks_outside_the_clip_miss() {
        let node = UiNode {
            anchor: Anchor::TopLeft,
            offset: Vec2 { x: 0.0, y: 0.0 },
            size: Vec2 { x: 100.0, y: 100.0 },
            layer: 0,
        };
        let c = clip(50.0, 0.0, 100.0, 100.0);
        assert!(hit(&node, None, 10.0, 10.0, 800.0, 600.0));
        assert!(!hit(&node, Some(&c), 10.0, 10.0, 800.0, 600.0));
        assert!(hit(&node, Some(&c), 60.0, 10.0, 800.0, 600.0));
    }
}
