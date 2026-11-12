//! Porting Unity and Unreal gameplay code to Marionette (decision 0020).
//!
//! - [`scan`] reads C# (Unity) and C++ (Unreal) sources and Blueprint graphs copied as text,
//!   finds the gameplay classes, their fields and lifecycle methods, and every engine API call,
//!   and maps each to its Marionette equivalent with a [`Status`]. The result is a
//!   [`PortReport`] (operation `port_scan`).
//! - [`scaffold`] writes one TypeScript script skeleton per gameplay class: exported hooks with
//!   the original code kept as comments, each engine call annotated with its equivalent
//!   (operation `port_scaffold`).
//! - [`convert`] converts positions, directions, rotations and scales between engine
//!   coordinate systems (operation `port_convert`).
//! - [`map`] holds the concept tables (operation `port_map`, generated reference
//!   `docs/ai/reference/porting-map.md`).
//!
//! The guide for agents is `docs/ai/PORTING.md`.

pub mod convert;
pub mod lex;
pub mod map;
pub mod scaffold;
pub mod source;

use map::{Engine, Entry, Role, Status, table};
use marionette_core::{Diagnostic, Errors};
use schemars::JsonSchema;
use serde::Serialize;
use source::{Chain, RawClass, SourceFile};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

pub use map::Status as PortStatus;

/// Stable error codes added by this crate.
pub mod codes {
    /// The scan source does not exist or cannot be read.
    pub const SOURCE: &str = "port.source";
    /// No C#, C++ or Blueprint text files were found under the source.
    pub const NOTHING_FOUND: &str = "port.nothing_found";
    /// A conversion input is malformed (wrong number of components, unknown kind).
    pub const BAD_VALUE: &str = "port.bad_value";
    /// A scaffold output file exists and `overwrite` was not set.
    pub const EXISTS: &str = "port.exists";
}

/// Limits that keep a scan bounded on very large projects.
pub const MAX_FILES: usize = 20_000;
pub const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;

/// Folders never scanned: engine caches, build output, version control.
const SKIP_DIRS: [&str; 14] = [
    "Library",
    "Temp",
    "Logs",
    "obj",
    "bin",
    "Build",
    "Builds",
    "Intermediate",
    "Binaries",
    "Saved",
    "DerivedDataCache",
    "node_modules",
    "UserSettings",
    "MemoryCaptures",
];

fn engine_for(path: &Path) -> Option<(Engine, bool)> {
    let name = path.file_name()?.to_string_lossy().to_lowercase();
    if name.ends_with(".bp.txt") || name.ends_with(".t3d") {
        return Some((Engine::Unreal, true));
    }
    match path.extension()?.to_string_lossy().to_lowercase().as_str() {
        "cs" => Some((Engine::Unity, false)),
        "h" | "hpp" | "hh" | "cpp" | "cc" | "cxx" | "inl" => Some((Engine::Unreal, false)),
        _ => None,
    }
}

/// A file that was found but not read.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Skipped {
    pub file: String,
    pub reason: String,
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<PathBuf>, skipped: &mut Vec<Skipped>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for p in entries {
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if p.is_dir() {
            if name.starts_with('.') || SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            collect(root, &p, out, skipped);
        } else if engine_for(&p).is_some() {
            if out.len() >= MAX_FILES {
                skipped.push(Skipped {
                    file: rel(root, &p),
                    reason: format!("more than {MAX_FILES} source files"),
                });
                continue;
            }
            out.push(p);
        }
    }
}

fn rel(root: &Path, p: &Path) -> String {
    let r = p.strip_prefix(root).unwrap_or(p);
    let s = r.to_string_lossy().replace('\\', "/");
    if s.is_empty() {
        p.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    } else {
        s
    }
}

/// One mapped (or unknown) engine API use in the source.
#[derive(Debug, Clone)]
pub struct Occurrence {
    pub file: usize,
    pub line: u32,
    pub token: usize,
    pub engine: Engine,
    pub name: String,
    pub status: Status,
    pub marionette: String,
    pub note: String,
    /// The string-literal first argument (an axis or action name), if any.
    pub arg0: Option<String>,
    pub generic: Option<String>,
    pub args: Option<String>,
}

/// A class found in the source, with resolved role and methods.
#[derive(Debug, Clone)]
pub struct Class {
    pub raw: RawClass,
    pub role: Role,
    /// The engine base class reached through the inheritance chain (`MonoBehaviour`).
    pub engine_base: Option<String>,
    /// Methods bound to hooks by delegates (`OnComponentBeginOverlap.AddDynamic(this,
    /// &AFoo::OnOverlap)`) or input bindings: method name -> (pattern, hook or empty).
    pub bound: BTreeMap<String, (String, String)>,
}

/// Everything a scan found; reports and skeletons are built from it.
pub struct Analysis {
    pub root: PathBuf,
    pub files: Vec<SourceFile>,
    pub classes: Vec<Class>,
    pub occurrences: Vec<Occurrence>,
    pub enums: BTreeSet<String>,
    pub skipped: Vec<Skipped>,
}

struct Matcher {
    by_first: HashMap<String, Vec<(Vec<String>, &'static Entry)>>,
    roots: BTreeSet<&'static str>,
}

impl Matcher {
    fn new(engine: Engine) -> Self {
        let t = table(engine);
        let mut by_first: HashMap<String, Vec<(Vec<String>, &'static Entry)>> = HashMap::new();
        for e in &t.api {
            let segs: Vec<String> = e.name.split('.').map(str::to_string).collect();
            by_first.entry(segs[0].clone()).or_default().push((segs, e));
        }
        for list in by_first.values_mut() {
            // Longest patterns first.
            list.sort_by_key(|b| std::cmp::Reverse(b.0.len()));
        }
        Self {
            by_first,
            roots: t.roots.iter().map(String::as_str).collect(),
        }
    }

    fn single_ok(c: &Chain, p: usize, name: &str) -> bool {
        let seg = &c.segs[p];
        let upper = name.chars().next().is_some_and(char::is_uppercase);
        let member = p > 0 || c.leading_member;
        let last = p + 1 == c.segs.len();
        (seg.call || seg.generic.is_some()) && (last || seg.call)
            || (p == 0 && c.attribute)
            || (p == 0 && upper && c.segs.len() > 1)
            || (member && !upper)
    }

    fn matches(&self, file: usize, engine: Engine, c: &Chain, out: &mut Vec<Occurrence>) {
        if c.declaration {
            return;
        }
        let mut found = false;
        let mut p = 0;
        while p < c.segs.len() {
            let mut hit = None;
            if let Some(list) = self.by_first.get(&c.segs[p].name) {
                for (segs, e) in list {
                    let k = segs.len();
                    if p + k > c.segs.len() || !segs.iter().zip(&c.segs[p..p + k]).all(|(a, b)| *a == b.name)
                    {
                        continue;
                    }
                    if k == 1 && !Self::single_ok(c, p, &segs[0]) {
                        continue;
                    }
                    hit = Some((k, *e));
                    break;
                }
            }
            if let Some((k, e)) = hit {
                let seg = &c.segs[p + k - 1];
                out.push(Occurrence {
                    file,
                    line: c.line,
                    token: c.token,
                    engine,
                    name: e.name.clone(),
                    status: e.status,
                    marionette: e.marionette.clone(),
                    note: e.note.clone(),
                    arg0: seg.arg0.clone(),
                    generic: seg.generic.clone(),
                    args: seg.args.clone(),
                });
                found = true;
                p += k;
            } else {
                p += 1;
            }
        }
        if !found
            && !c.leading_member
            && self.roots.contains(c.segs[0].name.as_str())
            && (c.segs.len() > 1 || c.segs[0].call)
        {
            let name: Vec<&str> = c.segs.iter().take(2).map(|s| s.name.as_str()).collect();
            let seg = &c.segs[name.len() - 1];
            out.push(Occurrence {
                file,
                line: c.line,
                token: c.token,
                engine,
                name: name.join("."),
                status: Status::Unknown,
                marionette: String::new(),
                note: "Not in the porting tables yet; check docs/ai/reference/ and add an entry to crates/marionette-port/data/ if it recurs.".into(),
                arg0: seg.arg0.clone(),
                generic: seg.generic.clone(),
                args: seg.args.clone(),
            });
        }
    }
}

/// Scans a file or folder. Errors only when nothing can be read; problems with single files
/// are listed in `skipped`.
pub fn scan(source: &Path) -> Result<Analysis, Errors> {
    if !source.exists() {
        return Err(
            Diagnostic::error(codes::SOURCE, format!("`{}` does not exist", source.display()))
                .fix("pass a Unity Assets folder, an Unreal Source folder, or one .cs/.h/.cpp/.t3d file")
                .into(),
        );
    }
    let (root, paths, mut skipped) = if source.is_dir() {
        let mut paths = Vec::new();
        let mut skipped = Vec::new();
        collect(source, source, &mut paths, &mut skipped);
        (source.to_path_buf(), paths, skipped)
    } else {
        let root = source.parent().map(Path::to_path_buf).unwrap_or_default();
        (root, vec![source.to_path_buf()], Vec::new())
    };
    let mut files = Vec::new();
    for p in &paths {
        let shown = rel(&root, p);
        let Some((engine, blueprint)) = engine_for(p) else {
            skipped.push(Skipped {
                file: shown,
                reason: "not a .cs, C++ or Blueprint text (.t3d, .bp.txt) file".into(),
            });
            continue;
        };
        match std::fs::metadata(p) {
            Ok(m) if m.len() > MAX_FILE_BYTES => {
                skipped.push(Skipped {
                    file: shown,
                    reason: format!("larger than {MAX_FILE_BYTES} bytes"),
                });
                continue;
            }
            Err(e) => {
                skipped.push(Skipped {
                    file: shown,
                    reason: e.to_string(),
                });
                continue;
            }
            _ => {}
        }
        match std::fs::read(p) {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                let text = text.trim_start_matches('\u{feff}');
                files.push(SourceFile::new(shown, engine, blueprint, text));
            }
            Err(e) => skipped.push(Skipped {
                file: shown,
                reason: e.to_string(),
            }),
        }
    }
    if files.is_empty() {
        return Err(Diagnostic::error(codes::NOTHING_FOUND, format!("no source files under `{}`", source.display()))
            .fix("Unity scripts are .cs files under Assets/; Unreal code is .h/.cpp under Source/; Blueprints can be copied from the editor graph into a .t3d text file")
            .into());
    }
    Ok(analyze(root, files, skipped))
}

/// Analyzes in-memory sources (used by `scan` and by tests).
/// Whether a binding call (`port_scan` method `bound_by`) sets a timer or starts a coroutine,
/// whose callback becomes an exported function called by `engine.after` / `engine.every`.
pub fn is_timer_binder(pattern: &str) -> bool {
    ["SetTimer", "Invoke", "InvokeRepeating", "StartCoroutine"]
        .iter()
        .any(|b| pattern == *b || pattern.ends_with(&format!(".{b}")))
}

pub fn analyze(root: PathBuf, files: Vec<SourceFile>, skipped: Vec<Skipped>) -> Analysis {
    let mut raw: Vec<RawClass> = Vec::new();
    let mut defs: Vec<(String, source::RawMethod)> = Vec::new();
    let mut occurrences = Vec::new();
    let mut enums = BTreeSet::new();
    let unity = Matcher::new(Engine::Unity);
    let unreal = Matcher::new(Engine::Unreal);
    for (fi, f) in files.iter().enumerate() {
        if f.blueprint {
            raw.push(blueprint_class(fi, f, &mut occurrences));
            continue;
        }
        raw.extend(source::classes(&f.tokens, f.engine, fi));
        if f.engine == Engine::Unreal {
            defs.extend(source::cpp_definitions(&f.tokens, fi));
        }
        enums.extend(source::enums(&f.tokens));
        let m = if f.engine == Engine::Unity {
            &unity
        } else {
            &unreal
        };
        for c in source::chains(&f.tokens) {
            m.matches(fi, f.engine, &c, &mut occurrences);
        }
    }
    // Attach out-of-class C++ definitions to their classes (by name; the header may be in
    // another file). A definition replaces a body-less declaration of the same name.
    for (class, def) in defs {
        if let Some(c) = raw
            .iter_mut()
            .find(|c| c.name == class && c.engine == Engine::Unreal)
        {
            if let Some(m) = c
                .methods
                .iter_mut()
                .find(|m| m.name == def.name && m.body.is_none())
            {
                *m = source::RawMethod {
                    ret: if def.ret.is_empty() {
                        m.ret.clone()
                    } else {
                        def.ret.clone()
                    },
                    ..def
                };
            } else {
                c.methods.push(def);
            }
        } else {
            raw.push(RawClass {
                name: class,
                engine: Engine::Unreal,
                file: def.file,
                line: def.line,
                base: None,
                fields: Vec::new(),
                methods: vec![def],
                blueprint: false,
            });
        }
    }
    // Resolve roles through inheritance between scanned classes.
    let bases: BTreeMap<String, Option<String>> =
        raw.iter().map(|c| (c.name.clone(), c.base.clone())).collect();
    let mut classes: Vec<Class> = raw
        .into_iter()
        .map(|raw| {
            let t = table(raw.engine);
            let mut engine_base = None;
            let mut role = if raw.blueprint {
                Role::Blueprint
            } else {
                Role::Plain
            };
            let mut cur = raw.base.clone();
            let mut guard = 0;
            while let Some(b) = cur {
                if let Some(e) = t.base(&b) {
                    role = e.role.unwrap_or(Role::Other);
                    engine_base = Some(b);
                    break;
                }
                guard += 1;
                if guard > 32 {
                    break;
                }
                cur = bases.get(&b).cloned().flatten();
            }
            // Unity editor folders hold editor-only code.
            let path = &files[raw.file].path;
            if raw.engine == Engine::Unity && (path.contains("/Editor/") || path.starts_with("Editor/")) {
                role = Role::Editor;
            }
            if raw.blueprint {
                role = Role::Blueprint;
            }
            Class {
                raw,
                role,
                engine_base,
                bound: BTreeMap::new(),
            }
        })
        .collect();
    classes.sort_by(|a, b| {
        (files[a.raw.file].path.as_str(), a.raw.line, &a.raw.name).cmp(&(
            files[b.raw.file].path.as_str(),
            b.raw.line,
            &b.raw.name,
        ))
    });
    // Methods bound by delegates, input bindings, listeners, timers and coroutines: the
    // first argument that names a method of a class in the same file is the handler.
    const BINDERS: [&str; 9] = [
        "AddDynamic",
        "AddUniqueDynamic",
        "BindAxis",
        "BindAction",
        "AddListener",
        "SetTimer",
        "Invoke",
        "InvokeRepeating",
        "StartCoroutine",
    ];
    for o in &occurrences {
        if !BINDERS
            .iter()
            .any(|b| o.name == *b || o.name.ends_with(&format!(".{b}")))
        {
            continue;
        }
        let mut candidates: Vec<String> = o.arg0.iter().cloned().collect();
        for part in o.args.as_deref().unwrap_or("").split(',') {
            let part = part.trim().trim_start_matches('&');
            let part = part.strip_suffix("()").unwrap_or(part);
            let name = part.rsplit("::").next().unwrap_or(part);
            if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                candidates.push(name.to_string());
            }
        }
        let hook = if o.marionette.contains("onCollisionEnter") {
            "onCollisionEnter"
        } else if o.marionette.contains("onCollisionExit") {
            "onCollisionExit"
        } else {
            ""
        };
        for c in classes.iter_mut() {
            if !(c.raw.file == o.file || c.raw.methods.iter().any(|m| m.file == o.file)) {
                continue;
            }
            if let Some(h) = candidates
                .iter()
                .find(|h| c.raw.methods.iter().any(|m| &m.name == *h))
            {
                c.bound
                    .entry(h.clone())
                    .or_insert((o.name.clone(), hook.to_string()));
            }
        }
    }
    Analysis {
        root,
        files,
        classes,
        occurrences,
        enums,
        skipped,
    }
}

fn blueprint_class(fi: usize, f: &SourceFile, occurrences: &mut Vec<Occurrence>) -> RawClass {
    let t = table(Engine::Unreal);
    let text = f.lines.join("\n");
    let name = Path::new(&f.path)
        .file_name()
        .map(|n| {
            n.to_string_lossy()
                .trim_end_matches(".t3d")
                .trim_end_matches(".bp.txt")
                .to_string()
        })
        .unwrap_or_else(|| "Blueprint".into());
    let mut methods = Vec::new();
    let mut seen_hooks = BTreeSet::new();
    for (line, member) in source::blueprint_names(&text) {
        if t.hook(&member).is_some() {
            if seen_hooks.insert(member.clone()) {
                methods.push(source::RawMethod {
                    name: member,
                    line,
                    ret: String::new(),
                    params: String::new(),
                    body: None,
                    file: fi,
                    end_line: line,
                });
            }
            continue;
        }
        let e = t.api.iter().find(|e| e.name == member).or_else(|| {
            t.api
                .iter()
                .find(|e| e.name.rsplit('.').next() == Some(member.as_str()))
        });
        if let Some(e) = e {
            occurrences.push(Occurrence {
                file: fi,
                line,
                token: 0,
                engine: Engine::Unreal,
                name: e.name.clone(),
                status: e.status,
                marionette: e.marionette.clone(),
                note: e.note.clone(),
                arg0: None,
                generic: None,
                args: None,
            });
        }
    }
    RawClass {
        name,
        engine: Engine::Unreal,
        file: fi,
        line: 1,
        base: Some("AActor".into()),
        fields: Vec::new(),
        methods,
        blueprint: true,
    }
}

// ---------------------------------------------------------------------------------------
// Reports

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Location {
    pub file: String,
    pub line: u32,
    /// `Class.Method` the use is in, when known.
    #[serde(rename = "in", skip_serializing_if = "Option::is_none")]
    pub within: Option<String>,
}

/// One engine concept used by the source, with every place it is used (up to a limit).
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ApiUse {
    pub engine: Engine,
    pub name: String,
    pub status: Status,
    pub marionette: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
    pub count: usize,
    /// Distinct string arguments seen (axis, button or action names), at most 8.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub arguments: Vec<String>,
    pub locations: Vec<Location>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct FieldReport {
    pub name: String,
    pub line: u32,
    pub source_type: String,
    /// TypeScript type for `self.state` (or `unknown`).
    pub ts_type: String,
    /// value, entity, component, asset, collection, unknown.
    pub kind: String,
    /// Shown and saved by the source engine's editor (Unity serialized field, Unreal
    /// `UPROPERTY` with an Edit/Visible specifier).
    pub editable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_source: Option<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct MethodReport {
    pub name: String,
    pub file: String,
    pub line: u32,
    pub end_line: u32,
    /// The Marionette hook this method's body goes into, if it is a lifecycle method or
    /// bound to one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hook: Option<String>,
    pub status: Status,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// Unity coroutine (returns IEnumerator): becomes a step counter in `self.state`.
    pub coroutine: bool,
    /// Bound by a delegate, input binding, listener or timer (pattern name).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bound_by: Option<String>,
    pub has_body: bool,
}

/// An engine component the class needs on its entity, from `GetComponent<T>`,
/// `RequireComponent`, `CreateDefaultSubobject<T>` or component-typed fields.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ComponentNeed {
    pub source: String,
    pub marionette: String,
    pub status: Status,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ClassReport {
    pub name: String,
    pub engine: Engine,
    pub file: String,
    pub line: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine_base: Option<String>,
    pub role: Role,
    /// What the class becomes in Marionette.
    pub becomes: String,
    /// Where `port_scaffold` writes its skeleton (relative to the output folder), if it
    /// writes one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script: Option<String>,
    pub fields: Vec<FieldReport>,
    pub methods: Vec<MethodReport>,
    pub components: Vec<ComponentNeed>,
    /// Engine API uses inside this class, by status.
    pub uses: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
pub struct Summary {
    pub files: usize,
    pub classes: usize,
    /// Classes that become scripts (behaviours, actors, components, Blueprints).
    pub scripts: usize,
    /// Engine API uses by status (every occurrence counts).
    pub uses: BTreeMap<String, usize>,
    /// Gaps used most, most first: what Marionette needs to run this code unchanged.
    pub top_gaps: Vec<String>,
    /// Determinism hazards found (must be fixed while porting).
    pub hazards: Vec<String>,
    /// Action names to define in the action map (from Input.GetAxis("Horizontal"),
    /// BindAxis("MoveForward"), ...).
    pub actions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PortReport {
    pub root: String,
    pub engines: Vec<Engine>,
    pub summary: Summary,
    pub classes: Vec<ClassReport>,
    /// Classes not shown because of `offset`/`limit`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_offset: Option<usize>,
    /// Every engine concept used, gaps and hazards first, then by count.
    pub api: Vec<ApiUse>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<Skipped>,
    pub next_steps: Vec<String>,
}

fn status_rank(s: Status) -> u8 {
    match s {
        Status::Hazard => 0,
        Status::Gap => 1,
        Status::Unknown => 2,
        Status::Rewrite => 3,
        Status::Script => 4,
        Status::Ready => 5,
    }
}

impl Analysis {
    /// The (class, method) whose body contains token `token` of file `file`.
    pub fn within(&self, file: usize, token: usize, _line: u32) -> Option<(usize, usize)> {
        if self.files[file].blueprint {
            return None;
        }
        for (ci, c) in self.classes.iter().enumerate() {
            for (mi, m) in c.raw.methods.iter().enumerate() {
                if let (true, Some((a, b))) = (m.file == file, m.body)
                    && (a..=b).contains(&token)
                {
                    return Some((ci, mi));
                }
            }
        }
        None
    }

    pub fn hook_for(&self, c: &Class, method: &str) -> (Option<String>, Status, String, Option<String>) {
        let t = table(c.raw.engine);
        if let Some(h) = t.hook(method) {
            let hook = (!h.marionette.is_empty() && h.marionette.chars().all(|ch| ch.is_alphanumeric()))
                .then(|| h.marionette.clone());
            return (hook, h.status, h.note.clone(), None);
        }
        if let Some((pattern, hook)) = c.bound.get(method) {
            let hook = (!hook.is_empty()).then(|| hook.clone());
            if hook.is_none() && is_timer_binder(pattern) {
                return (
                    None,
                    Status::Script,
                    format!(
                        "Called by a timer ({pattern}): engine.after / engine.every call it as {method}(self, arg)."
                    ),
                    Some(pattern.clone()),
                );
            }
            let note = if hook.is_some() {
                format!("Bound with {pattern}; the body goes into the hook.")
            } else {
                format!("Bound with {pattern}; call it from update when its action or due tick fires.")
            };
            return (hook, Status::Rewrite, note, Some(pattern.clone()));
        }
        (None, Status::Script, String::new(), None)
    }

    pub fn makes_script(c: &Class) -> bool {
        matches!(
            c.role,
            Role::Behaviour | Role::Actor | Role::Component | Role::Blueprint
        )
    }

    pub fn field_report(&self, c: &Class, f: &source::RawField) -> FieldReport {
        let t = table(c.raw.engine);
        let (ts_type, kind, note) = ts_type(c.raw.engine, &f.ty, &self.enums);
        let editable = match c.raw.engine {
            Engine::Unity => {
                let public = f.modifiers.iter().any(|m| m == "public");
                let excluded = f
                    .modifiers
                    .iter()
                    .any(|m| m == "static" || m == "const" || m == "readonly")
                    || f.attributes
                        .iter()
                        .any(|a| a == "NonSerialized" || a == "System.NonSerialized");
                (public || f.attributes.iter().any(|a| a == "SerializeField")) && !excluded
            }
            Engine::Unreal => f
                .attributes
                .iter()
                .any(|a| a.starts_with("Edit") || a.starts_with("Visible") || a == "BlueprintReadWrite"),
        };
        let _ = t;
        let mut default = source::literal(&f.init_tokens);
        let mut note = note;
        // `PlayerMode.Walking` for an enum-typed field becomes the string "Walking".
        if default.is_none()
            && self.enums.contains(&ts_type)
            && let Some(member) = f
                .init
                .as_deref()
                .and_then(|i| i.strip_prefix(&format!("{ts_type}.")))
        {
            default = Some(serde_json::Value::String(member.to_string()));
        }
        // Unity vectors are positions or directions in a left-handed space: flip z (scales
        // and sizes keep their sign).
        let lname = f.name.to_lowercase();
        if c.raw.engine == Engine::Unity
            && f.ty == "Vector3"
            && !lname.contains("scale")
            && !lname.contains("size")
            && let Some(serde_json::Value::Object(m)) = default.as_mut()
            && let Some(z) = m.get("z").and_then(serde_json::Value::as_f64)
        {
            let flipped = if z == 0.0 { 0.0 } else { -z };
            m.insert(
                "z".into(),
                if flipped.fract() == 0.0 {
                    serde_json::json!(flipped as i64)
                } else {
                    serde_json::json!(flipped)
                },
            );
            note = "Default converted from Unity axes (z flipped).".into();
        }
        FieldReport {
            name: f.name.clone(),
            line: f.line,
            source_type: f.ty.clone(),
            ts_type,
            kind,
            editable,
            default_source: if default.is_none() { f.init.clone() } else { None },
            default,
            note,
        }
    }

    pub fn components(&self, ci: usize) -> Vec<ComponentNeed> {
        let c = &self.classes[ci];
        let t = table(c.raw.engine);
        let mut names: BTreeSet<String> = BTreeSet::new();
        for f in &c.raw.fields {
            let base = strip_type(&f.ty);
            if t.component(&base).is_some() {
                names.insert(base);
            }
        }
        for o in &self.occurrences {
            let in_class = self.within(o.file, o.token, o.line).is_some_and(|(k, _)| k == ci)
                || (o.file == c.raw.file && o.name == "RequireComponent");
            if !in_class {
                continue;
            }
            if let Some(g) = &o.generic
                && t.component(g).is_some()
            {
                names.insert(g.clone());
            }
            if let Some(a) = &o.args
                && let Some(inner) = a.strip_prefix("typeof(")
            {
                let n = inner.trim_end_matches(')').to_string();
                if t.component(&n).is_some() {
                    names.insert(n);
                }
            }
        }
        // RequireComponent sits above the class, outside any method.
        for o in self
            .occurrences
            .iter()
            .filter(|o| o.file == c.raw.file && o.name == "RequireComponent")
        {
            for part in o.args.as_deref().unwrap_or("").split("typeof(").skip(1) {
                let n = part.split(')').next().unwrap_or("").trim().to_string();
                if t.component(&n).is_some() {
                    names.insert(n);
                }
            }
        }
        names
            .into_iter()
            .filter_map(|n| {
                t.component(&n).map(|e| ComponentNeed {
                    source: n,
                    marionette: e.marionette.clone(),
                    status: e.status,
                    note: e.note.clone(),
                })
            })
            .collect()
    }

    pub fn class_report(&self, ci: usize) -> ClassReport {
        let c = &self.classes[ci];
        let t = table(c.raw.engine);
        let becomes = match c.engine_base.as_deref().and_then(|b| t.base(b)) {
            Some(e) => e.marionette.clone(),
            None if c.role == Role::Blueprint => {
                "An entity with a TypeScript script (from the Blueprint graph).".into()
            }
            None => "Plain TypeScript (functions or a class) used by the scripts.".into(),
        };
        let mut uses: BTreeMap<String, usize> = BTreeMap::new();
        for o in &self.occurrences {
            if self.within(o.file, o.token, o.line).is_some_and(|(k, _)| k == ci) {
                *uses.entry(o.status.as_str().to_string()).or_default() += 1;
            }
        }
        let methods = c
            .raw
            .methods
            .iter()
            .map(|m| {
                let (hook, status, note, bound_by) = self.hook_for(c, &m.name);
                MethodReport {
                    name: m.name.clone(),
                    file: self.files[m.file].path.clone(),
                    line: m.line,
                    end_line: m.end_line,
                    hook,
                    status,
                    note,
                    coroutine: m.ret == "IEnumerator" || m.ret.starts_with("IEnumerator<"),
                    bound_by,
                    has_body: m.body.is_some() || c.role == Role::Blueprint,
                }
            })
            .collect();
        ClassReport {
            name: c.raw.name.clone(),
            engine: c.raw.engine,
            file: self.files[c.raw.file].path.clone(),
            line: c.raw.line,
            base: c.raw.base.clone(),
            engine_base: c.engine_base.clone(),
            role: c.role,
            becomes,
            script: Self::makes_script(c).then(|| scaffold::script_name(c)),
            fields: c
                .raw
                .fields
                .iter()
                .filter(|f| !f.modifiers.iter().any(|m| m == "const"))
                .map(|f| self.field_report(c, f))
                .collect(),
            methods,
            components: self.components(ci),
            uses,
        }
    }

    /// Builds the report. `offset`/`limit` page the classes; `max_locations` limits the
    /// places listed per API use.
    pub fn report(&self, offset: usize, limit: usize, max_locations: usize) -> PortReport {
        let mut groups: BTreeMap<(Engine, String), ApiUse> = BTreeMap::new();
        let mut summary = Summary {
            files: self.files.len(),
            classes: self.classes.len(),
            ..Default::default()
        };
        summary.scripts = self.classes.iter().filter(|c| Self::makes_script(c)).count();
        let mut actions = BTreeSet::new();
        for o in &self.occurrences {
            *summary.uses.entry(o.status.as_str().to_string()).or_default() += 1;
            let g = groups
                .entry((o.engine, o.name.clone()))
                .or_insert_with(|| ApiUse {
                    engine: o.engine,
                    name: o.name.clone(),
                    status: o.status,
                    marionette: o.marionette.clone(),
                    note: o.note.clone(),
                    count: 0,
                    arguments: Vec::new(),
                    locations: Vec::new(),
                });
            g.count += 1;
            if let Some(a) = &o.arg0 {
                if g.arguments.len() < 8 && !g.arguments.contains(a) {
                    g.arguments.push(a.clone());
                }
                let input = ["Input.", "Bind", "GetInputAxisValue"]
                    .iter()
                    .any(|p| o.name.starts_with(p) || o.name.contains(&format!(".{p}")))
                    || o.name.contains("BindAxis")
                    || o.name.contains("BindAction");
                if input {
                    actions.insert(a.clone());
                }
            }
            if g.locations.len() < max_locations {
                let within = self.within(o.file, o.token, o.line).map(|(ci, mi)| {
                    let c = &self.classes[ci];
                    format!("{}.{}", c.raw.name, c.raw.methods[mi].name)
                });
                g.locations.push(Location {
                    file: self.files[o.file].path.clone(),
                    line: o.line,
                    within,
                });
            }
        }
        let mut api: Vec<ApiUse> = groups.into_values().collect();
        api.sort_by(|a, b| {
            (status_rank(a.status), std::cmp::Reverse(a.count), &a.name).cmp(&(
                status_rank(b.status),
                std::cmp::Reverse(b.count),
                &b.name,
            ))
        });
        summary.top_gaps = api
            .iter()
            .filter(|a| a.status == Status::Gap)
            .take(10)
            .map(|a| format!("{} ({})", a.name, a.count))
            .collect();
        // Hooks that are gaps count too (OnDestroy, EndPlay).
        for c in &self.classes {
            for m in &c.raw.methods {
                if let Some(h) = table(c.raw.engine).hook(&m.name)
                    && h.status == Status::Gap
                    && Self::makes_script(c)
                {
                    let s = format!("hook {} ({}.{})", m.name, c.raw.name, m.name);
                    if summary.top_gaps.len() < 20 {
                        summary.top_gaps.push(s);
                    }
                }
            }
        }
        summary.hazards = api
            .iter()
            .filter(|a| a.status == Status::Hazard)
            .map(|a| format!("{} ({})", a.name, a.count))
            .collect();
        summary.actions = actions.into_iter().collect();
        let mut engines: Vec<Engine> = self
            .files
            .iter()
            .map(|f| f.engine)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        engines.sort();
        let limit = limit.clamp(1, 500);
        let end = (offset + limit).min(self.classes.len());
        let classes = (offset.min(end)..end).map(|ci| self.class_report(ci)).collect();
        let next_offset = (end < self.classes.len()).then_some(end);
        let mut next_steps = vec![
            "Read docs/ai/PORTING.md (the porting recipe) before changing code.".to_string(),
            "Run `port_scaffold` with the same source to write one TypeScript skeleton per gameplay class."
                .to_string(),
        ];
        if !summary.actions.is_empty() {
            next_steps.push(format!(
                "Define these input actions in the scene's action map: {}.",
                summary.actions.join(", ")
            ));
        }
        if !summary.hazards.is_empty() {
            next_steps.push("Fix every hazard while porting: they break the state hash and replays (ARCHITECTURE.md, determinism).".into());
        }
        if !summary.top_gaps.is_empty() {
            next_steps.push("Work around each gap as its note says; list the ones you could not work around in the port notes so engine work can be prioritized.".into());
        }
        next_steps.push("Convert positions, rotations and sizes from the source scene with `port_convert` (axes and units differ).".into());
        PortReport {
            root: self.root.to_string_lossy().replace('\\', "/"),
            engines,
            summary,
            classes,
            next_offset,
            api,
            skipped: self.skipped.clone(),
            next_steps,
        }
    }
}

/// The bare type name: no pointers, references, `const`, `TObjectPtr<>` or namespaces.
pub fn strip_type(ty: &str) -> String {
    let mut s = ty.trim().to_string();
    for w in ["const ", "class ", "struct ", "mutable "] {
        s = s.replace(w, "");
    }
    for wrap in [
        "TObjectPtr<",
        "TWeakObjectPtr<",
        "TSoftObjectPtr<",
        "TStrongObjectPtr<",
        "TSubclassOf<",
    ] {
        if let Some(inner) = s.strip_prefix(wrap) {
            if wrap == "TSubclassOf<" {
                return "TSubclassOf".into();
            }
            s = inner.trim_end_matches('>').to_string();
        }
    }
    s = s.trim_end_matches(['*', '&', ' ']).to_string();
    s.rsplit(['.', ':']).next().unwrap_or(&s).trim().to_string()
}

/// Maps a C# or C++ type to a TypeScript type for `self.state`: (type, kind, note).
pub fn ts_type(engine: Engine, ty: &str, enums: &BTreeSet<String>) -> (String, String, String) {
    let t = table(engine);
    let s = ty.trim().replace("const ", "");
    // Collections.
    for (open, is_map) in [
        ("List<", false),
        ("TArray<", false),
        ("HashSet<", false),
        ("TSet<", false),
        ("Dictionary<", true),
        ("TMap<", true),
    ] {
        if let Some(inner) = s.strip_prefix(open) {
            let inner = inner.strip_suffix('>').unwrap_or(inner);
            if is_map {
                let mut parts = inner.splitn(2, ',');
                let k = ts_type(engine, parts.next().unwrap_or(""), enums).0;
                let v = ts_type(engine, parts.next().unwrap_or(""), enums).0;
                return (
                    format!("Record<string, {v}>"),
                    "collection".into(),
                    format!("Keys become strings ({k}); never let iteration order change game state."),
                );
            }
            let (e, _, _) = ts_type(engine, inner, enums);
            return (format!("{e}[]"), "collection".into(), String::new());
        }
    }
    if let Some(inner) = s.strip_suffix("[]") {
        let (e, _, _) = ts_type(engine, inner, enums);
        return (format!("{e}[]"), "collection".into(), String::new());
    }
    let base = strip_type(&s);
    if let Some(e) = t.ty(&base) {
        let m = e.marionette.as_str();
        let (ts, kind) = if m.starts_with("entity id") {
            ("number".to_string(), "entity")
        } else if m.starts_with("string (") {
            ("string".to_string(), "asset")
        } else if m.starts_with('{') || matches!(m, "number" | "boolean" | "string") {
            (m.to_string(), "value")
        } else {
            ("unknown".to_string(), "unknown")
        };
        let ts = if ts.starts_with('{') && ts.contains("(quaternion)") {
            "{ x: number, y: number, z: number, w: number }".to_string()
        } else {
            ts
        };
        let note = if e.status == Status::Ready || e.note.is_empty() {
            e.note.clone()
        } else {
            format!("{}: {}", e.status.as_str(), e.note)
        };
        let note = if kind == "unknown" && note.is_empty() {
            format!("{}: {}", e.status.as_str(), m)
        } else {
            note
        };
        return (ts, kind.into(), note);
    }
    if let Some(e) = t.component(&base) {
        return (
            String::new(),
            "component".into(),
            format!(
                "A component reference, not state: use self.get(\"{}\") where it is used.",
                e.marionette.split_whitespace().next().unwrap_or("")
            ),
        );
    }
    if enums.contains(&base) {
        return (
            base,
            "value".into(),
            "An enum from the scanned code; port it as a TypeScript enum or string union.".into(),
        );
    }
    (
        "unknown".into(),
        "unknown".into(),
        format!("No mapping for `{s}`; port the type or store an entity id."),
    )
}
