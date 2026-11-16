//! World canvases (decision 0202): screen UI nodes (`UiNode` with `UiRect`, `UiText`,
//! `UiClip`, `UiButton`) marked `UiOnCanvas` are laid out in a `UiCanvas`'s pixels and drawn as
//! world quads with the panel font; a world pointer presses their buttons. Pure functions over
//! plain data, so the drawing and the hit test can be checked without a world.

use crate::components::UiCanvas;
use crate::font::FontData;
use crate::layout::{Quad, Rect};
use marionette_core::spatial::Color;
use marionette_core::ui::{UiClip, UiNode, clip_quad, hit, layout};

/// What one node on a canvas shows.
#[derive(Debug, Clone, PartialEq)]
pub enum CanvasLook {
    Rect(Color),
    /// Bitmap-font text as on the screen: `size` pixels per row, upper case.
    Text {
        text: String,
        size: f32,
        color: Color,
    },
    /// Shows nothing (only a button).
    None,
}

/// One node of a canvas, as the module reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct CanvasNode {
    pub id: u64,
    pub node: UiNode,
    pub look: CanvasLook,
    pub clip: Option<UiClip>,
    /// Its `UiButton` action ("" for none).
    pub action: String,
}

fn rgba(c: Color) -> [f32; 4] {
    [c.r, c.g, c.b, c.a]
}

/// Draw order on the screen: by layer, then by scene id.
pub fn sort(nodes: &mut [CanvasNode]) {
    nodes.sort_by_key(|n| (n.node.layer, n.id));
}

/// The quads of a canvas's nodes (already in draw order), in canvas meters with the origin at
/// the top-left corner and y down, as panel quads are.
pub fn quads(canvas: &UiCanvas, nodes: &[CanvasNode], font: &FontData) -> Vec<Quad> {
    let (pw, ph) = (canvas.pixels.x.max(1.0), canvas.pixels.y.max(1.0));
    let (kx, ky) = (canvas.size.x / pw, canvas.size.y / ph);
    let mut out = Vec::new();
    // One quad given in pixels with its texture corners at top-left and bottom-right, cut to
    // the node's clip, moved to meters.
    let mut push = |q: [f32; 4],
                    uv_tl: [f32; 2],
                    uv_br: [f32; 2],
                    color: [f32; 4],
                    clip: Option<(f32, f32, f32, f32)>| {
        let cut = match clip {
            Some(c) => clip_quad(q, uv_tl, uv_br, c),
            None => Some((q, uv_tl, uv_br)),
        };
        if let Some(([x0, y0, x1, y1], tl, br)) = cut {
            out.push(Quad {
                rect: Rect {
                    x: x0 * kx,
                    y: y0 * ky,
                    w: (x1 - x0) * kx,
                    h: (y1 - y0) * ky,
                },
                // Panel quads keep v0 at the bottom edge.
                uv: [tl[0], br[1], br[0], tl[1]],
                color,
                layer: 0,
            });
        }
    };
    for n in nodes {
        let clip = n.clip.map(|c| c.rect(pw, ph));
        let (x, y, w, h) = layout(&n.node, pw, ph);
        match &n.look {
            CanvasLook::Rect(c) => {
                if c.a > 0.0 && w > 0.0 && h > 0.0 {
                    let [u, v] = font.white_uv;
                    push([x, y, x + w, y + h], [u, v], [u, v], rgba(*c), clip);
                }
            }
            CanvasLook::Text { text, size, color } => {
                let s = *size;
                let baseline = y + font.ascent * s;
                let mut pen = x;
                for ch in text.chars().flat_map(char::to_uppercase) {
                    let g = *font.glyph(ch);
                    if ch != ' ' {
                        let q = [
                            pen + g.rect[0] * s,
                            baseline - g.rect[3] * s,
                            pen + g.rect[2] * s,
                            baseline - g.rect[1] * s,
                        ];
                        // Glyph uv: v0 is the bottom edge, so the top-left corner is (u0, v1).
                        push(q, [g.uv[0], g.uv[3]], [g.uv[2], g.uv[1]], rgba(*color), clip);
                    }
                    pen += g.advance * s;
                }
            }
            CanvasLook::None => {}
        }
    }
    out
}

/// The action of the top-most button under canvas point (`x`, `y`) in pixels, as the window
/// picks a screen button: highest layer, then highest scene id.
pub fn button_at(canvas: &UiCanvas, nodes: &[CanvasNode], x: f32, y: f32) -> Option<String> {
    let (pw, ph) = (canvas.pixels.x.max(1.0), canvas.pixels.y.max(1.0));
    nodes
        .iter()
        .filter(|n| !n.action.is_empty() && hit(&n.node, n.clip.as_ref(), x, y, pw, ph))
        .max_by_key(|n| (n.node.layer, n.id))
        .map(|n| n.action.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use marionette_core::spatial::Vec2;
    use marionette_core::ui::Anchor;

    fn node(id: u64, x: f32, y: f32, w: f32, h: f32, layer: i32, action: &str) -> CanvasNode {
        CanvasNode {
            id,
            node: UiNode {
                anchor: Anchor::TopLeft,
                offset: Vec2 { x, y },
                size: Vec2 { x: w, y: h },
                layer,
            },
            look: CanvasLook::Rect(Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            }),
            clip: None,
            action: action.to_string(),
        }
    }

    #[test]
    fn nodes_are_laid_out_in_the_canvas_pixels_and_scaled_to_meters() {
        let canvas = UiCanvas {
            size: Vec2 { x: 1.0, y: 0.5 },
            pixels: Vec2 { x: 1000.0, y: 500.0 },
            ..UiCanvas::default()
        };
        let font = FontData::builtin();
        let q = quads(&canvas, &[node(1, 100.0, 50.0, 200.0, 100.0, 0, "")], &font);
        assert_eq!(q.len(), 1);
        let r = q[0].rect;
        assert!((r.x - 0.1).abs() < 1e-6 && (r.y - 0.05).abs() < 1e-6);
        assert!((r.w - 0.2).abs() < 1e-6 && (r.h - 0.1).abs() < 1e-6);
    }

    #[test]
    fn the_top_most_button_takes_the_press_and_clips_cut_it() {
        let canvas = UiCanvas::default();
        let mut nodes = vec![
            node(2, 0.0, 0.0, 100.0, 100.0, 5, "top"),
            node(1, 0.0, 0.0, 100.0, 100.0, 1, "below"),
        ];
        sort(&mut nodes);
        assert_eq!(nodes[0].id, 1);
        assert_eq!(button_at(&canvas, &nodes, 50.0, 50.0).as_deref(), Some("top"));
        assert_eq!(button_at(&canvas, &nodes, 150.0, 50.0), None);
        nodes[1].clip = Some(UiClip {
            anchor: Anchor::TopLeft,
            offset: Vec2 { x: 60.0, y: 0.0 },
            size: Vec2 { x: 40.0, y: 100.0 },
        });
        assert_eq!(button_at(&canvas, &nodes, 50.0, 50.0).as_deref(), Some("below"));
    }

    #[test]
    fn text_draws_one_quad_per_letter_inside_its_clip() {
        let canvas = UiCanvas::default();
        let font = FontData::builtin();
        let mut n = node(1, 10.0, 10.0, 200.0, 16.0, 0, "");
        n.look = CanvasLook::Text {
            text: "ab c".into(),
            size: 16.0,
            color: Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            },
        };
        assert_eq!(quads(&canvas, std::slice::from_ref(&n), &font).len(), 3);
        n.clip = Some(UiClip {
            anchor: Anchor::TopLeft,
            offset: Vec2 { x: 500.0, y: 0.0 },
            size: Vec2 { x: 10.0, y: 10.0 },
        });
        assert!(quads(&canvas, &[n], &font).is_empty());
    }
}
