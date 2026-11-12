//! Provider tests with no network and no real keys: the mock is deterministic and loads in the
//! engine; config refuses keys; key, network and spend-cap checks come before any request; the
//! network plug-ins build the documented requests and parse recorded answer shapes through a
//! scripted transport; keys never appear in errors or debug output.

use marionette_core::Errors;
use marionette_providers::http::{HttpRequest, HttpResponse, Method, Transport};
use marionette_providers::{
    self as providers, CapSource, Choice, Env, ImageRequest, Kind, ModelRequest, NoNetwork, Output,
    ProviderCaps, ProvidersConfig, Request, SpendCap, TextRequest, claude, codes, qwen, spend, tripo,
};
use serde_json::json;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const FAKE_KEY: &str = "test-key-not-real-0123456789";

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("marionette-prov-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Answers requests from a script, in order, and records what was sent.
struct Scripted {
    answers: RefCell<Vec<HttpResponse>>,
    sent: RefCell<Vec<HttpRequest>>,
}

impl Scripted {
    fn new(answers: Vec<HttpResponse>) -> Self {
        Self {
            answers: RefCell::new(answers.into_iter().rev().collect()),
            sent: RefCell::new(Vec::new()),
        }
    }
    fn sent(&self) -> Vec<HttpRequest> {
        self.sent.borrow().clone()
    }
}

impl Transport for Scripted {
    fn send(&self, request: &HttpRequest) -> Result<HttpResponse, String> {
        self.sent.borrow_mut().push(request.clone());
        self.answers
            .borrow_mut()
            .pop()
            .ok_or_else(|| "script exhausted".to_string())
    }
}

fn vars(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
    let map: BTreeMap<String, String> = pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    move |k| map.get(k).cloned()
}

fn no_wait(_: u32) {}

fn config(kind: Kind, provider: &str, model: Option<&str>) -> ProvidersConfig {
    let mut c = ProvidersConfig::new();
    c.set(
        kind,
        Choice {
            provider: provider.into(),
            model: model.map(Into::into),
        },
    );
    c
}

fn cap(usd: f64) -> SpendCap {
    SpendCap {
        usd,
        source: CapSource::Environment,
    }
}

fn run(
    cfg: &ProvidersConfig,
    var: &dyn Fn(&str) -> Option<String>,
    transport: &dyn Transport,
    ledger: &Path,
    cap: SpendCap,
    request: &Request,
) -> Result<providers::Generation, Errors> {
    let env = Env {
        var,
        transport,
        ledger,
        cap,
        provider_caps: &ProviderCaps::default(),
        max_polls: 5,
        wait: &no_wait,
    };
    providers::generate(cfg, &env, request)
}

fn code(e: &Errors) -> &'static str {
    e.0[0].code
}

fn text(prompt: &str) -> Request {
    Request::Text(TextRequest {
        system: None,
        prompt: prompt.into(),
        max_tokens: 100,
    })
}

fn image(prompt: &str, w: u32, h: u32) -> Request {
    Request::Image(ImageRequest {
        prompt: prompt.into(),
        width: w,
        height: h,
    })
}

fn model(prompt: &str) -> Request {
    Request::Model(ModelRequest {
        prompt: Some(prompt.into()),
        image_url: None,
        ..Default::default()
    })
}

fn sha(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn mock_output(request: &Request, model_name: Option<&str>) -> Output {
    let dir = temp_dir("mock-ledger");
    let cfg = config(request.kind(), "mock", model_name);
    run(
        &cfg,
        &vars(&[]),
        &NoNetwork,
        &dir.join("l.jsonl"),
        SpendCap::ZERO,
        request,
    )
    .unwrap()
    .generated
    .output
}

#[test]
fn mock_is_deterministic_and_pinned_across_platforms() {
    let img = mock_output(&image("red brick wall", 64, 32), None);
    let glb = mock_output(&model("wooden crate"), None);
    let txt = mock_output(&text("write a jump function"), None);
    assert_eq!(img, mock_output(&image("red brick wall", 64, 32), None));
    assert_eq!(glb, mock_output(&model("wooden crate"), None));
    let (Output::Image(png), Output::Model(glb), Output::Text(txt)) = (img, glb, txt) else {
        panic!("wrong output kinds")
    };
    // Pinned so Windows and Linux CI prove byte-identical mock output. The PNG pin also
    // depends on the locked `png` crate's encoder; a dependency update may move it (say so).
    assert_eq!(
        sha(&png),
        "f9049f7ad383f5b97363c83ee7711822ea04b9aba608958ae44a323be995bbbf"
    );
    assert_eq!(
        sha(&glb),
        "658a8b15dcc62bf38e65bde6c267b411c0cf2c57abe5c9cb13cdd0a97f1308ad"
    );
    assert_eq!(txt, "[mock-text] reply e1f8aa3bdecd: write a jump function");
    // The model name changes the output.
    assert_ne!(
        Output::Image(png),
        mock_output(&image("red brick wall", 64, 32), Some("other"))
    );
}

#[test]
fn mock_outputs_load_in_the_engine() {
    let Output::Image(png) = mock_output(&image("grass", 40, 24), None) else {
        panic!()
    };
    let decoded = marionette_assets::decode_png(&png).unwrap();
    assert_eq!((decoded.width, decoded.height), (40, 24));

    let Output::Model(glb) = mock_output(&model("crate"), None) else {
        panic!()
    };
    let dir = temp_dir("glb");
    let path = dir.join("crate.glb");
    std::fs::write(&path, &glb).unwrap();
    let data = marionette_assets::load_gltf(&path).unwrap();
    assert_eq!(data.parts.len(), 1);
    let part = &data.parts[0];
    assert_eq!(data.materials[part.material].roughness, 0.8);
}

#[test]
fn config_defaults_to_mock_and_refuses_keys() {
    let dir = temp_dir("config");
    let (cfg, exists) = ProvidersConfig::load(&dir).unwrap();
    assert!(!exists);
    for kind in Kind::ALL {
        let (p, _) = providers::resolve(&cfg, kind).unwrap();
        assert_eq!(p.info().name, "mock");
    }
    let with_key = r#"{"format":"marionette.providers/1","language":{"provider":"claude","api_key":"x"}}"#;
    assert_eq!(
        code(&ProvidersConfig::parse(with_key).unwrap_err()),
        codes::BAD_CONFIG
    );
    let top_key = r#"{"format":"marionette.providers/1","anthropic_api_key":"x"}"#;
    assert_eq!(
        code(&ProvidersConfig::parse(top_key).unwrap_err()),
        codes::BAD_CONFIG
    );
    let cfg = config(Kind::Image, "qwen", Some("qwen-image"));
    assert_eq!(ProvidersConfig::parse(&cfg.to_text()).unwrap(), cfg);
    assert_eq!(
        code(
            &providers::resolve(&config(Kind::Language, "tripo", None), Kind::Language)
                .err()
                .unwrap()
        ),
        codes::KIND_UNSUPPORTED
    );
    assert_eq!(
        code(
            &providers::resolve(&config(Kind::Image, "nope", None), Kind::Image)
                .err()
                .unwrap()
        ),
        codes::UNKNOWN_PROVIDER
    );
}

#[test]
fn checks_run_in_order_before_any_request_is_sent() {
    let dir = temp_dir("order");
    let ledger = dir.join("spend.jsonl");
    let cfg = config(Kind::Language, "claude", None);
    let t = Scripted::new(vec![]);
    // No key.
    let e = run(&cfg, &vars(&[]), &t, &ledger, cap(10.0), &text("hi")).unwrap_err();
    assert_eq!(code(&e), codes::KEY_MISSING);
    // Key but no network.
    let key = vars(&[("ANTHROPIC_API_KEY", FAKE_KEY)]);
    let e = run(&cfg, &key, &NoNetwork, &ledger, cap(10.0), &text("hi")).unwrap_err();
    assert_eq!(code(&e), codes::NETWORK_DISABLED);
    // Key and network, but no cap.
    let e = run(&cfg, &key, &t, &ledger, SpendCap::ZERO, &text("hi")).unwrap_err();
    assert_eq!(code(&e), codes::SPEND_CAP);
    // Bad request is refused first of all.
    let e = run(&cfg, &key, &t, &ledger, cap(10.0), &text("  ")).unwrap_err();
    assert_eq!(code(&e), codes::BAD_REQUEST);
    assert!(
        t.sent().is_empty(),
        "nothing may be sent before every check passes"
    );
    assert!(!ledger.exists());
    for e in [
        run(&cfg, &vars(&[]), &t, &ledger, cap(10.0), &text("hi")).unwrap_err(),
        run(&cfg, &key, &NoNetwork, &ledger, cap(10.0), &text("hi")).unwrap_err(),
    ] {
        assert!(!format!("{e} {e:?}").contains(FAKE_KEY));
    }
}

fn claude_answer(text: &str, input: u64, output: u64) -> HttpResponse {
    HttpResponse::json(
        200,
        &json!({
            "id": "msg_1", "type": "message", "role": "assistant", "model": "claude-opus-5-5",
            "content": [{ "type": "text", "text": text }],
            "stop_reason": "end_turn",
            "usage": { "input_tokens": input, "output_tokens": output }
        }),
    )
}

#[test]
fn claude_request_shape_cost_and_ledger() {
    let dir = temp_dir("claude");
    let ledger = dir.join(".marionette/spend.jsonl");
    let cfg = config(Kind::Language, "claude", None);
    let key = vars(&[("ANTHROPIC_API_KEY", FAKE_KEY)]);
    let t = Scripted::new(vec![claude_answer("fn jump() {}", 1000, 2000)]);
    let request = Request::Text(TextRequest {
        system: Some("You write Rust.".into()),
        prompt: "write a jump function".into(),
        max_tokens: 4000,
    });
    let g = run(&cfg, &key, &t, &ledger, cap(1.0), &request).unwrap();
    assert_eq!(g.generated.output, Output::Text("fn jump() {}".into()));
    // 1000 * $4/M + 2000 * $20/M.
    assert!((g.generated.cost_usd - 0.044).abs() < 1e-12);
    assert!((spend::total(&ledger).unwrap() - 0.044).abs() < 1e-12);

    let sent = t.sent();
    assert_eq!(sent.len(), 1);
    let req = &sent[0];
    assert_eq!(req.method, Method::Post);
    assert_eq!(req.url, "https://api.anthropic.com/v1/messages");
    assert_eq!(req.header_value("x-api-key"), Some(FAKE_KEY));
    assert_eq!(req.header_value("anthropic-version"), Some("2026-10-21"));
    let body = req.body_json().unwrap();
    assert_eq!(body["model"], "claude-opus-5-5");
    assert_eq!(body["max_tokens"], 4000);
    assert_eq!(body["system"], "You write Rust.");
    assert_eq!(
        body["messages"],
        json!([{ "role": "user", "content": "write a jump function" }])
    );
    for field in ["temperature", "top_p", "top_k"] {
        assert!(body.get(field).is_none());
    }
    assert!(
        !format!("{req:?}").contains(FAKE_KEY),
        "debug output must redact keys"
    );

    // A second call whose estimate passes the cap is refused before sending.
    let t2 = Scripted::new(vec![claude_answer("x", 1, 1)]);
    let big = Request::Text(TextRequest {
        system: None,
        prompt: "x".into(),
        max_tokens: 100_000,
    });
    let e = run(&cfg, &key, &t2, &ledger, cap(1.0), &big).unwrap_err();
    assert_eq!(code(&e), codes::SPEND_CAP);
    assert!(t2.sent().is_empty());
}

#[test]
fn claude_refusal_and_http_errors_are_data_without_the_key() {
    let dir = temp_dir("claude-err");
    let ledger = dir.join("spend.jsonl");
    let cfg = config(Kind::Language, "claude", None);
    let key = vars(&[("ANTHROPIC_API_KEY", FAKE_KEY)]);
    let refusal = HttpResponse::json(
        200,
        &json!({ "content": [], "stop_reason": "refusal", "stop_details": { "type": "refusal", "category": "cyber" },
                 "usage": { "input_tokens": 5, "output_tokens": 0 } }),
    );
    let e = run(
        &cfg,
        &key,
        &Scripted::new(vec![refusal]),
        &ledger,
        cap(1.0),
        &text("hi"),
    )
    .unwrap_err();
    assert_eq!(code(&e), codes::REFUSED);
    let denied = HttpResponse::json(
        401,
        &json!({ "type": "error", "error": { "type": "authentication_error", "message": "invalid x-api-key" } }),
    );
    let e = run(
        &cfg,
        &key,
        &Scripted::new(vec![denied]),
        &ledger,
        cap(1.0),
        &text("hi"),
    )
    .unwrap_err();
    assert_eq!(code(&e), codes::HTTP);
    assert!(e.0[0].message.contains("401") && e.0[0].message.contains("invalid x-api-key"));
    assert!(!format!("{e} {e:?}").contains(FAKE_KEY));
    // Failed calls cost nothing in the ledger.
    assert_eq!(spend::total(&ledger).unwrap(), 0.0);
}

fn png_bytes() -> Vec<u8> {
    marionette_assets::encode_png(2, 2, &[255; 16])
}

#[test]
fn qwen_submits_polls_and_downloads() {
    let dir = temp_dir("qwen");
    let ledger = dir.join("spend.jsonl");
    let cfg = config(Kind::Image, "qwen", None);
    let key = vars(&[("DASHSCOPE_API_KEY", FAKE_KEY)]);
    let t = Scripted::new(vec![
        HttpResponse::json(
            200,
            &json!({ "output": { "task_id": "t-1", "task_status": "PENDING" } }),
        ),
        HttpResponse::json(
            200,
            &json!({ "output": { "task_id": "t-1", "task_status": "RUNNING" } }),
        ),
        HttpResponse::json(
            200,
            &json!({ "output": { "task_id": "t-1", "task_status": "SUCCEEDED",
                                  "results": [{ "url": "https://files.example/out.png" }] } }),
        ),
        HttpResponse {
            status: 200,
            body: png_bytes(),
        },
    ]);
    let g = run(
        &cfg,
        &key,
        &t,
        &ledger,
        cap(1.0),
        &image("stone texture", 1024, 768),
    )
    .unwrap();
    assert_eq!(g.generated.output, Output::Image(png_bytes()));
    assert_eq!(g.generated.cost_usd, qwen::PRICE_PER_IMAGE);
    let sent = t.sent();
    let urls: Vec<&str> = sent.iter().map(|r| r.url.as_str()).collect();
    assert_eq!(
        urls,
        [
            "https://dashscope-intl.aliyuncs.com/api/v1/services/aigc/text2image/image-synthesis",
            "https://dashscope-intl.aliyuncs.com/api/v1/tasks/t-1",
            "https://dashscope-intl.aliyuncs.com/api/v1/tasks/t-1",
            "https://files.example/out.png",
        ]
    );
    let body = sent[0].body_json().unwrap();
    assert_eq!(body["model"], "qwen-image");
    assert_eq!(body["input"]["prompt"], "stone texture");
    assert_eq!(body["parameters"]["size"], "1024*768");
    assert_eq!(sent[0].header_value("x-dashscope-async"), Some("enable"));
    assert_eq!(
        sent[0].header_value("authorization"),
        Some(format!("Bearer {FAKE_KEY}").as_str())
    );
    assert!(
        sent[3].header_value("authorization").is_none(),
        "downloads carry no key"
    );

    // Failure and timeout.
    let failed = Scripted::new(vec![
        HttpResponse::json(200, &json!({ "output": { "task_id": "t-2" } })),
        HttpResponse::json(
            200,
            &json!({ "output": { "task_status": "FAILED", "message": "bad prompt" } }),
        ),
    ]);
    let e = run(&cfg, &key, &failed, &ledger, cap(1.0), &image("x", 64, 64)).unwrap_err();
    assert_eq!(code(&e), codes::JOB_FAILED);
    let mut endless = vec![HttpResponse::json(
        200,
        &json!({ "output": { "task_id": "t-3" } }),
    )];
    endless
        .extend((0..5).map(|_| HttpResponse::json(200, &json!({ "output": { "task_status": "RUNNING" } }))));
    let e = run(
        &cfg,
        &key,
        &Scripted::new(endless),
        &ledger,
        cap(1.0),
        &image("x", 64, 64),
    )
    .unwrap_err();
    assert_eq!(code(&e), codes::JOB_TIMEOUT);
    // A non-PNG download is refused.
    let jpeg = Scripted::new(vec![
        HttpResponse::json(200, &json!({ "output": { "task_id": "t-4" } })),
        HttpResponse::json(
            200,
            &json!({ "output": { "task_status": "SUCCEEDED", "results": [{ "url": "u" }] } }),
        ),
        HttpResponse {
            status: 200,
            body: vec![0xff, 0xd8, 0xff],
        },
    ]);
    let e = run(&cfg, &key, &jpeg, &ledger, cap(1.0), &image("x", 64, 64)).unwrap_err();
    assert_eq!(code(&e), codes::BAD_RESPONSE);
    // Only the successful job is in the ledger.
    assert_eq!(spend::entries(&ledger).unwrap().len(), 1);
}

#[test]
fn tripo_text_and_image_jobs() {
    let dir = temp_dir("tripo");
    let ledger = dir.join("spend.jsonl");
    let key = vars(&[("TRIPO_API_KEY", FAKE_KEY)]);
    let Output::Model(glb) = mock_output(&model("anything"), None) else {
        panic!()
    };
    let script = |glb: &[u8]| {
        Scripted::new(vec![
            HttpResponse::json(200, &json!({ "code": 0, "data": { "task_id": "j-1" } })),
            HttpResponse::json(200, &json!({ "code": 0, "data": { "status": "queued" } })),
            HttpResponse::json(
                200,
                &json!({ "code": 0, "data": { "status": "success",
                                               "output": { "pbr_model": "https://files.example/m.glb" } } }),
            ),
            HttpResponse {
                status: 200,
                body: glb.to_vec(),
            },
        ])
    };
    let t = script(&glb);
    let g = run(
        &config(Kind::Model3d, "tripo", None),
        &key,
        &t,
        &ledger,
        cap(1.0),
        &model("a wooden crate"),
    )
    .unwrap();
    assert_eq!(g.generated.output, Output::Model(glb.clone()));
    let sent = t.sent();
    assert_eq!(sent[0].url, "https://api.tripo3d.ai/v2/openapi/task");
    assert_eq!(sent[1].url, "https://api.tripo3d.ai/v2/openapi/task/j-1");
    assert_eq!(
        sent[0].body_json().unwrap(),
        json!({ "type": "text_to_model", "prompt": "a wooden crate" })
    );

    let t = script(&glb);
    let req = Request::Model(ModelRequest {
        prompt: None,
        image_url: Some("https://files.example/ref.JPG?x=1".into()),
        ..Default::default()
    });
    run(
        &config(Kind::Model3d, "tripo", Some("v9")),
        &key,
        &t,
        &ledger,
        cap(1.0),
        &req,
    )
    .unwrap();
    assert_eq!(
        t.sent()[0].body_json().unwrap(),
        json!({ "type": "image_to_model", "file": { "type": "jpg", "url": "https://files.example/ref.JPG?x=1" },
                "model_version": "v9" })
    );
    // Two jobs at the assumed price; a third would pass a cap of 1.0.
    assert!((spend::total(&ledger).unwrap() - 2.0 * tripo::PRICE_PER_MODEL).abs() < 1e-12);
    let e = run(
        &config(Kind::Model3d, "tripo", None),
        &key,
        &script(&glb),
        &ledger,
        cap(1.0),
        &model("x"),
    )
    .unwrap_err();
    assert_eq!(code(&e), codes::SPEND_CAP);
}

#[test]
fn unreadable_ledger_refuses_paid_calls_but_not_free_ones() {
    let dir = temp_dir("ledger");
    let ledger = dir.join("spend.jsonl");
    std::fs::write(&ledger, "not json\n").unwrap();
    let key = vars(&[("ANTHROPIC_API_KEY", FAKE_KEY)]);
    let t = Scripted::new(vec![claude_answer("x", 1, 1)]);
    let e = run(
        &config(Kind::Language, "claude", None),
        &key,
        &t,
        &ledger,
        cap(5.0),
        &text("hi"),
    )
    .unwrap_err();
    assert_eq!(code(&e), codes::LEDGER);
    assert!(t.sent().is_empty());
    // Free calls do not depend on the ledger.
    run(
        &ProvidersConfig::new(),
        &vars(&[]),
        &NoNetwork,
        &ledger,
        SpendCap::ZERO,
        &text("hi"),
    )
    .unwrap();
}

#[test]
fn spend_cap_comes_from_the_environment_or_the_user_config_only() {
    assert_eq!(spend::resolve_cap(&vars(&[])).unwrap(), SpendCap::ZERO);
    let c = spend::resolve_cap(&vars(&[("MARIONETTE_SPEND_CAP_USD", "2.5")])).unwrap();
    assert_eq!((c.usd, c.source), (2.5, CapSource::Environment));
    assert_eq!(
        code(&spend::resolve_cap(&vars(&[("MARIONETTE_SPEND_CAP_USD", "-1")])).unwrap_err()),
        codes::SPEND_CAP
    );
    assert_eq!(
        code(&spend::resolve_cap(&vars(&[("MARIONETTE_SPEND_CAP_USD", "lots")])).unwrap_err()),
        codes::SPEND_CAP
    );
    let dir = temp_dir("cap");
    let cfg_dir = dir.join("marionette");
    std::fs::create_dir_all(&cfg_dir).unwrap();
    std::fs::write(cfg_dir.join("config.json"), r#"{"spend_cap_usd": 3.0}"#).unwrap();
    let d = dir.to_str().unwrap();
    let env = vars(&[("XDG_CONFIG_HOME", d), ("APPDATA", d)]);
    let c = spend::resolve_cap(&env).unwrap();
    assert_eq!((c.usd, c.source), (3.0, CapSource::UserConfig));
}

#[test]
fn every_offered_kind_has_a_default_model_and_keys_are_env_names() {
    for p in providers::all() {
        let info = p.info();
        for kind in info.kinds {
            assert!(info.default_model(*kind).is_some(), "{} {:?}", info.name, kind);
        }
        if let Some(var) = info.key_env {
            assert!(var.chars().all(|c| c.is_ascii_uppercase() || c == '_'), "{var}");
        }
        assert_eq!(info.network, info.key_env.is_some());
    }
    // Claude price table: unknown models are charged at the most expensive known model.
    let c = providers::find("claude").unwrap();
    let r = text("hi");
    assert!(c.estimate_usd("claude-unknown", &r) >= c.estimate_usd(claude::DEFAULT_MODEL, &r));
}

#[test]
fn a_provider_can_have_its_own_cap_or_none() {
    let dir = temp_dir("pcap");
    let cfg_dir = dir.join("marionette");
    std::fs::create_dir_all(&cfg_dir).unwrap();
    std::fs::write(
        cfg_dir.join("config.json"),
        r#"{"provider_caps_usd": {"tripo": 10, "qwen": "unlimited"}}"#,
    )
    .unwrap();
    let d = dir.to_str().unwrap();
    let env = vars(&[("XDG_CONFIG_HOME", d), ("APPDATA", d)]);
    let caps = spend::resolve_provider_caps(&env).unwrap();
    assert_eq!(caps.0["tripo"].limit, providers::Limit::Usd(10.0));
    assert_eq!(caps.0["qwen"].limit, providers::Limit::Unlimited);
    assert!(!caps.0.contains_key("claude"));
    // The environment wins over the file.
    let env2 = vars(&[
        ("XDG_CONFIG_HOME", d),
        ("APPDATA", d),
        ("MARIONETTE_SPEND_CAP_TRIPO_USD", "2"),
    ]);
    let caps2 = spend::resolve_provider_caps(&env2).unwrap();
    assert_eq!(caps2.0["tripo"].limit, providers::Limit::Usd(2.0));
    assert_eq!(caps2.0["tripo"].source, CapSource::Environment);
    let bad = vars(&[("MARIONETTE_SPEND_CAP_QWEN_USD", "lots")]);
    assert_eq!(
        code(&spend::resolve_provider_caps(&bad).unwrap_err()),
        codes::SPEND_CAP
    );

    // Checks: unlimited passes with a zero total cap; tripo counts only its own spending.
    let entry = |p: &str, c: f64| spend::Entry {
        provider: p.into(),
        model: "m".into(),
        kind: Kind::Image,
        cost_usd: c,
        estimate_usd: c,
    };
    let ledger = vec![entry("qwen", 50.0), entry("tripo", 9.5), entry("claude", 0.5)];
    assert!(spend::check_for(&SpendCap::ZERO, &caps, "qwen", &ledger, 0.0, 1.0).is_ok());
    assert!(spend::check_for(&SpendCap::ZERO, &caps, "tripo", &ledger, 0.0, 0.4).is_ok());
    assert!(spend::check_for(&SpendCap::ZERO, &caps, "tripo", &ledger, 0.2, 0.4).is_err());
    // Claude has no own cap: the total cap, over spending of uncapped providers only.
    assert!(spend::check_for(&cap(1.0), &caps, "claude", &ledger, 0.0, 0.5).is_ok());
    assert!(spend::check_for(&cap(1.0), &caps, "claude", &ledger, 0.0, 0.6).is_err());
}

#[test]
fn qwen_finds_the_keys_region_after_a_401() {
    let dir = temp_dir("qwen-region");
    let ledger = dir.join("spend.jsonl");
    let cfg = config(Kind::Image, "qwen", None);
    let key = vars(&[("DASHSCOPE_API_KEY", FAKE_KEY)]);
    let refused = HttpResponse::json(
        401,
        &json!({ "code": "InvalidApiKey", "message": "Invalid API-key provided." }),
    );
    let t = Scripted::new(vec![
        refused.clone(),
        HttpResponse::json(
            200,
            &json!({ "output": { "task_id": "t-9", "task_status": "PENDING" } }),
        ),
        HttpResponse::json(
            200,
            &json!({ "output": { "task_id": "t-9", "task_status": "SUCCEEDED",
                                  "results": [{ "url": "https://files.example/cn.png" }] } }),
        ),
        HttpResponse {
            status: 200,
            body: png_bytes(),
        },
    ]);
    run(&cfg, &key, &t, &ledger, cap(1.0), &image("x", 64, 64)).unwrap();
    let urls: Vec<String> = t.sent().iter().map(|r| r.url.clone()).collect();
    assert!(urls[0].starts_with("https://dashscope-intl.aliyuncs.com/"));
    assert!(urls[1].starts_with("https://dashscope.aliyuncs.com/api/v1/services/"));
    assert_eq!(
        urls[2], "https://dashscope.aliyuncs.com/api/v1/tasks/t-9",
        "polls the region that answered"
    );

    // A key no region accepts: the last region's 401 is the error, nothing charged.
    let t = Scripted::new(vec![refused.clone(), refused.clone(), refused]);
    let e = run(&cfg, &key, &t, &ledger, cap(1.0), &image("x", 64, 64)).unwrap_err();
    assert_eq!(code(&e), codes::HTTP);
    assert!(e.0[0].message.contains("401"));
    assert!(!format!("{e:?}").contains(FAKE_KEY));
    assert_eq!(t.sent().len(), 3);
    assert_eq!(
        spend::entries(&ledger).unwrap().len(),
        1,
        "only the successful call"
    );
}

#[test]
fn qwen_uses_a_personal_plan_address_with_the_synchronous_api() {
    let dir = temp_dir("qwen-plan");
    let ledger = dir.join("spend.jsonl");
    let cfg = config(Kind::Image, "qwen", Some("qwen-image-3.0-pro"));
    let key = vars(&[
        ("DASHSCOPE_API_KEY", FAKE_KEY),
        (
            "MARIONETTE_QWEN_ENDPOINT",
            "https://token-plan.example-region.maas.aliyuncs.com/compatible-mode/v1",
        ),
    ]);
    let t = Scripted::new(vec![
        HttpResponse::json(
            200,
            &json!({ "output": { "choices": [ { "message": { "role": "assistant",
                "content": [ { "image": "https://files.example/plan.png" } ] } } ] },
                "usage": { "image_count": 1 } }),
        ),
        HttpResponse {
            status: 200,
            body: png_bytes(),
        },
    ]);
    run(&cfg, &key, &t, &ledger, cap(1.0), &image("a crate", 64, 64)).unwrap();
    let sent = t.sent();
    assert_eq!(
        sent[0].url,
        "https://token-plan.example-region.maas.aliyuncs.com/api/v1/services/aigc/multimodal-generation/generation"
    );
    let body: serde_json::Value = serde_json::from_slice(sent[0].body.as_ref().unwrap()).unwrap();
    assert_eq!(body["model"], "qwen-image-3.0-pro");
    assert_eq!(body["input"]["messages"][0]["content"][0]["text"], "a crate");
    assert_eq!(body["parameters"]["size"], "64*64");
    assert_eq!(sent[1].url, "https://files.example/plan.png");
    assert_eq!(sent.len(), 2, "no region search and no polling on a plan address");

    // A refused key on the plan address is an error at once, nothing charged.
    let t = Scripted::new(vec![HttpResponse::json(
        401,
        &json!({ "code": "InvalidApiKey", "message": "Invalid API-key provided." }),
    )]);
    let e = run(&cfg, &key, &t, &ledger, cap(1.0), &image("x", 64, 64)).unwrap_err();
    assert_eq!(code(&e), codes::HTTP);
    assert!(!format!("{e:?}").contains(FAKE_KEY));
    assert_eq!(t.sent().len(), 1);
    assert_eq!(spend::entries(&ledger).unwrap().len(), 1);
}
