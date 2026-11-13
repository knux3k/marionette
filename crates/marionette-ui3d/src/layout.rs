//! Layout and mesh of one panel. Coordinates here are meters with the origin at the panel's
//! top-left corner and y pointing down; [`crate::module`] moves them into the world.

use crate::components::{UiAlign, UiContent, UiPanel, UiWidget};
use crate::font::FontData;
use marionette_core::spatial::Color;

/// A rectangle (top-left corner, size) in panel meters.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x <= self.x + self.w && y >= self.y && y <= self.y + self.h
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Layout {
    /// One rectangle per item, in item order.
    pub rects: Vec<Rect>,
    /// Some item ends below the panel (it is not drawn).
    pub overflow: bool,
}

/// A coloured, textured rectangle of the panel mesh. `layer` orders coplanar quads (0 is the
/// back); `uv` is `[u0, v0, u1, v1]` with v0 at the bottom edge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quad {
    pub rect: Rect,
    pub uv: [f32; 4],
    pub color: [f32; 4],
    pub layer: u8,
}

/// What a pointer is doing to a panel's items (changes colours only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PanelState {
    pub hover: Option<usize>,
    pub hover_row: Option<usize>,
    pub pressed: Option<usize>,
}

fn default_height(w: &UiWidget, s: f32, font: &FontData, width: f32) -> f32 {
    match w {
        UiWidget::Label { text, size, .. } => {
            let s = if *size > 0.0 { *size } else { s };
            wrap(font, text, s, width).len().max(1) as f32 * font.line_height * s
        }
        UiWidget::Button { .. } => 2.0 * s,
        UiWidget::Toggle { .. } => 1.6 * s,
        UiWidget::Slider { .. } => 2.4 * s,
        UiWidget::Progress { .. } => 1.6 * s,
        UiWidget::List { visible, .. } => (*visible).max(1) as f32 * row_height(s),
        UiWidget::Spacer => s,
    }
}

/// Height of one list row for text size `s`.
pub fn row_height(s: f32) -> f32 {
    1.5 * s
}

/// The slider track of an item rectangle: `(left, right, centre y)`; the knob travels
/// between left and right.
pub fn slider_track(r: Rect, s: f32) -> (f32, f32, f32) {
    (r.x + 0.5 * s, r.x + r.w - 0.5 * s, r.y + r.h - 0.7 * s)
}

pub fn compute(panel: &UiPanel, content: &UiContent, font: &FontData) -> Layout {
    let cols = panel.columns.max(1) as usize;
    let inner_w = (panel.size.x - 2.0 * panel.padding).max(0.0);
    let col_w = ((inner_w - panel.gap * (cols - 1) as f32) / cols as f32).max(0.0);
    let mut rects = vec![Rect::default(); content.items.len()];
    let (mut col, mut y, mut row_h) = (0usize, panel.padding, 0.0f32);
    let mut row: Vec<usize> = Vec::new();
    let mut overflow = false;
    let finish_row = |row: &mut Vec<usize>, rects: &mut [Rect], y: &mut f32, row_h: &mut f32| {
        for &i in row.iter() {
            rects[i].y = *y;
            // Items in a row share its height.
            rects[i].h = rects[i].h.max(0.0);
        }
        *y += *row_h + panel.gap;
        *row_h = 0.0;
        row.clear();
    };
    for (i, item) in content.items.iter().enumerate() {
        let span = (item.span.max(1) as usize).min(cols);
        if col + span > cols {
            finish_row(&mut row, &mut rects, &mut y, &mut row_h);
            col = 0;
        }
        let w = span as f32 * col_w + (span - 1) as f32 * panel.gap;
        let h = if item.height > 0.0 {
            item.height
        } else {
            default_height(&item.widget, panel.text_size, font, w)
        };
        rects[i] = Rect {
            x: panel.padding + col as f32 * (col_w + panel.gap),
            y: 0.0,
            w,
            h,
        };
        row.push(i);
        row_h = row_h.max(h);
        col += span;
    }
    finish_row(&mut row, &mut rects, &mut y, &mut row_h);
    let bottom = panel.size.y - panel.padding;
    for r in &rects {
        if r.y + r.h > bottom + 1e-4 {
            overflow = true;
        }
    }
    Layout { rects, overflow }
}

/// Splits `text` into lines of at most `max_w` meters at size `s` (words are not broken
/// unless one is wider than the line). `\n` starts a new line.
pub fn wrap(font: &FontData, text: &str, s: f32, max_w: f32) -> Vec<String> {
    let width = |t: &str| t.chars().map(|c| font.glyph(c).advance).sum::<f32>() * s;
    let mut lines = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        for word in para.split(' ') {
            let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if width(&candidate) <= max_w || line.is_empty() {
                line = candidate;
            } else {
                lines.push(std::mem::take(&mut line));
                line = word.to_string();
            }
        }
        lines.push(line);
    }
    lines
}

fn rgba(c: Color) -> [f32; 4] {
    [c.r, c.g, c.b, c.a]
}

struct Builder<'a> {
    font: &'a FontData,
    out: Vec<Quad>,
}

impl Builder<'_> {
    fn rect(&mut self, rect: Rect, color: [f32; 4], layer: u8) {
        if color[3] <= 0.0 || rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        let [u, v] = self.font.white_uv;
        self.out.push(Quad { rect, uv: [u, v, u, v], color, layer });
    }

    /// Draws one line of text; returns its width. `y` is the top of the line.
    fn line(&mut self, text: &str, s: f32, x: f32, y: f32, color: [f32; 4], layer: u8) -> f32 {
        let baseline = y + self.font.ascent * s;
        let mut pen = x;
        for c in text.chars() {
            let g = *self.font.glyph(c);
            if c != ' ' {
                self.out.push(Quad {
                    rect: Rect {
                        x: pen + g.rect[0] * s,
                        y: baseline - g.rect[3] * s,
                        w: (g.rect[2] - g.rect[0]) * s,
                        h: (g.rect[3] - g.rect[1]) * s,
                    },
                    uv: [g.uv[0], g.uv[1], g.uv[2], g.uv[3]],
                    color,
                    layer,
                });
            }
            pen += g.advance * s;
        }
        pen - x
    }

    fn width(&self, text: &str, s: f32) -> f32 {
        text.chars().map(|c| self.font.glyph(c).advance).sum::<f32>() * s
    }

    /// Text on one line, placed inside `r` by `align`, centred vertically.
    fn aligned(&mut self, text: &str, s: f32, r: Rect, align: UiAlign, color: [f32; 4], layer: u8) {
        let w = self.width(text, s);
        let x = match align {
            UiAlign::Left => r.x,
            UiAlign::Center => r.x + (r.w - w) / 2.0,
            UiAlign::Right => r.x + r.w - w,
        };
        let y = r.y + (r.h - self.font.line_height * s) / 2.0;
        self.line(text, s, x, y, color, layer);
    }
}

/// The quads of a panel, back to front.
pub fn mesh(panel: &UiPanel, content: &UiContent, layout: &Layout, font: &FontData, state: PanelState) -> Vec<Quad> {
    let mut b = Builder { font, out: Vec::new() };
    let full = Rect { x: 0.0, y: 0.0, w: panel.size.x, h: panel.size.y };
    b.rect(full, rgba(panel.background), 0);
    let s0 = panel.text_size;
    let (text, widget, hover, accent) = (rgba(panel.text_color), rgba(panel.widget), rgba(panel.hover), rgba(panel.accent));
    let bottom = panel.size.y - panel.padding;
    for (i, item) in content.items.iter().enumerate() {
        let r = layout.rects[i];
        if r.y + r.h > bottom + 1e-4 {
            continue;
        }
        let hot = state.hover == Some(i);
        let down = state.pressed == Some(i);
        let fill = if down { accent } else if hot { hover } else { widget };
        match &item.widget {
            UiWidget::Label { text: t, size, align } => {
                let s = if *size > 0.0 { *size } else { s0 };
                for (n, line) in wrap(font, t, s, r.w).iter().enumerate() {
                    let lr = Rect { x: r.x, y: r.y + n as f32 * font.line_height * s, w: r.w, h: font.line_height * s };
                    b.aligned(line, s, lr, *align, text, 2);
                }
            }
            UiWidget::Button { text: t, .. } => {
                b.rect(r, fill, 1);
                b.aligned(t, s0, r, UiAlign::Center, text, 2);
            }
            UiWidget::Toggle { text: t, on } => {
                b.aligned(t, s0, r, UiAlign::Left, text, 2);
                let track = Rect { x: r.x + r.w - 2.0 * s0, y: r.y + r.h / 2.0 - 0.5 * s0, w: 2.0 * s0, h: s0 };
                b.rect(track, if *on { accent } else if hot { hover } else { widget }, 1);
                let knob = Rect {
                    x: if *on { track.x + track.w - s0 + 0.12 * s0 } else { track.x + 0.12 * s0 },
                    y: track.y + 0.12 * s0,
                    w: 0.76 * s0,
                    h: 0.76 * s0,
                };
                b.rect(knob, text, 2);
            }
            UiWidget::Slider { text: t, min, max, value } => {
                let top = Rect { x: r.x, y: r.y, w: r.w, h: 1.2 * s0 };
                b.aligned(t, s0, top, UiAlign::Left, text, 2);
                b.aligned(&format!("{value:.2}"), s0, top, UiAlign::Right, text, 2);
                let (l, rt, cy) = slider_track(r, s0);
                let track = Rect { x: l, y: cy - 0.15 * s0, w: rt - l, h: 0.3 * s0 };
                b.rect(track, fill, 1);
                let f = if max > min { ((value - min) / (max - min)).clamp(0.0, 1.0) } else { 0.0 };
                b.rect(Rect { w: track.w * f, ..track }, accent, 2);
                let kx = l + (rt - l) * f;
                b.rect(Rect { x: kx - 0.4 * s0, y: cy - 0.4 * s0, w: 0.8 * s0, h: 0.8 * s0 }, text, 3);
            }
            UiWidget::Progress { text: t, value } => {
                b.rect(r, widget, 1);
                b.rect(Rect { w: r.w * value.clamp(0.0, 1.0), ..r }, accent, 2);
                b.aligned(t, s0, r, UiAlign::Center, text, 3);
            }
            UiWidget::List { rows, visible, scroll, selected } => {
                let rh = row_height(s0);
                let first = scroll.max(0.0) as usize;
                for n in 0..(*visible).max(1) as usize {
                    let Some(row) = rows.get(first + n) else { break };
                    let rr = Rect { x: r.x, y: r.y + n as f32 * rh, w: r.w, h: rh };
                    let idx = (first + n) as i32;
                    let color = if *selected == idx {
                        accent
                    } else if hot && state.hover_row == Some(first + n) {
                        hover
                    } else {
                        widget
                    };
                    b.rect(Rect { h: rh - 0.1 * s0, ..rr }, color, 1);
                    let tr = Rect { x: rr.x + 0.4 * s0, w: rr.w - 0.8 * s0, ..rr };
                    b.aligned(row, s0, tr, UiAlign::Left, text, 2);
                }
            }
            UiWidget::Spacer => {}
        }
    }
    b.out
}
