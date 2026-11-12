//! The mock provider: offline, free and deterministic. The output depends only on the model
//! name and the request, byte for byte on every platform (integer arithmetic only), so tests
//! and examples can use generated content without a network or a key.
//!
//! - Language: a short reply naming the model and a digest of the prompt, then the prompt's
//!   first line.
//! - Image: a two-color checkerboard PNG at the requested size; colors and cell size come
//!   from the digest.
//! - 3D: a GLB box; proportions and color come from the digest.

use crate::{Ctx, Generated, Kind, Output, Provider, ProviderInfo, Request, Usage, glb, sha256, sha256_hex};
use marionette_core::Errors;

pub struct Mock;

static INFO: ProviderInfo = ProviderInfo {
    name: "mock",
    title: "Mock (offline, deterministic)",
    kinds: &[Kind::Language, Kind::Image, Kind::Model3d],
    key_env: None,
    network: false,
    default_models: &[
        (Kind::Language, "mock-text"),
        (Kind::Image, "mock-image"),
        (Kind::Model3d, "mock-model"),
    ],
    verified: true,
    notes: "Default for every kind. Free, needs no key or network; any model name is accepted and changes the output.",
};

/// The digest that drives every mock output.
fn digest(model: &str, kind: Kind, text: &str) -> [u8; 32] {
    sha256(format!("{model}\n{}\n{text}", kind.as_str()).as_bytes())
}

impl Provider for Mock {
    fn info(&self) -> &'static ProviderInfo {
        &INFO
    }

    fn estimate_usd(&self, _: &str, _: &Request) -> f64 {
        0.0
    }

    fn run(&self, ctx: &Ctx, request: &Request) -> Result<Generated, Errors> {
        let model = ctx.model;
        Ok(match request {
            Request::Text(t) => {
                let input = format!("{}\n{}", t.system.as_deref().unwrap_or(""), t.prompt);
                let tag = &sha256_hex(input.as_bytes())[..12];
                let first = t.prompt.lines().next().unwrap_or("").trim();
                let first: String = first.chars().take(120).collect();
                let reply = format!("[{model}] reply {tag}: {first}");
                // Rough counts: words, so they stay integers and platform independent.
                let usage = Usage {
                    input_tokens: input.split_whitespace().count() as u64,
                    output_tokens: reply.split_whitespace().count() as u64,
                };
                Generated {
                    output: Output::Text(reply),
                    cost_usd: 0.0,
                    usage: Some(usage),
                }
            }
            Request::Image(i) => {
                let d = digest(model, Kind::Image, &i.prompt);
                let a = [d[0], d[1], d[2], 255];
                let b = [d[3], d[4], d[5], 255];
                let cell = 4 + (d[6] as u32 % 29);
                let mut rgba = Vec::with_capacity((i.width * i.height * 4) as usize);
                for y in 0..i.height {
                    for x in 0..i.width {
                        let c = if ((x / cell) + (y / cell)).is_multiple_of(2) {
                            a
                        } else {
                            b
                        };
                        rgba.extend_from_slice(&c);
                    }
                }
                Generated {
                    output: Output::Image(marionette_assets::encode_png(i.width, i.height, &rgba)),
                    cost_usd: 0.0,
                    usage: None,
                }
            }
            Request::Model(m) => {
                let text = format!(
                    "{}\n{}",
                    m.prompt.as_deref().unwrap_or(""),
                    m.image_url.as_deref().unwrap_or("")
                );
                let d = digest(model, Kind::Model3d, &text);
                // Half extents 0.25 to 1.0 m in 1/256 steps; color channels from bytes.
                let half = [0, 1, 2].map(|i| 0.25 + (d[i] as f32) * (0.75 / 255.0));
                let color = [d[3] as f32 / 255.0, d[4] as f32 / 255.0, d[5] as f32 / 255.0, 1.0];
                let name = format!("mock-{}", &sha256_hex(text.as_bytes())[..8]);
                Generated {
                    output: Output::Model(glb::box_glb(half, color, &name)),
                    cost_usd: 0.0,
                    usage: None,
                }
            }
        })
    }
}
