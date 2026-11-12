//! Claude (Anthropic Messages API) for language: text and code.
//!
//! Request: `POST https://api.anthropic.com/v1/messages` with `x-api-key` (from
//! `ANTHROPIC_API_KEY`), `anthropic-version: 2026-10-21` and a JSON body of `model`,
//! `max_tokens`, optional `system` and one user message. No sampling fields (`temperature`
//! and friends are rejected by that API version). The answer's text blocks are joined;
//! `stop_reason: "refusal"` becomes `provider.refused`; cost comes from the reported usage.
//!
//! UNVERIFIED: built from the API documentation and tested against recorded shapes only; no
//! live call has been made (decision 0019).

use crate::http::HttpRequest;
use crate::{Ctx, Generated, Kind, Output, Provider, ProviderInfo, Request, Usage, codes, net};
use marionette_core::{Diagnostic, Errors};
use serde_json::{Value, json};

pub struct Claude;

pub const URL: &str = "https://api.anthropic.com/v1/messages";
pub const API_VERSION: &str = "2026-10-21";
pub const DEFAULT_MODEL: &str = "claude-opus-5-5";

static INFO: ProviderInfo = ProviderInfo {
    name: "claude",
    title: "Claude (Anthropic Messages API)",
    kinds: &[Kind::Language],
    key_env: Some("ANTHROPIC_API_KEY"),
    network: true,
    default_models: &[(Kind::Language, DEFAULT_MODEL)],
    verified: false,
    notes: "Text and code. Priced per token from the reported usage.",
};

/// US dollars per million (input, output) tokens. Unknown models use the most expensive row.
const PRICES: &[(&str, f64, f64)] = &[
    ("claude-fable-5-2", 8.0, 40.0),
    ("claude-opus-5-5", 4.0, 20.0),
    ("claude-sonnet-5-5", 2.0, 10.0),
    ("claude-haiku-5-5", 0.10, 0.50),
];

fn price(model: &str) -> (f64, f64) {
    PRICES
        .iter()
        .find(|(m, _, _)| *m == model)
        .map(|(_, i, o)| (*i, *o))
        .unwrap_or_else(|| {
            PRICES.iter().fold((0.0, 0.0), |acc, (_, i, o)| {
                (f64::max(acc.0, *i), f64::max(acc.1, *o))
            })
        })
}

fn cost(model: &str, input_tokens: u64, output_tokens: u64) -> f64 {
    let (i, o) = price(model);
    (input_tokens as f64 * i + output_tokens as f64 * o) / 1_000_000.0
}

/// Builds the HTTP request (exposed for tests and documentation).
pub fn build(model: &str, key: &str, system: Option<&str>, prompt: &str, max_tokens: u32) -> HttpRequest {
    let mut body = json!({
        "model": model,
        "max_tokens": max_tokens,
        "messages": [{ "role": "user", "content": prompt }],
    });
    if let Some(s) = system {
        body["system"] = Value::String(s.to_string());
    }
    HttpRequest::post_json(URL, &body)
        .secret_header("x-api-key", key)
        .header("anthropic-version", API_VERSION)
}

/// Reads text and usage from a Messages API answer.
pub fn parse(v: &Value) -> Result<(String, Usage), Errors> {
    if v.get("stop_reason").and_then(Value::as_str) == Some("refusal") {
        let detail = v
            .pointer("/stop_details/category")
            .and_then(Value::as_str)
            .map(|c| format!(" (category {c})"))
            .unwrap_or_default();
        return Err(
            Diagnostic::error(codes::REFUSED, format!("claude declined the request{detail}"))
                .fix("rephrase the request")
                .into(),
        );
    }
    let content = v
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| net::bad_response("claude", "answer has no content array"))?;
    let text: String = content
        .iter()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|b| b.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("");
    let tokens = |name: &str| v.pointer(&format!("/usage/{name}")).and_then(Value::as_u64);
    let usage = Usage {
        input_tokens: tokens("input_tokens")
            .ok_or_else(|| net::bad_response("claude", "answer has no usage"))?,
        output_tokens: tokens("output_tokens")
            .ok_or_else(|| net::bad_response("claude", "answer has no usage"))?,
    };
    Ok((text, usage))
}

impl Provider for Claude {
    fn info(&self) -> &'static ProviderInfo {
        &INFO
    }

    fn estimate_usd(&self, model: &str, request: &Request) -> f64 {
        match request {
            // Upper bound: one token per input byte, the full max_tokens in output.
            Request::Text(t) => {
                let input = t.prompt.len() + t.system.as_deref().map_or(0, str::len);
                cost(model, input as u64, t.max_tokens as u64)
            }
            _ => 0.0,
        }
    }

    fn run(&self, ctx: &Ctx, request: &Request) -> Result<Generated, Errors> {
        let Request::Text(t) = request else {
            unreachable!("generate checks the kind first")
        };
        let key = ctx.key.expect("generate checks the key first");
        let req = build(ctx.model, key, t.system.as_deref(), &t.prompt, t.max_tokens);
        let resp = net::send(ctx.transport, &req, "claude")?;
        let (text, usage) = parse(&net::json(&resp, "claude")?)?;
        Ok(Generated {
            output: Output::Text(text),
            cost_usd: cost(ctx.model, usage.input_tokens, usage.output_tokens),
            usage: Some(usage),
        })
    }
}
