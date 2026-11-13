//! Porting acceptance tests on the Unity and Unreal fixtures (decision 0020).

use marionette_port::map::{Role, Status};
use marionette_port::{Analysis, PortReport, scaffold, scan};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn report(name: &str) -> (Analysis, PortReport) {
    let a = scan(&fixture(name)).expect("fixture scans");
    let r = a.report(0, 500, 10);
    (a, r)
}

fn class<'a>(r: &'a PortReport, name: &str) -> &'a marionette_port::ClassReport {
    r.classes
        .iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("class {name} not found"))
}

fn uses(r: &PortReport, name: &str) -> Option<(Status, usize)> {
    r.api.iter().find(|u| u.name == name).map(|u| (u.status, u.count))
}

#[test]
fn unity_classes_roles_hooks_and_components() {
    let (_, r) = report("unity");
    assert_eq!(class(&r, "PlayerController").role, Role::Behaviour);
    // Inherits MonoBehaviour through a scanned base class.
    assert_eq!(class(&r, "Enemy").role, Role::Behaviour);
    assert_eq!(class(&r, "Enemy").engine_base.as_deref(), Some("MonoBehaviour"));
    assert_eq!(class(&r, "PlayerEditor").role, Role::Editor);
    assert_eq!(class(&r, "MathUtil").role, Role::Plain);
    assert_eq!(r.summary.scripts, 3);

    let p = class(&r, "PlayerController");
    let hook = |m: &str| {
        p.methods
            .iter()
            .find(|x| x.name == m)
            .and_then(|x| x.hook.clone())
    };
    assert_eq!(hook("Awake").as_deref(), Some("start"));
    assert_eq!(hook("Start").as_deref(), Some("start"));
    assert_eq!(hook("Update").as_deref(), Some("update"));
    assert_eq!(hook("OnTriggerEnter").as_deref(), Some("onTriggerEnter"));
    assert_eq!(hook("OnDestroy").as_deref(), Some("onDestroy"));
    assert!(p.methods.iter().find(|m| m.name == "Blink").unwrap().coroutine);
    // Invoke("Respawn") and StartCoroutine(Blink()) bind their methods.
    assert_eq!(
        p.methods
            .iter()
            .find(|m| m.name == "Respawn")
            .unwrap()
            .bound_by
            .as_deref(),
        Some("Invoke")
    );
    assert_eq!(
        p.methods
            .iter()
            .find(|m| m.name == "Blink")
            .unwrap()
            .bound_by
            .as_deref(),
        Some("StartCoroutine")
    );

    let comps: Vec<&str> = p.components.iter().map(|c| c.source.as_str()).collect();
    assert_eq!(comps, ["BoxCollider", "Rigidbody"]);

    let field = |n: &str| {
        p.fields
            .iter()
            .find(|f| f.name == n)
            .unwrap_or_else(|| panic!("field {n}"))
    };
    assert_eq!(field("speed").default, Some(serde_json::json!(5)));
    assert!(field("speed").editable, "[SerializeField] private is editable");
    assert!(!field("coins").editable);
    assert_eq!(field("lives").default, Some(serde_json::json!(3)));
    assert_eq!(
        field("spawnPoint").default,
        Some(serde_json::json!({"x": 0, "y": 1, "z": 2})),
        "z flipped"
    );
    assert_eq!(field("target").kind, "entity");
    assert_eq!(field("rb").kind, "component");
    assert_eq!(field("scores").ts_type, "number[]");
    assert_eq!(field("mode").default, Some(serde_json::json!("Walking")));
    assert!(
        p.fields.iter().all(|f| f.name != "MaxSpeed" && f.name != "Score"),
        "consts and properties are not state"
    );
}

#[test]
fn unity_api_uses_actions_gaps_and_hazards() {
    let (_, r) = report("unity");
    // Text in strings and comments is not code.
    assert_eq!(uses(&r, "Input.GetAxis"), Some((Status::Script, 1)));
    assert_eq!(uses(&r, "Input.GetButtonDown"), Some((Status::Script, 1)));
    assert_eq!(uses(&r, "Time.deltaTime"), Some((Status::Script, 1)));
    assert_eq!(uses(&r, "transform.position").map(|u| u.1), Some(2));
    assert_eq!(uses(&r, "Mathf.Sin").map(|u| u.0), Some(Status::Hazard));
    assert_eq!(uses(&r, "DateTime.Now").map(|u| u.0), Some(Status::Hazard));
    assert_eq!(uses(&r, "AddForce").map(|u| u.0), Some(Status::Gap));
    assert_eq!(uses(&r, "Destroy").map(|u| u.0), Some(Status::Script));
    assert_eq!(uses(&r, "Invoke").map(|u| u.0), Some(Status::Script));
    assert_eq!(uses(&r, "StartCoroutine").map(|u| u.0), Some(Status::Rewrite));
    assert_eq!(uses(&r, "GameObject.Find").map(|u| u.0), Some(Status::Script));
    assert_eq!(r.summary.actions, ["Horizontal", "Jump"]);
    assert!(
        !r.summary
            .top_gaps
            .iter()
            .any(|g| g.contains("OnDestroy") || g.contains("Destroy (")),
        "{:?}",
        r.summary.top_gaps
    );
    let loc = &r.api.iter().find(|u| u.name == "Mathf.Sin").unwrap().locations[0];
    assert_eq!(
        (loc.file.as_str(), loc.within.as_deref()),
        (
            "Assets/Scripts/PlayerController.cs",
            Some("PlayerController.Update")
        )
    );
    // Hazards first, then gaps.
    assert_eq!(r.api[0].status, Status::Hazard);
}

#[test]
fn unreal_classes_definitions_and_bindings() {
    let (_, r) = report("unreal");
    let c = class(&r, "ADemoCharacter");
    assert_eq!(c.role, Role::Actor);
    assert_eq!(c.engine_base.as_deref(), Some("ACharacter"));
    let m = |n: &str| {
        c.methods
            .iter()
            .find(|x| x.name == n)
            .unwrap_or_else(|| panic!("method {n}"))
    };
    assert_eq!(m("BeginPlay").hook.as_deref(), Some("start"));
    assert_eq!(m("Tick").hook.as_deref(), Some("update"));
    assert!(
        m("Tick").file.ends_with(".cpp") && m("Tick").has_body,
        "the .cpp definition replaces the header declaration"
    );
    assert_eq!(m("OnPickupOverlap").hook.as_deref(), Some("onTriggerEnter"));
    assert_eq!(
        m("OnPickupOverlap").bound_by.as_deref(),
        Some("OnComponentBeginOverlap.AddDynamic")
    );
    assert_eq!(
        m("MoveForward").bound_by.as_deref(),
        Some("PlayerInputComponent.BindAxis")
    );
    assert!(
        c.methods
            .iter()
            .all(|x| x.name != "BeginPlay" || x.file.ends_with(".cpp"))
    );
    // Super::BeginPlay() is a call, not a definition of a class named Super.
    assert!(r.classes.iter().all(|c| c.name != "Super"));

    let f = |n: &str| {
        c.fields
            .iter()
            .find(|x| x.name == n)
            .unwrap_or_else(|| panic!("field {n}"))
    };
    assert_eq!(f("TurnRate").default, Some(serde_json::json!(45)));
    assert!(f("TurnRate").editable);
    assert_eq!(
        f("Ammo").default,
        Some(serde_json::json!(30)),
        "brace initializer"
    );
    assert_eq!(
        f("MuzzleOffset").default,
        Some(serde_json::json!({"x": 100, "y": 0, "z": 50})),
        "Unreal values are not converted automatically"
    );
    assert_eq!(f("PickupBox").kind, "component");
    assert_eq!(
        c.components.iter().map(|x| x.source.as_str()).collect::<Vec<_>>(),
        ["UBoxComponent"]
    );

    assert_eq!(uses(&r, "FMath.Sin").map(|u| u.0), Some(Status::Hazard));
    assert_eq!(uses(&r, "GetWorld.SpawnActor").map(|u| u.0), Some(Status::Script));
    assert_eq!(
        uses(&r, "GetTimerManager.SetTimer").map(|u| u.0),
        Some(Status::Script)
    );
    assert_eq!(uses(&r, "GetActorLocation").map(|u| u.1), Some(2));
    assert_eq!(r.summary.actions, ["Fire", "MoveForward"]);
}

#[test]
fn blueprint_text_is_read() {
    let (_, r) = report("unreal");
    let bp = class(&r, "BP_Door");
    assert_eq!(bp.role, Role::Blueprint);
    let hooks: Vec<_> = bp
        .methods
        .iter()
        .map(|m| (m.name.as_str(), m.hook.as_deref()))
        .collect();
    assert_eq!(
        hooks,
        [
            ("ReceiveBeginPlay", Some("start")),
            ("ReceiveActorBeginOverlap", Some("onTriggerEnter"))
        ]
    );
    assert_eq!(uses(&r, "Delay").map(|u| u.0), Some(Status::Rewrite));
    assert_eq!(uses(&r, "K2_SetActorLocation").map(|u| u.0), Some(Status::Script));
}

#[test]
fn reports_and_skeletons_are_deterministic() {
    for name in ["unity", "unreal"] {
        let (a1, r1) = report(name);
        let (a2, r2) = report(name);
        assert_eq!(
            serde_json::to_string(&r1).unwrap(),
            serde_json::to_string(&r2).unwrap()
        );
        for ci in 0..a1.classes.len() {
            assert_eq!(
                scaffold::skeleton(&a1, ci, "x.ts"),
                scaffold::skeleton(&a2, ci, "x.ts")
            );
        }
    }
}

#[test]
fn skeletons_keep_every_original_line_and_annotate_calls() {
    let (a, _) = report("unity");
    let ci = a
        .classes
        .iter()
        .position(|c| c.raw.name == "PlayerController")
        .unwrap();
    let ts = scaffold::skeleton(&a, ci, "../scripts/ported/PlayerController.ts");
    assert!(ts.contains("\"Script\": { \"path\": \"../scripts/ported/PlayerController.ts\" }"));
    assert!(ts.contains("export function start(self: Entity): void {"));
    assert!(ts.contains("export function update(self: Entity): void {"));
    assert!(ts.contains("export function onTriggerEnter(self: Entity, other: Entity): void {"));
    assert!(ts.contains("self.state.speed = self.state.speed ?? 5;"));
    assert!(ts.contains("-> Input.GetAxis \"Horizontal\": engine.action(\"Horizontal\") [script]"));
    assert!(ts.contains("-> GetComponent<Rigidbody>: self.get(\"RigidBody\") [script]"));
    // OnDestroy has a hook; Invoke and coroutine targets are exported timer callbacks.
    assert!(ts.contains("export function onDestroy(self: Entity): void {"));
    assert!(ts.contains("export function Respawn(self: Entity"));
    assert!(ts.contains("export function Blink(self: Entity"));
    assert!(ts.contains("engine.after(seconds, fn) [script]"), "{ts}");
    // Awake goes before Start inside start().
    assert!(ts.find("--- Awake").unwrap() < ts.find("--- Start").unwrap());
    let src = std::fs::read_to_string(fixture("unity/Assets/Scripts/PlayerController.cs")).unwrap();
    for line in src.lines().skip(35).take(11) {
        let l = line.trim();
        if !l.is_empty() {
            assert!(ts.contains(l), "original line kept: {l}");
        }
    }
}

/// Generated skeletons type-check with `tsc --strict` against the script API typings that
/// `marionette docs` generates (`docs/ai/reference/marionette.d.ts`). Skipped when `tsc` is
/// not installed.
#[test]
fn skeletons_type_check_with_tsc() {
    let Ok(out) = std::process::Command::new("tsc").arg("--version").output() else {
        eprintln!("tsc not installed; skipping");
        return;
    };
    if !out.status.success() {
        eprintln!("tsc not usable; skipping");
        return;
    }
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("port-tsc");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // The generated script typings (decision 0022), as committed in the reference docs.
    let typings = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/ai/reference/marionette.d.ts");
    std::fs::copy(typings, dir.join("api.d.ts")).unwrap();
    let mut files = vec!["api.d.ts".to_string()];
    for name in ["unity", "unreal"] {
        let (a, _) = report(name);
        for (ci, c) in a.classes.iter().enumerate() {
            if Analysis::makes_script(c) {
                let f = format!("{name}_{}.ts", c.raw.name);
                std::fs::write(dir.join(&f), scaffold::skeleton(&a, ci, "x.ts")).unwrap();
                files.push(f);
            }
        }
    }
    let out = std::process::Command::new("tsc")
        .current_dir(&dir)
        .args([
            "--strict",
            "--noEmit",
            "--target",
            "es2022",
            "--module",
            "es2022",
            "--skipLibCheck",
        ])
        .args(&files)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "tsc failed:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn missing_and_empty_sources_are_errors() {
    let e = scan(Path::new("definitely/not/here")).err().unwrap();
    assert_eq!(e.0[0].code, marionette_port::codes::SOURCE);
    let empty = Path::new(env!("CARGO_TARGET_TMPDIR")).join("port-empty");
    std::fs::create_dir_all(&empty).unwrap();
    let e = scan(&empty).err().unwrap();
    assert_eq!(e.0[0].code, marionette_port::codes::NOTHING_FOUND);
}

/// Burst jobs, partial classes and C# tuple types (a real project crashed the scanner on
/// tuple types; decision 0072).
#[test]
fn unity_jobs_partials_and_tuples() {
    let (a, r) = report("unity-jobs");
    assert_eq!(class(&r, "FlowJob").role, Role::Job);
    assert_eq!(class(&r, "Cell").role, Role::Plain);
    assert_eq!(r.summary.jobs, 1);
    // The two parts of FlowWorld are one class: a behaviour (base from the second part)
    // with the methods of both files.
    let parts: Vec<_> = r.classes.iter().filter(|c| c.name == "FlowWorld").collect();
    assert_eq!(parts.len(), 1, "partial class reported once");
    let w = parts[0];
    assert_eq!(w.role, Role::Behaviour);
    for m in ["Count", "Step", "Update", "OnDestroy"] {
        assert!(w.methods.iter().any(|x| x.name == m), "{m} missing");
    }
    assert!(!w.methods.iter().any(|x| x.name == "readonly"));
    assert_eq!(uses(&r, "NativeArray").map(|u| u.0), Some(Status::Rewrite));
    assert_eq!(uses(&r, "BurstCompile").map(|u| u.0), Some(Status::Ready));
    assert_eq!(uses(&r, "math.min").map(|u| u.0), Some(Status::Script));
    assert!(r.next_steps.iter().any(|s| s.contains("job struct")));

    // The Rust skeleton names the job and its struct, maps the types, keeps the body.
    let rs = scaffold::rust_jobs(&a).expect("jobs found");
    assert!(rs.contains("pub struct Cell {"));
    assert!(rs.contains("pub kind: i32 /* enum CellKind */,"));
    assert!(rs.contains("pub cells: Vec<Cell>,"));
    assert!(rs.contains("pub order: Vec<i32>, // read-only"));
    assert!(rs.contains("pub tick: i64,"));
    assert!(rs.contains("pub fn execute(&mut self) {"));
    assert!(rs.contains("//       tick.Value = tick.Value + 1;"));
    // It compiles as it is when rustc is on the path.
    let dir = std::env::temp_dir().join(format!("marionette-port-jobs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("jobs.rs");
    std::fs::write(&file, &rs).unwrap();
    if let Ok(out) = std::process::Command::new("rustc")
        .args([
            "--edition",
            "2024",
            "--crate-type",
            "lib",
            "--emit",
            "metadata",
            "-o",
        ])
        .arg(dir.join("jobs.rmeta"))
        .arg(&file)
        .output()
    {
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    }
    std::fs::remove_dir_all(&dir).ok();
    assert!(scaffold::rust_jobs(&report("unity").0).is_none());
}

// ---------------------------------------------------------------------------------------
// Serializable types to serde (decision 0240)

fn serial() -> marionette_port::serde_gen::Generated {
    let a = scan(&fixture("serial")).expect("fixture scans");
    marionette_port::serde_gen::generate(&a, &Default::default())
}

#[test]
fn serde_generator_maps_jsonutility_rules() {
    let g = serial();
    let names: Vec<&str> = g.types.iter().map(|t| t.name.as_str()).collect();
    for n in [
        "Profile",
        "Slot",
        "Inner",
        "Split",
        "Mode",
        "Perks",
        "Vector3",
        "Quaternion",
        "Color",
        "ObjectRef",
    ] {
        assert!(names.contains(&n), "{n} missing: {names:?}");
    }
    // Not [Serializable]: never emitted.
    assert!(!names.contains(&"Helper"));
    let r = &g.rust;
    // Base fields first, then own fields; keywords and case handled with renames.
    let base = r.find("pub version").unwrap();
    assert!(base < r.find("pub name").unwrap());
    assert!(r.contains("#[serde(rename = \"type\")]\n    pub type_: i32"));
    assert!(r.contains("#[serde(rename = \"HPValue\")]\n    pub hp_value: i32"));
    assert!(r.contains("#[serde(rename = \"_underscore\")]\n    pub underscore: i32"));
    // Private [SerializeField] is saved; plain private, static, const, properties, dictionaries are not.
    assert!(r.contains("pub hidden: i32"));
    for gone in [
        "not_saved",
        "skipped",
        "count",
        "max",
        "level",
        "lookup",
        "jagged",
    ] {
        assert!(!r.contains(&format!("pub {gone}:")), "{gone} must not be saved");
    }
    // Unknown type and unsaved class: unknown is kept raw, plain class is skipped.
    assert!(r.contains("pub other: serde_json::Value"));
    assert!(!r.contains("pub helper"));
    // Initializers become the Default.
    assert!(r.contains("speed: 1.5,"));
    assert!(r.contains("ratio: 2.0,"));
    assert!(r.contains("ticks: -7,"));
    assert!(r.contains("big: 255,"));
    assert!(r.contains("mode: Mode::Busy,"));
    assert!(r.contains("name: \"new \\\"player\\\"\".to_string(),"));
    assert!(r.contains("ids: vec![4, 5],"));
    assert!(r.contains("position: Vector3 { x: 1.0, y: 2.0, z: 3.0 },"));
    assert!(r.contains("rotation: Quaternion { x: 0.0, y: 0.0, z: 0.0, w: 1.0 },"));
    assert!(r.contains("inner: Inner::default(),"));
    // Enum values, including shifts and earlier members, and a byte underlying type.
    assert!(r.contains("pub const Done: Mode = Mode(6);"));
    assert!(r.contains("pub struct Perks(pub u8);"));
    assert!(r.contains("pub const Both: Perks = Perks(3);"));
    // Partial parts merge.
    assert!(r.contains("pub a: i32") && r.contains("pub b: i32"));
    let kinds: Vec<(&str, &str)> = g.notes.iter().map(|n| (n.kind.as_str(), n.at.as_str())).collect();
    assert!(kinds.contains(&("skipped", "Profile.lookup")), "{kinds:?}");
    assert!(kinds.contains(&("skipped", "Profile.jagged")));
    assert!(kinds.contains(&("skipped", "Profile.helper")));
    assert!(kinds.contains(&("unresolved", "Profile.other")));
    assert!(kinds.contains(&("initializer", "Profile.computed")));
}

/// The generated module for the fixture, kept in the repository: it is compiled below and
/// must equal what the generator writes now (regenerate with `MARIONETTE_BLESS=1`).
#[rustfmt::skip]
#[path = "generated/serial_fixture.rs"]
mod generated;

#[test]
fn serde_generated_code_is_current_and_round_trips() {
    let g = serial();
    let golden = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/generated/serial_fixture.rs");
    if std::env::var_os("MARIONETTE_BLESS").is_some() {
        std::fs::write(&golden, &g.rust).unwrap();
    }
    assert_eq!(
        std::fs::read_to_string(&golden).unwrap(),
        g.rust,
        "generated module is stale: run with MARIONETTE_BLESS=1"
    );
    // Missing keys keep the C# initializers; nested objects are never null.
    let p: generated::Profile =
        generated::from_json("\u{feff}{\"name\":\"x\",\"slots\":[{\"id\":3}]}").unwrap();
    assert_eq!(p.name, "x");
    assert_eq!(p.version, 3);
    assert_eq!(p.speed, 1.5);
    assert_eq!(p.ids, vec![4, 5]);
    assert_eq!(p.mode, generated::Mode::Busy);
    assert_eq!(p.slots[0].id, 3);
    assert_eq!(p.inner.underscore, 1);
    assert_eq!(p.rotation.w, 1.0);
    // Unknown keys are ignored; unknown enum integers survive a round trip.
    let text = "{\"mode\":42,\"perks\":3,\"extra\":true,\"other\":{\"k\":[1]}}";
    let p: generated::Profile = generated::from_json(text).unwrap();
    assert_eq!(p.mode, generated::Mode(42));
    let back = generated::to_json(&p).unwrap();
    let again: generated::Profile = generated::from_json(&back).unwrap();
    assert_eq!(p, again);
    assert!(back.contains("\"mode\":42") && back.contains("\"other\":{\"k\":[1]}"));
    // Fields are written in declaration order, base class first, every field present.
    assert!(back.find("\"version\"").unwrap() < back.find("\"name\"").unwrap());
    assert!(back.contains("\"inner\":{") && back.contains("\"tags\":[]"));
}
