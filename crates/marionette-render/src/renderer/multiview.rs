//! Single-pass stereo (decision 0285): both eyes drawn by one render pass into a two-layer
//! target with multiview, so the CPU records every draw once instead of once per eye.
//! Shaders get a multiview form made from their normal source: the view uniform becomes an
//! array of the two eyes' views, picked with `@builtin(view_index)` at the start of every
//! entry point, so the shader bodies stay the same.

use std::num::NonZeroU32;
use std::sync::OnceLock;

/// Both layers of the target.
pub(crate) const MASK: Option<NonZeroU32> = NonZeroU32::new(0b11);

/// The multiview mask of a pipeline into `target` (a `variants` target).
pub(crate) fn mask(target: usize) -> Option<NonZeroU32> {
    if target == super::variants::STEREO {
        MASK
    } else {
        None
    }
}

/// The multiview form of a shader, compiled the first time a stereo pipeline needs it.
pub(crate) struct LazyModule {
    source: String,
    label: &'static str,
    module: OnceLock<wgpu::ShaderModule>,
}

impl LazyModule {
    pub fn new(source: &str, label: &'static str) -> Self {
        Self {
            source: self::source(source),
            label,
            module: OnceLock::new(),
        }
    }

    pub fn get(&self, device: &wgpu::Device) -> &wgpu::ShaderModule {
        self.module.get_or_init(|| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(self.label),
                source: wgpu::ShaderSource::Wgsl(self.source.as_str().into()),
            })
        })
    }
}

const VIEW_DECL: &str = "@group(0) @binding(0) var<uniform> view: ViewU;";

/// The multiview form of a WGSL source that declares [`VIEW_DECL`]; other sources are
/// returned unchanged.
pub(crate) fn source(src: &str) -> String {
    if !src.contains(VIEW_DECL) {
        return src.to_string();
    }
    let src = src.replacen(
        VIEW_DECL,
        "@group(0) @binding(0) var<uniform> views: array<ViewU, 2>;\nvar<private> view: ViewU;",
        1,
    );
    let mut out = String::with_capacity(src.len() + 1024);
    let mut rest = src.as_str();
    while let Some(at) = next_entry(rest) {
        let (head, tail) = rest.split_at(at);
        out.push_str(head);
        // `tail` starts at `fn name(`.
        let open = tail.find('(').unwrap_or(0);
        out.push_str(&tail[..=open]);
        let after = &tail[open + 1..];
        out.push_str("@builtin(view_index) mv_view_index: u32");
        if !after.trim_start().starts_with(')') {
            out.push_str(", ");
        }
        // The parameter list ends at the matching parenthesis; the body at the next brace.
        let mut depth = 1usize;
        let mut close = after.len();
        for (i, c) in after.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        close = i;
                        break;
                    }
                }
                _ => {}
            }
        }
        let brace = after[close..].find('{').map_or(after.len(), |b| close + b);
        out.push_str(&after[..=brace.min(after.len() - 1)]);
        out.push_str("\n    view = views[mv_view_index];");
        rest = &after[(brace + 1).min(after.len())..];
    }
    out.push_str(rest);
    out
}

/// Byte offset of the next `fn` that follows a `@vertex` or `@fragment` attribute.
fn next_entry(s: &str) -> Option<usize> {
    let mut from = 0;
    loop {
        let v = s[from..].find("@vertex");
        let f = s[from..].find("@fragment");
        let at = match (v, f) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) | (None, Some(a)) => a,
            (None, None) => return None,
        } + from;
        let fn_at = s[at..].find("fn ")? + at;
        // Only whitespace and other attributes between the stage and `fn`.
        if s[at..fn_at].lines().count() <= 2 {
            return Some(fn_at);
        }
        from = at + 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_points_pick_their_eye() {
        let src = "struct ViewU { m: mat4x4<f32> }\n\
                   @group(0) @binding(0) var<uniform> view: ViewU;\n\
                   @vertex\nfn vs(@location(0) p: vec3<f32>) -> @builtin(position) vec4<f32> {\n  return view.m * vec4(p, 1.0);\n}\n\
                   @fragment fn fs() -> @location(0) vec4<f32> { return vec4(f(vec2(1.0, 2.0))); }\n\
                   fn f(v: vec2<f32>) -> f32 { return v.x; }\n";
        let out = source(src);
        assert!(out.contains("var<uniform> views: array<ViewU, 2>;"));
        assert!(out.contains("fn vs(@builtin(view_index) mv_view_index: u32, @location(0) p: vec3<f32>)"));
        assert!(out.contains("fn fs(@builtin(view_index) mv_view_index: u32)"));
        assert_eq!(out.matches("view = views[mv_view_index];").count(), 2);
        // Helpers stay as they are.
        assert!(out.contains("fn f(v: vec2<f32>) -> f32"));
        assert_eq!(source("fn g() {}"), "fn g() {}");
    }
}
