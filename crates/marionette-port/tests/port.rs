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
    assert_eq!(hook("OnTriggerEnter").as_deref(), Some("onCollisionEnter"));
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
    assert_eq!(m("OnPickupOverlap").hook.as_deref(), Some("onCollisionEnter"));
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
            ("ReceiveActorBeginOverlap", Some("onCollisionEnter"))
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
    assert!(ts.contains("export function onCollisionEnter(self: Entity, other: Entity): void {"));
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
