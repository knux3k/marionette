//! Qwen image generation through Alibaba Cloud Model Studio (DashScope), images only.
//!
//! Flow: `POST {BASE}/api/v1/services/aigc/text2image/image-synthesis` with
//! `Authorization: Bearer <DASHSCOPE_API_KEY>` and `X-DashScope-Async: enable`, body
//! `{model, input: {prompt}, parameters: {size: "W*H", n: 1, watermark: false,
//! prompt_extend: false}}`; answer `output.task_id`. Then
//! `GET {BASE}/api/v1/tasks/<id>` until `output.task_status` is `SUCCEEDED` (or `FAILED`,
//! `CANCELED`, `UNKNOWN`), and download `output.results[0].url`. The image must be PNG.
//!
//! UNVERIFIED: endpoint, field names and the price are from documentation as remembered, not
//! checked against the live service (decision 0019).

use crate::http::HttpRequest;
use crate::{Ctx, Generated, Kind, Output, Provider, ProviderInfo, Request, net};
use marionette_core::Errors;
use serde_json::{Value, json};

pub struct Qwen;

pub const BASE: &str = "https://dashscope-intl.aliyuncs.com";
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
        format!("{BASE}/api/v1/services/aigc/text2image/image-synthesis"),
        &body,
    )
    .secret_header("authorization", format!("Bearer {key}"))
    .header("x-dashscope-async", "enable")
}

pub fn build_status(key: &str, task_id: &str) -> HttpRequest {
    HttpRequest::get(format!("{BASE}/api/v1/tasks/{task_id}"))
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
        let submit = net::send(
            ctx.transport,
            &build_submit(ctx.model, key, &i.prompt, i.width, i.height),
            "qwen",
        )?;
        let task_id = net::str_at(&net::json(&submit, "qwen")?, "/output/task_id", "qwen")?.to_string();
        let url = net::poll(ctx.max_polls, ctx.wait, "qwen", || {
            let resp = net::send(ctx.transport, &build_status(key, &task_id), "qwen")?;
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
