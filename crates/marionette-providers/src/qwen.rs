//! Qwen image generation through Alibaba Cloud Model Studio (DashScope), images only.
//!
//! Flow: `POST {BASE}/api/v1/services/aigc/text2image/image-synthesis` with
//! `Authorization: Bearer <DASHSCOPE_API_KEY>` and `X-DashScope-Async: enable`, body
//! `{model, input: {prompt}, parameters: {size: "W*H", n: 1, watermark: false,
//! prompt_extend: false}}`; answer `output.task_id`. Then
//! `GET {BASE}/api/v1/tasks/<id>` until `output.task_status` is `SUCCEEDED` (or `FAILED`,
//! `CANCELED`, `UNKNOWN`), and download `output.results[0].url`. The image must be PNG.
//!
//! The key's region is found by trying [`BASES`] in order (a 401 moves on).
//!
//! A person's own address (decision 0046, `endpoints`), such as a Model Studio plan address
//! `https://token-plan.<region>.maas.aliyuncs.com/compatible-mode/v1`, switches to the
//! synchronous image API on that host: `POST {host}/api/v1/services/aigc/multimodal-generation/generation`
//! with `{model, input: {messages: [{role: "user", content: [{text}]}]}, parameters: {size, n,
//! watermark, prompt_extend}}`; the image address is in `output.choices[0].message.content[*].image`.
//! No region search there.
//!
//! UNVERIFIED: endpoint, field names and the price are from documentation as remembered, not
//! checked against the live service (decision 0019).

use crate::http::HttpRequest;
use crate::{Ctx, Generated, Kind, Output, Provider, ProviderInfo, Request, net};
use marionette_core::Errors;
use serde_json::{Value, json};

pub struct Qwen;

pub const BASE: &str = "https://dashscope-intl.aliyuncs.com";
/// Model Studio regions, tried in this order. A key works in the region it was made in only
/// (international/Singapore, China/Beijing, US/Virginia); a 401 from one region moves on to the
/// next. A refused key is not charged.
pub const BASES: &[&str] = &[
    BASE,
    "https://dashscope.aliyuncs.com",
    "https://dashscope-us.aliyuncs.com",
];
pub const DEFAULT_MODEL: &str = "qwen-image";
/// Assumed upper price per image, US dollars (UNVERIFIED).
pub const PRICE_PER_IMAGE: f64 = 0.05;

static INFO: ProviderInfo = ProviderInfo {
    name: "qwen",
    title: "Qwen image (Alibaba Cloud Model Studio)",
    kinds: &[Kind::Image],
    key_env: Some("DASHSCOPE_API_KEY"),
    network: true,
    default_models: &[(Kind::Image, DEFAULT_MODEL)],
    verified: false,
    notes: "Images only. Asynchronous job: submit, poll, download. Priced per image.",
};

pub fn build_submit(model: &str, key: &str, prompt: &str, width: u32, height: u32) -> HttpRequest {
    build_submit_at(BASE, model, key, prompt, width, height)
}

pub fn build_submit_at(
    base: &str,
    model: &str,
    key: &str,
    prompt: &str,
    width: u32,
    height: u32,
) -> HttpRequest {
    let body = json!({
        "model": model,
        "input": { "prompt": prompt },
        // No watermark on game assets; no prompt rewriting, so a recipe's prompt is what runs.
        "parameters": {
            "size": format!("{width}*{height}"),
            "n": 1,
            "watermark": false,
            "prompt_extend": false,
        },
    });
    HttpRequest::post_json(
        format!("{base}/api/v1/services/aigc/text2image/image-synthesis"),
        &body,
    )
    .secret_header("authorization", format!("Bearer {key}"))
    .header("x-dashscope-async", "enable")
}

pub fn build_status(key: &str, task_id: &str) -> HttpRequest {
    build_status_at(BASE, key, task_id)
}

pub fn build_status_at(base: &str, key: &str, task_id: &str) -> HttpRequest {
    HttpRequest::get(format!("{base}/api/v1/tasks/{task_id}"))
        .secret_header("authorization", format!("Bearer {key}"))
}

/// `Some(url)` when the task succeeded, `None` while it runs, an error when it failed.
pub fn parse_status(v: &Value) -> Result<Option<String>, Errors> {
    let status = net::str_at(v, "/output/task_status", "qwen")?;
    match status {
        "SUCCEEDED" => Ok(Some(net::str_at(v, "/output/results/0/url", "qwen")?.to_string())),
        "PENDING" | "RUNNING" => Ok(None),
        other => Err(net::job_failed(
            "qwen",
            other,
            v.pointer("/output/message").and_then(Value::as_str),
        )),
    }
}

/// The host part of a personal address: `scheme://host`, path dropped.
pub fn host_of(url: &str) -> &str {
    let rest = url.find("://").map_or(0, |i| i + 3);
    match url[rest..].find('/') {
        Some(i) => &url[..rest + i],
        None => url.trim_end_matches('/'),
    }
}

/// The synchronous image call on a personal address (see the module docs).
pub fn build_sync_at(
    endpoint: &str,
    model: &str,
    key: &str,
    prompt: &str,
    width: u32,
    height: u32,
) -> HttpRequest {
    let body = json!({
        "model": model,
        "input": { "messages": [ { "role": "user", "content": [ { "text": prompt } ] } ] },
        "parameters": {
            "size": format!("{width}*{height}"),
            "n": 1,
            "watermark": false,
            "prompt_extend": false,
        },
    });
    HttpRequest::post_json(
        format!(
            "{}/api/v1/services/aigc/multimodal-generation/generation",
            host_of(endpoint)
        ),
        &body,
    )
    .secret_header("authorization", format!("Bearer {key}"))
}

/// The image address in a synchronous answer.
pub fn parse_sync(v: &Value) -> Result<String, Errors> {
    v.pointer("/output/choices/0/message/content")
        .and_then(Value::as_array)
        .and_then(|c| c.iter().find_map(|p| p.get("image").and_then(Value::as_str)))
        .map(str::to_string)
        .ok_or_else(|| {
            let msg = v.get("message").and_then(Value::as_str);
            net::job_failed(
                "qwen",
                "NO_IMAGE",
                msg.or(Some("the answer holds no image address")),
            )
        })
}

impl Provider for Qwen {
    fn info(&self) -> &'static ProviderInfo {
        &INFO
    }

    fn estimate_usd(&self, _: &str, request: &Request) -> f64 {
        match request {
            Request::Image(_) => PRICE_PER_IMAGE,
            _ => 0.0,
        }
    }

    fn run(&self, ctx: &Ctx, request: &Request) -> Result<Generated, Errors> {
        let Request::Image(i) = request else {
            unreachable!("generate checks the kind first")
        };
        let key = ctx.key.expect("generate checks the key first");
        if let Some(endpoint) = ctx.endpoint {
            let req = build_sync_at(endpoint, ctx.model, key, &i.prompt, i.width, i.height);
            let resp = net::send(ctx.transport, &req, "qwen")?;
            let url = parse_sync(&net::json(&resp, "qwen")?)?;
            let png = net::send(ctx.transport, &HttpRequest::get(url), "qwen")?.body;
            net::check_png(&png, "qwen")?;
            return Ok(Generated {
                output: Output::Image(png),
                cost_usd: PRICE_PER_IMAGE,
                usage: None,
            });
        }
        // Find the key's region: a 401 means "not this region's key", so try the next one.
        let mut found = None;
        for (n, base) in BASES.iter().enumerate() {
            let req = build_submit_at(base, ctx.model, key, &i.prompt, i.width, i.height);
            let resp = ctx.transport.send(&req);
            if matches!(&resp, Ok(r) if r.status == 401) && n + 1 < BASES.len() {
                continue;
            }
            found = Some((*base, net::send_result(resp, &req, "qwen")?));
            break;
        }
        let (base, submit) = found.expect("the last region returns or errors");
        let task_id = net::str_at(&net::json(&submit, "qwen")?, "/output/task_id", "qwen")?.to_string();
        let url = net::poll(ctx.max_polls, ctx.wait, "qwen", || {
            let resp = net::send(ctx.transport, &build_status_at(base, key, &task_id), "qwen")?;
            parse_status(&net::json(&resp, "qwen")?)
        })?;
        let png = net::send(ctx.transport, &HttpRequest::get(url), "qwen")?.body;
        net::check_png(&png, "qwen")?;
        Ok(Generated {
            output: Output::Image(png),
            cost_usd: PRICE_PER_IMAGE,
            usage: None,
        })
    }
}
