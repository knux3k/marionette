//! Tripo for 3D models (GLB), from a text prompt or a reference image URL.
//!
//! Flow: `POST {BASE}/v2/openapi/task` with `Authorization: Bearer <TRIPO_API_KEY>`, body
//! `{type: "text_to_model", prompt}` or `{type: "image_to_model", file: {type, url}}` (an image job
//! sends no prompt), plus
//! `model_version` unless the model is `default`; answer `data.task_id`. Then
//! `GET {BASE}/v2/openapi/task/<id>` until `data.status` is `success` (failures: `failed`,
//! `cancelled`, `banned`, `expired`, `unknown`), and download `data.output.pbr_model` (or
//! `data.output.model`). The file must be GLB. Rigging (Tripo auto-rig) belongs to workstream
//! W4.
//!
//! UNVERIFIED: endpoint, field names and the price are from documentation as remembered, not
//! checked against the live service (decision 0019).

use crate::http::HttpRequest;
use crate::{Ctx, Generated, Kind, ModelRequest, Output, Provider, ProviderInfo, Request, net};
use marionette_core::Errors;
use serde_json::{Value, json};

pub struct Tripo;

pub const BASE: &str = "https://api.tripo3d.ai";
pub const DEFAULT_MODEL: &str = "default";
/// Assumed upper price per model, US dollars (UNVERIFIED; Tripo bills in credits).
pub const PRICE_PER_MODEL: f64 = 0.40;

static INFO: ProviderInfo = ProviderInfo {
    name: "tripo",
    title: "Tripo (3D models)",
    kinds: &[Kind::Model3d],
    key_env: Some("TRIPO_API_KEY"),
    network: true,
    default_models: &[(Kind::Model3d, DEFAULT_MODEL)],
    verified: false,
    notes: "3D models as GLB from text or an image URL. Asynchronous job: submit, poll, download. Model `default` lets the service choose its version.",
};

fn image_type(url: &str) -> &'static str {
    let path = url.split(['?', '#']).next().unwrap_or("").to_ascii_lowercase();
    if path.ends_with(".jpg") || path.ends_with(".jpeg") {
        "jpg"
    } else if path.ends_with(".webp") {
        "webp"
    } else {
        "png"
    }
}

pub fn build_submit(model: &str, key: &str, request: &ModelRequest) -> HttpRequest {
    let mut body = match &request.image_url {
        Some(url) if !url.trim().is_empty() => json!({
            "type": "image_to_model",
            "file": { "type": image_type(url), "url": url },
        }),
        _ => json!({
            "type": "text_to_model",
            "prompt": request.prompt.as_deref().unwrap_or(""),
        }),
    };
    if model != DEFAULT_MODEL {
        body["model_version"] = Value::String(model.to_string());
    }
    HttpRequest::post_json(format!("{BASE}/v2/openapi/task"), &body)
        .secret_header("authorization", format!("Bearer {key}"))
}

pub fn build_status(key: &str, task_id: &str) -> HttpRequest {
    HttpRequest::get(format!("{BASE}/v2/openapi/task/{task_id}"))
        .secret_header("authorization", format!("Bearer {key}"))
}

/// `Some(url)` when the job succeeded, `None` while it runs, an error when it failed.
pub fn parse_status(v: &Value) -> Result<Option<String>, Errors> {
    let status = net::str_at(v, "/data/status", "tripo")?;
    match status {
        "success" => {
            let url = ["/data/output/pbr_model", "/data/output/model"]
                .iter()
                .find_map(|p| v.pointer(p).and_then(Value::as_str))
                .ok_or_else(|| net::bad_response("tripo", "finished job has no model URL"))?;
            Ok(Some(url.to_string()))
        }
        "queued" | "running" => Ok(None),
        other => Err(net::job_failed("tripo", other, None)),
    }
}

impl Provider for Tripo {
    fn info(&self) -> &'static ProviderInfo {
        &INFO
    }

    fn estimate_usd(&self, _: &str, request: &Request) -> f64 {
        match request {
            Request::Model(_) => PRICE_PER_MODEL,
            _ => 0.0,
        }
    }

    fn run(&self, ctx: &Ctx, request: &Request) -> Result<Generated, Errors> {
        let Request::Model(m) = request else {
            unreachable!("generate checks the kind first")
        };
        let key = ctx.key.expect("generate checks the key first");
        let submit = net::send(ctx.transport, &build_submit(ctx.model, key, m), "tripo")?;
        let task_id = net::str_at(&net::json(&submit, "tripo")?, "/data/task_id", "tripo")?.to_string();
        let url = net::poll(ctx.max_polls, ctx.wait, "tripo", || {
            let resp = net::send(ctx.transport, &build_status(key, &task_id), "tripo")?;
            parse_status(&net::json(&resp, "tripo")?)
        })?;
        let glb = net::send(ctx.transport, &HttpRequest::get(url), "tripo")?.body;
        net::check_glb(&glb, "tripo")?;
        Ok(Generated {
            output: Output::Model(glb),
            cost_usd: PRICE_PER_MODEL,
            usage: None,
        })
    }
}
