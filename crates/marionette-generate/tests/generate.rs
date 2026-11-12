//! Generation tests with no network and no real keys. Paid providers are driven through a
//! scripted transport answering with the sample answers in `tests/recorded/`; the PNG and GLB
//! bodies come from the mock provider. Proves: recipes rebuild byte-identically from the cache
//! and from scratch; the same request is never paid twice; a zero cap, a missing approval or
//! a run over budget refuses before anything is sent; post-processing gives engine-ready files.

use marionette_core::Errors;
use marionette_generate::recipe::{ModelOptions, OutputKind, Recipe, TextureOptions};
use marionette_generate::{self as generate, Env, StepStatus, Target, approval, codes, provenance};
use marionette_providers::http::{HttpRequest, HttpResponse, Transport};
use marionette_providers::{
    self as providers, CapSource, Choice, ImageRequest, ModelRequest, NoNetwork, Output, ProvidersConfig,
    Request, SpendCap, spend,
};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const FAKE_KEY: &str = "test-key-not-real-0123456789";
const DIR: &str = "assets/generated";

fn temp_project(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("marionette-gen-{tag}-{}", std::process::id()));
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
    fn left(&self) -> usize {
        self.answers.borrow().len()
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

fn recorded(name: &str) -> HttpResponse {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/recorded")
        .join(name);
    HttpResponse {
        status: 200,
        body: std::fs::read(path).unwrap(),
    }
}

fn bytes(body: Vec<u8>) -> HttpResponse {
    HttpResponse { status: 200, body }
}

fn mock_bytes(request: Request) -> Vec<u8> {
    let cfg = ProvidersConfig::new();
    let dir = temp_project("mockbytes");
    let env = providers::Env {
        var: &|_| None,
        transport: &NoNetwork,
        ledger: &dir.join("l.jsonl"),
        cap: SpendCap::ZERO,
        max_polls: 1,
        wait: &|_| {},
    };
    match providers::generate(&cfg, &env, &request)
        .unwrap()
        .generated
        .output
    {
        Output::Image(b) | Output::Model(b) => b,
        Output::Text(_) => unreachable!(),
    }
}

fn mock_png(w: u32, h: u32) -> Vec<u8> {
    mock_bytes(Request::Image(ImageRequest {
        prompt: "sample download".into(),
        width: w,
        height: h,
    }))
}

fn mock_glb() -> Vec<u8> {
    mock_bytes(Request::Model(ModelRequest {
        prompt: Some("sample download".into()),
        ..Default::default()
    }))
}

fn cap(usd: f64) -> SpendCap {
    SpendCap {
        usd,
        source: CapSource::Environment,
    }
}

fn vars(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
    let map: BTreeMap<String, String> = pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    move |k| map.get(k).cloned()
}

fn keys() -> impl Fn(&str) -> Option<String> {
    vars(&[("DASHSCOPE_API_KEY", FAKE_KEY), ("TRIPO_API_KEY", FAKE_KEY)])
}

fn env<'a>(var: &'a dyn Fn(&str) -> Option<String>, t: &'a dyn Transport, cap: SpendCap) -> Env<'a> {
    Env {
        var,
        transport: t,
        cap,
        max_polls: 5,
        wait: &|_| {},
    }
}

fn recipe(name: &str, output: OutputKind) -> Recipe {
    Recipe {
        format: generate::recipe::FORMAT.into(),
        name: name.into(),
        output,
        prompt: None,
        size: None,
        texture: None,
        from: None,
        model: None,
        provider: None,
        notes: None,
    }
}

fn image_recipe(name: &str, prompt: &str, size: [u32; 2]) -> Recipe {
    Recipe {
        prompt: Some(prompt.into()),
        size: Some(size),
        ..recipe(name, OutputKind::Image)
    }
}

fn pin(provider: &str) -> Option<Choice> {
    Some(Choice {
        provider: provider.into(),
        model: None,
    })
}

fn target<'a>(root: &'a Path, names: &'a [String]) -> Target<'a> {
    Target {
        root,
        dir: DIR,
        names,
    }
}

fn write(root: &Path, r: &Recipe) {
    generate::write_recipe(&target(root, &[]), r, true).unwrap();
}

fn read(root: &Path, file: &str) -> Vec<u8> {
    std::fs::read(root.join(DIR).join(file)).unwrap()
}

fn code(e: &Errors) -> &'static str {
    e.0[0].code
}

/// The three acceptance assets: a reference image, a tileable texture, a model from the image.
fn three_recipes(root: &Path) {
    write(
        root,
        &image_recipe("crate_ref", "a wooden crate, plain background", [256, 256]),
    );
    write(
        root,
        &Recipe {
            prompt: Some("red brick wall".into()),
            size: Some([300, 200]),
            texture: Some(TextureOptions {
                resize: None,
                tileable: true,
            }),
            ..recipe("bricks", OutputKind::Texture)
        },
    );
    write(
        root,
        &Recipe {
            from: Some("crate_ref".into()),
            model: Some(ModelOptions {
                height_m: Some(1.0),
                max_triangles: Some(10),
            }),
            ..recipe("crate", OutputKind::Model)
        },
    );
}

#[test]
fn three_assets_rebuild_identically_from_recipes() {
    let root = temp_project("three");
    three_recipes(&root);
    let none = vars(&[]);
    let e = env(&none, &NoNetwork, SpendCap::ZERO);
    let first = generate::run(&target(&root, &[]), &e, false).unwrap();
    assert_eq!(first.calls, 3);
    assert!(first.blocked.is_empty(), "{:?}", first.blocked);
    let names: Vec<&str> = first.built.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(
        names,
        ["bricks", "crate_ref", "crate"],
        "images first, then models"
    );
    let files = [
        "bricks.png",
        "bricks.provenance.json",
        "crate_ref.png",
        "crate_ref.provenance.json",
        "crate.glb",
        "crate.provenance.json",
    ];
    let snapshot: Vec<Vec<u8>> = files.iter().map(|f| read(&root, f)).collect();

    // Every asset has its license and source recorded, and is blocked from the pool.
    for name in ["bricks", "crate_ref", "crate"] {
        let p = provenance::read(&root.join(DIR), name).unwrap();
        assert_eq!(p.license.status, "generated");
        assert_eq!(p.pool, "blocked");
        assert_eq!(p.source.provider, "mock");
        assert_eq!(p.recipe, format!("{name}.recipe.json"));
    }
    let crate_rec = provenance::read(&root.join(DIR), "crate").unwrap();
    let from = crate_rec.source.from.unwrap();
    assert_eq!(from.file, "crate_ref.png");
    assert_eq!(from.sha256, generate::sha256_hex(&read(&root, "crate_ref.png")));
    assert!(!from.approved, "mock is free, so no approval was needed");
    assert!(
        crate_rec.postprocess["notes"][0]
            .as_str()
            .unwrap()
            .contains("over the budget of 10"),
        "triangle budget is reported"
    );

    // A second run does nothing.
    let again = generate::run(&target(&root, &[]), &e, false).unwrap();
    assert_eq!(again.calls, 0);
    assert!(again.built.iter().all(|b| b.how == "up_to_date"));

    // Outputs deleted: rebuilt offline from the cache, byte for byte.
    for f in &files {
        std::fs::remove_file(root.join(DIR).join(f)).unwrap();
    }
    let offline = generate::run(&target(&root, &[]), &e, true).unwrap();
    assert_eq!(offline.calls, 0);
    assert!(offline.built.iter().all(|b| b.how == "cached"));
    assert_eq!(files.iter().map(|f| read(&root, f)).collect::<Vec<_>>(), snapshot);

    // Cache and outputs deleted: the mock regenerates the same bytes.
    std::fs::remove_dir_all(root.join(".marionette")).unwrap();
    for f in &files {
        std::fs::remove_file(root.join(DIR).join(f)).unwrap();
    }
    let fresh = generate::run(&target(&root, &[]), &e, false).unwrap();
    assert_eq!(fresh.calls, 3);
    assert_eq!(files.iter().map(|f| read(&root, f)).collect::<Vec<_>>(), snapshot);

    // Offline with an empty cache refuses instead of calling.
    std::fs::remove_dir_all(root.join(".marionette")).unwrap();
    std::fs::remove_file(root.join(DIR).join("bricks.png")).unwrap();
    let err = generate::run(&target(&root, &["bricks".into()]), &e, true).unwrap_err();
    assert_eq!(code(&err), codes::NOT_CACHED);
}

#[test]
fn engine_formats_textures_are_powers_of_two_and_models_stand_on_the_ground() {
    let root = temp_project("formats");
    three_recipes(&root);
    let none = vars(&[]);
    generate::run(
        &target(&root, &[]),
        &env(&none, &NoNetwork, SpendCap::ZERO),
        false,
    )
    .unwrap();
    let tex = marionette_assets::decode_png(&read(&root, "bricks.png")).unwrap();
    assert_eq!((tex.width, tex.height), (256, 128));
    let model = marionette_assets::load_gltf(&root.join(DIR).join("crate.glb")).unwrap();
    let mut lo = [f32::MAX; 3];
    let mut hi = [f32::MIN; 3];
    for part in &model.parts {
        for p in &part.mesh.positions {
            let w = part.transform.transform_point3((*p).into());
            for i in 0..3 {
                lo[i] = lo[i].min(w[i]);
                hi[i] = hi[i].max(w[i]);
            }
        }
    }
    assert!(lo[1].abs() < 1e-5, "grounded: lowest point {}", lo[1]);
    assert!((hi[1] - 1.0).abs() < 1e-5, "scaled to 1 m: top {}", hi[1]);
    assert!(
        (lo[0] + hi[0]).abs() < 1e-5 && (lo[2] + hi[2]).abs() < 1e-5,
        "centered"
    );
}

#[test]
fn zero_cap_refuses_before_anything_is_sent() {
    let root = temp_project("zerocap");
    write(
        &root,
        &Recipe {
            provider: pin("qwen"),
            ..image_recipe("sky", "blue sky", [512, 512])
        },
    );
    let t = Scripted::new(vec![]);
    let k = keys();
    let e = env(&k, &t, SpendCap::ZERO);
    let plan = generate::plan(&target(&root, &[]), &e).unwrap();
    assert_eq!(plan.steps[0].status, StepStatus::Blocked);
    assert_eq!(
        plan.steps[0].blocked.as_ref().unwrap().code,
        providers::codes::SPEND_CAP
    );
    assert!(
        plan.steps[0]
            .blocked
            .as_ref()
            .unwrap()
            .fix
            .as_ref()
            .unwrap()
            .contains("MARIONETTE_SPEND_CAP_USD")
    );
    let err = generate::run(&target(&root, &[]), &e, false).unwrap_err();
    assert_eq!(code(&err), providers::codes::SPEND_CAP);
    assert!(t.sent().is_empty());
    assert!(!root.join(spend::LEDGER_PATH).exists());
    assert!(!root.join(DIR).join("sky.png").exists());
    // Even with no key, the cap is the reason given first.
    let none = vars(&[]);
    let err = generate::run(&target(&root, &[]), &env(&none, &t, SpendCap::ZERO), false).unwrap_err();
    assert_eq!(code(&err), providers::codes::SPEND_CAP);
}

#[test]
fn a_paid_image_is_paid_once_and_then_comes_from_the_cache() {
    let root = temp_project("paidonce");
    write(
        &root,
        &Recipe {
            provider: pin("qwen"),
            texture: Some(TextureOptions {
                resize: Some([64, 64]),
                tileable: false,
            }),
            prompt: Some("mossy stone".into()),
            size: Some([128, 128]),
            ..recipe("stone", OutputKind::Texture)
        },
    );
    let t = Scripted::new(vec![
        recorded("qwen_submit.json"),
        recorded("qwen_status_running.json"),
        recorded("qwen_status_succeeded.json"),
        bytes(mock_png(128, 128)),
    ]);
    let k = keys();
    let e = env(&k, &t, cap(1.0));
    let plan = generate::plan(&target(&root, &[]), &e).unwrap();
    assert_eq!(plan.steps[0].status, StepStatus::Call);
    assert_eq!(plan.total_estimate_usd, providers::qwen::PRICE_PER_IMAGE);
    assert!(plan.within_cap);

    let r = generate::run(&target(&root, &[]), &e, false).unwrap();
    assert_eq!((r.calls, r.built[0].how.as_str()), (1, "called"));
    assert_eq!(t.left(), 0);
    let sent = t.sent();
    assert_eq!(sent.len(), 4);
    assert_eq!(sent[0].body_json().unwrap()["input"]["prompt"], "mossy stone");
    assert!(
        !format!("{:?}", sent).contains(FAKE_KEY),
        "keys never show in debug output"
    );
    let ledger = spend::entries(&root.join(spend::LEDGER_PATH)).unwrap();
    assert_eq!(ledger.len(), 1);
    let rec = provenance::read(&root.join(DIR), "stone").unwrap();
    assert_eq!(rec.source.provider, "qwen");
    assert!(!rec.source.provider_verified);
    assert!(rec.license.terms.contains("terms of service"));
    assert_eq!(
        marionette_assets::decode_png(&read(&root, "stone.png"))
            .unwrap()
            .width,
        64
    );

    // Same request again: nothing sent, nothing spent.
    let again = generate::run(&target(&root, &[]), &e, false).unwrap();
    assert_eq!((again.calls, again.cost_usd), (0, 0.0));
    // Output lost and a post-processing change: rebuilt from the cache, still not paid.
    write(
        &root,
        &Recipe {
            provider: pin("qwen"),
            texture: Some(TextureOptions {
                resize: Some([32, 32]),
                tileable: true,
            }),
            prompt: Some("mossy stone".into()),
            size: Some([128, 128]),
            ..recipe("stone", OutputKind::Texture)
        },
    );
    let rebuilt = generate::run(&target(&root, &[]), &e, false).unwrap();
    assert_eq!(rebuilt.built[0].how, "cached");
    assert_eq!(t.sent().len(), 4);
    assert_eq!(spend::entries(&root.join(spend::LEDGER_PATH)).unwrap().len(), 1);
    assert_eq!(
        marionette_assets::decode_png(&read(&root, "stone.png"))
            .unwrap()
            .width,
        32
    );
    // Switching providers.json does not invalidate the built asset either.
    let mut cfg = ProvidersConfig::new();
    cfg.set(providers::Kind::Image, Choice::mock());
    std::fs::write(ProvidersConfig::path(&root), cfg.to_text()).unwrap();
    assert_eq!(generate::run(&target(&root, &[]), &e, false).unwrap().calls, 0);
}

#[test]
fn a_paid_model_needs_a_person_to_approve_its_image() {
    let root = temp_project("approval");
    write(
        &root,
        &image_recipe("hero_ref", "a small robot, front view", [128, 128]),
    );
    write(
        &root,
        &Recipe {
            from: Some("hero_ref".into()),
            provider: pin("tripo"),
            ..recipe("hero", OutputKind::Model)
        },
    );
    let k = keys();
    // First run: the free image is built; the paid model is refused, nothing is sent.
    let t = Scripted::new(vec![]);
    let e = env(&k, &t, cap(5.0));
    let plan = generate::plan(&target(&root, &[]), &e).unwrap();
    let hero = plan.steps.iter().find(|s| s.name == "hero").unwrap();
    assert_eq!(hero.status, StepStatus::WaitsForImage);
    assert!(hero.needs_approval);
    let r = generate::run(&target(&root, &[]), &e, false).unwrap();
    assert_eq!(r.built.len(), 1);
    assert_eq!(r.blocked[0].name, "hero");
    assert_eq!(r.blocked[0].problem.code, codes::NOT_APPROVED);
    assert!(t.sent().is_empty());

    // Tools cannot approve: the approval file name is refused by every writer, and the
    // approval call itself needs a person.
    let dir = root.join(DIR);
    let err = generate::write_file(&dir.join("hero_ref.approval.json"), "x", b"{}").unwrap_err();
    assert_eq!(code(&err), codes::APPROVAL_WRITE_REFUSED);
    let (_, file, sha) = generate::approvable(&target(&root, &[]), "hero_ref").unwrap();
    let err = approval::approve_by_person(&dir, "hero_ref", &file, &sha, false).unwrap_err();
    assert_eq!(code(&err), codes::APPROVAL_NEEDS_PERSON);

    // A person approves (as `marionette generate approve` does after typing the name).
    approval::approve_by_person(&dir, "hero_ref", &file, &sha, true).unwrap();
    let ref_png = read(&root, "hero_ref.png");
    let t = Scripted::new(vec![
        recorded("tripo_upload.json"),
        recorded("tripo_submit.json"),
        recorded("tripo_status_running.json"),
        recorded("tripo_status_success.json"),
        bytes(mock_glb()),
    ]);
    let e = env(&k, &t, cap(5.0));
    let r = generate::run(&target(&root, &[]), &e, false).unwrap();
    assert!(r.blocked.is_empty(), "{:?}", r.blocked);
    let sent = t.sent();
    assert!(sent[0].url.ends_with("/v2/openapi/upload"));
    let body = sent[0].body.as_ref().unwrap();
    assert!(
        body.windows(ref_png.len()).any(|w| w == ref_png.as_slice()),
        "the approved image is what gets uploaded"
    );
    let submit = sent[1].body_json().unwrap();
    assert_eq!(submit["type"], "image_to_model");
    assert_eq!(submit["file"]["file_token"], "tripo-image-token-0001");
    let rec = provenance::read(&dir, "hero").unwrap();
    assert!(rec.source.from.as_ref().unwrap().approved);
    assert_eq!(rec.source.cost_usd, providers::tripo::PRICE_PER_MODEL);

    // The image changes: its approval no longer applies, and the model is refused again.
    write(
        &root,
        &image_recipe("hero_ref", "a tall robot, front view", [128, 128]),
    );
    let t = Scripted::new(vec![]);
    let r = generate::run(&target(&root, &[]), &env(&k, &t, cap(5.0)), false).unwrap();
    assert_eq!(r.blocked[0].problem.code, codes::NOT_APPROVED);
    assert!(t.sent().is_empty());
}

#[test]
fn a_run_over_budget_is_refused_up_front() {
    let root = temp_project("budget");
    for name in ["a", "b"] {
        write(
            &root,
            &Recipe {
                provider: pin("qwen"),
                ..image_recipe(name, &format!("tile {name}"), [64, 64])
            },
        );
    }
    let t = Scripted::new(vec![]);
    let k = keys();
    // One image fits under the cap, two do not: nothing is sent at all.
    let e = env(&k, &t, cap(providers::qwen::PRICE_PER_IMAGE * 1.5));
    let plan = generate::plan(&target(&root, &[]), &e).unwrap();
    assert_eq!(plan.steps[1].status, StepStatus::Blocked);
    let err = generate::run(&target(&root, &[]), &e, false).unwrap_err();
    assert_eq!(code(&err), providers::codes::SPEND_CAP);
    assert!(t.sent().is_empty());
}

#[test]
fn unusable_provider_output_is_refused_and_not_written() {
    let root = temp_project("badglb");
    write(
        &root,
        &Recipe {
            prompt: Some("a barrel".into()),
            provider: pin("tripo"),
            ..recipe("barrel", OutputKind::Model)
        },
    );
    let mut fake = b"glTF".to_vec();
    fake.extend_from_slice(&[2, 0, 0, 0, 12, 0, 0, 0]);
    let t = Scripted::new(vec![
        recorded("tripo_submit.json"),
        recorded("tripo_status_success.json"),
        bytes(fake),
    ]);
    let k = keys();
    let err = generate::run(&target(&root, &[]), &env(&k, &t, cap(1.0)), false).unwrap_err();
    assert_eq!(code(&err), codes::POSTPROCESS);
    assert!(!root.join(DIR).join("barrel.glb").exists());
    // It was paid for, so it is cached and recorded in the ledger: a retry does not pay again.
    assert_eq!(spend::entries(&root.join(spend::LEDGER_PATH)).unwrap().len(), 1);
    let t2 = Scripted::new(vec![]);
    let err = generate::run(&target(&root, &[]), &env(&k, &t2, cap(1.0)), false).unwrap_err();
    assert_eq!(code(&err), codes::POSTPROCESS);
    assert!(t2.sent().is_empty());
}

#[test]
fn bad_recipes_are_refused() {
    let root = temp_project("bad");
    let t = target(&root, &[]);
    let bad = |r: Recipe| generate::write_recipe(&t, &r, true).unwrap_err();
    assert_eq!(
        code(&bad(recipe("x", OutputKind::Image))),
        codes::BAD_RECIPE,
        "no prompt"
    );
    assert_eq!(
        code(&bad(image_recipe("Bad Name", "p", [64, 64]))),
        codes::BAD_RECIPE
    );
    assert_eq!(code(&bad(image_recipe("x", "p", [8, 64]))), codes::BAD_RECIPE);
    assert_eq!(
        code(&bad(Recipe {
            texture: Some(TextureOptions {
                resize: Some([100, 64]),
                tileable: false
            }),
            ..Recipe {
                output: OutputKind::Texture,
                ..image_recipe("x", "p", [64, 64])
            }
        })),
        codes::BAD_RECIPE
    );
    assert_eq!(
        code(&bad(Recipe {
            provider: pin("tripo"),
            ..image_recipe("x", "p", [64, 64])
        })),
        codes::BAD_RECIPE,
        "tripo does not make images"
    );
    assert_eq!(
        code(&bad(Recipe {
            from: Some("nothing".into()),
            ..recipe("m", OutputKind::Model)
        })),
        codes::BAD_RECIPE,
        "from names a missing recipe"
    );
    // Hand-written files: unknown fields and a name that does not match the file.
    let dir = root.join(DIR);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("k.recipe.json"),
        r#"{"name": "k", "output": "image", "prompt": "p", "api_key": "x"}"#,
    )
    .unwrap();
    let err = generate::list(&t).unwrap_err();
    assert_eq!(code(&err), codes::BAD_RECIPE);
    assert!(err.0[0].line.is_some());
    std::fs::write(
        dir.join("k.recipe.json"),
        r#"{"name": "other", "output": "image", "prompt": "p"}"#,
    )
    .unwrap();
    assert_eq!(code(&generate::list(&t).unwrap_err()), codes::BAD_RECIPE);
    // Existing recipes are not replaced by accident.
    std::fs::remove_file(dir.join("k.recipe.json")).unwrap();
    generate::write_recipe(&t, &image_recipe("k", "p", [64, 64]), false).unwrap();
    assert_eq!(
        code(&generate::write_recipe(&t, &image_recipe("k", "q", [64, 64]), false).unwrap_err()),
        codes::BAD_RECIPE
    );
    // Unknown recipe names in a run.
    let none = vars(&[]);
    let names = ["nope".to_string()];
    let err = generate::run(
        &target(&root, &names),
        &env(&none, &NoNetwork, SpendCap::ZERO),
        false,
    )
    .unwrap_err();
    assert_eq!(code(&err), codes::NOT_FOUND);
}

#[test]
fn generated_assets_and_their_copies_are_blocked_from_the_pool() {
    let root = temp_project("pool");
    write(&root, &image_recipe("leaf", "a leaf", [64, 64]));
    let none = vars(&[]);
    generate::run(
        &target(&root, &[]),
        &env(&none, &NoNetwork, SpendCap::ZERO),
        false,
    )
    .unwrap();
    let dir = root.join(DIR);
    let asset = dir.join("leaf.png");
    let s = provenance::pool_status(&asset, &[]);
    assert!(s.generated && !s.admissible);
    let copy = root.join("elsewhere.png");
    std::fs::copy(&asset, &copy).unwrap();
    assert!(
        provenance::pool_status(&copy, &[]).admissible,
        "unknown without the folder"
    );
    assert!(!provenance::pool_status(&copy, &[dir.as_path()]).admissible);
    let other = root.join("other.png");
    std::fs::write(&other, mock_png(16, 16)).unwrap();
    assert!(provenance::pool_status(&other, &[dir.as_path()]).admissible);
    let listed = generate::list(&target(&root, &[])).unwrap();
    assert_eq!(listed[0].state, "built");
    assert_eq!(listed[0].pool, "blocked");
}
