//! The Marionette operation set: one table of typed operations shared by the CLI
//! (`marionette op <name> <json>`) and the MCP server (`marionette mcp`), so the two cannot
//! drift apart.
//!
//! Every operation takes a JSON object and returns a JSON object or [`Errors`] (diagnostics
//! as data). Input and output JSON Schemas are generated from the same Rust types that parse
//! and produce them.

#[cfg(feature = "anim")]
pub mod anim;
#[cfg(feature = "audio")]
pub mod audio;
pub mod batch;
pub mod docs;
pub mod edit;
#[cfg(feature = "editor")]
pub mod editor;
pub mod engine;
#[cfg(feature = "generate")]
pub mod generate;
pub mod handoff;
#[cfg(feature = "jobs-ts")]
pub mod jobs_ts;
#[cfg(feature = "import")]
pub mod library;
pub mod mcp;
#[cfg(feature = "nav")]
pub mod nav;
#[cfg(feature = "netplay")]
pub mod netplay;
pub mod observe;
#[cfg(feature = "package")]
pub mod package;
pub mod perf;
pub mod perf_suite;
#[cfg(feature = "pool")]
pub mod pool;
#[cfg(feature = "port")]
pub mod port;
#[cfg(feature = "import")]
pub mod prefab;
pub mod project;
#[cfg(feature = "generate")]
pub mod providers;
#[cfg(feature = "save")]
pub mod save;
pub mod scenario;
#[cfg(feature = "script")]
pub mod script;
pub mod session;
pub mod skills;
pub mod stability;
#[cfg(feature = "starter")]
pub mod starter;
#[cfg(feature = "audio")]
pub mod synth;
#[cfg(feature = "ui3d")]
mod ui3d;

use marionette_core::{Diagnostic, Errors, Registry};
use marionette_render::Gpu;
use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

pub use engine::{Engine, EngineBuilder};
pub use session::Session;

/// Stable error codes added by this crate.
pub mod codes {
    pub const BAD_INPUT: &str = "agent.bad_input";
    pub const UNKNOWN_OPERATION: &str = "agent.unknown_operation";
    pub const READ_ONLY: &str = "agent.read_only";
    pub const PATH: &str = "agent.path";
    pub const EXISTS: &str = "agent.exists";
    pub const NOT_FOUND: &str = "agent.not_found";
    pub const NO_SESSION: &str = "agent.no_session";
    pub const NO_ENTITY: &str = "agent.no_entity";
    pub const TARGET: &str = "agent.target";
    /// An operation hit an engine bug (a Rust panic). The process keeps serving; a live
    /// session the operation named is closed, because its state may be half-updated.
    pub const PANIC: &str = "agent.panic";
    /// Two modules or two operations of an engine share a name.
    pub const DUPLICATE: &str = "agent.duplicate";
    /// An operation name is not `lower_snake_case`.
    pub const BAD_NAME: &str = "agent.bad_name";
    /// The MCP server was started without this operation (`marionette mcp --tools`).
    pub const NOT_ENABLED: &str = "agent.not_enabled";
}

type Handler = Box<dyn Fn(&mut Agent, Value) -> Result<Value, Errors> + Send + Sync>;

/// Public description of one operation (for `marionette ops` and MCP `tools/list`).
#[derive(Debug, Clone, Serialize)]
pub struct OpInfo {
    pub name: &'static str,
    pub description: &'static str,
    /// Changes files or live sessions. Refused in read-only mode.
    pub mutating: bool,
    pub input_schema: Value,
    pub output_schema: Value,
}

/// One operation: its description, schemas and handler. Make it with [`op`].
pub struct Op {
    info: OpInfo,
    handler: Handler,
}

/// Makes an operation from a handler. `I` and `O` derive `Deserialize`/`Serialize` and
/// `JsonSchema`; their doc comments become the schema descriptions. `mutating` means the
/// operation writes files or changes a session (refused in read-only mode). A game passes its
/// operations to [`EngineBuilder::op`].
pub fn op<I, O>(
    name: &'static str,
    description: &'static str,
    mutating: bool,
    f: fn(&mut Agent, I) -> Result<O, Errors>,
) -> Op
where
    I: DeserializeOwned + JsonSchema + 'static,
    O: Serialize + JsonSchema + 'static,
{
    let handler: Handler = Box::new(move |agent, input| {
        let input = if input.is_null() {
            Value::Object(Default::default())
        } else {
            input
        };
        let parsed: I = serde_json::from_value(input).map_err(|e| {
            Errors::from(
                Diagnostic::error(codes::BAD_INPUT, format!("invalid input for `{name}`: {e}")).fix(format!(
                    "see the input schema of `{name}` (`marionette ops` or MCP tools/list)"
                )),
            )
        })?;
        let out = f(agent, parsed)?;
        Ok(serde_json::to_value(out).expect("operation outputs serialize"))
    });
    Op {
        info: OpInfo {
            name,
            description,
            mutating,
            input_schema: schemars::schema_for!(I).to_value(),
            output_schema: schemars::schema_for!(O).to_value(),
        },
        handler,
    }
}

/// The optional engine parts (cargo features) compiled into this build (decision 0062).
pub const FEATURES: &[&str] = &[
    #[cfg(feature = "physics")]
    "physics",
    #[cfg(feature = "audio")]
    "audio",
    #[cfg(feature = "anim")]
    "anim",
    #[cfg(feature = "xr")]
    "xr",
    #[cfg(feature = "script")]
    "script",
    #[cfg(feature = "import")]
    "import",
    #[cfg(feature = "pool")]
    "pool",
    #[cfg(feature = "generate")]
    "generate",
    #[cfg(feature = "port")]
    "port",
    #[cfg(feature = "jobs-ts")]
    "jobs-ts",
    #[cfg(feature = "package")]
    "package",
    #[cfg(feature = "save")]
    "save",
    #[cfg(feature = "starter")]
    "starter",
    #[cfg(feature = "ui3d")]
    "ui3d",
    #[cfg(feature = "nav")]
    "nav",
    #[cfg(feature = "net")]
    "net",
    #[cfg(feature = "editor")]
    "editor",
    #[cfg(feature = "netplay")]
    "netplay",
];

/// Every built-in operation compiled into this build, in a stable order.
pub(crate) fn builtin_ops() -> Vec<Op> {
    let groups: Vec<Vec<Op>> = vec![
        project::ops(),
        edit::ops(),
        session::ops(),
        observe::ops(),
        #[cfg(feature = "editor")]
        editor::ops(),
        scenario::ops(),
        #[cfg(feature = "import")]
        library::ops(),
        perf::ops(),
        perf_suite::ops(),
        #[cfg(feature = "jobs-ts")]
        jobs_ts::ops(),
        #[cfg(feature = "generate")]
        providers::ops(),
        #[cfg(feature = "anim")]
        anim::ops(),
        #[cfg(feature = "ui3d")]
        ui3d::ops(),
        #[cfg(feature = "audio")]
        audio::ops(),
        #[cfg(feature = "nav")]
        nav::ops(),
        #[cfg(feature = "audio")]
        synth::ops(),
        #[cfg(feature = "generate")]
        generate::ops(),
        #[cfg(feature = "script")]
        script::ops(),
        stability::ops(),
        #[cfg(feature = "pool")]
        pool::ops(),
        #[cfg(feature = "port")]
        port::ops(),
        #[cfg(feature = "save")]
        save::ops(),
        #[cfg(feature = "import")]
        prefab::ops(),
        #[cfg(feature = "package")]
        package::ops(),
        #[cfg(feature = "starter")]
        starter::ops(),
        #[cfg(feature = "netplay")]
        netplay::ops(),
        batch::ops(),
        skills::ops(),
        handoff::ops(),
    ];
    groups.into_iter().flatten().collect()
}

/// Holds the project root, live sessions and (lazily) the GPU between calls.
pub struct Agent {
    root: PathBuf,
    read_only: bool,
    engine: Engine,
    ops: std::sync::Arc<Vec<Op>>,
    pub(crate) sessions: BTreeMap<String, Session>,
    pub(crate) next_session: u64,
    gpu: Option<Gpu>,
}

impl Agent {
    /// `root` is the project directory; relative paths in operations resolve against it.
    pub fn new(root: impl Into<PathBuf>, read_only: bool) -> Self {
        Self::with_engine(root, read_only, Engine::standard())
    }

    /// An agent running a game's engine (its own modules and operations added).
    pub fn with_engine(root: impl Into<PathBuf>, read_only: bool, engine: Engine) -> Self {
        Self {
            root: root.into(),
            read_only,
            ops: engine.ops(),
            engine,
            sessions: BTreeMap::new(),
            next_session: 1,
            gpu: None,
        }
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    pub fn read_only(&self) -> bool {
        self.read_only
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn registry(&self) -> &Registry {
        self.engine.registry()
    }

    /// Loads a scene (path relative to the project root) with this agent's engine.
    pub fn load_sim(&self, scene: &str, seed: u64) -> Result<marionette_core::Sim, Errors> {
        let path = self.resolve(scene)?;
        self.engine.load_file(&path, scene, seed)
    }

    /// The simulation of a live session (for a game's own operations).
    pub fn session_sim(&mut self, session: &str) -> Result<&mut marionette_core::Sim, Errors> {
        session::get_mut(self, session).map(|s| &mut s.sim)
    }

    /// Writes a project file atomically. Refuses files only a person may write.
    pub fn write_file(&self, path: &str, bytes: &[u8]) -> Result<(), Errors> {
        if self.read_only {
            return Err(
                Diagnostic::error(codes::READ_ONLY, format!("cannot write `{path}`: read-only"))
                    .fix("restart without --read-only")
                    .into(),
            );
        }
        write_atomic(&self.resolve(path)?, path, bytes)
    }

    /// Every operation, in a stable order.
    pub fn operations(&self) -> impl Iterator<Item = &OpInfo> {
        self.ops.iter().map(|o| &o.info)
    }

    /// Runs one operation. Errors are data; this never panics on bad input.
    pub fn call(&mut self, name: &str, input: Value) -> Result<Value, Errors> {
        let Some(index) = self.ops.iter().position(|o| o.info.name == name) else {
            let names: Vec<&str> = self.ops.iter().map(|o| o.info.name).collect();
            return Err(
                Diagnostic::error(codes::UNKNOWN_OPERATION, format!("unknown operation `{name}`"))
                    .fix(format!("operations: {}", names.join(", ")))
                    .into(),
            );
        };
        if self.read_only && self.ops[index].info.mutating {
            return Err(Diagnostic::error(
                codes::READ_ONLY,
                format!("`{name}` changes files or sessions and this server is read-only"),
            )
            .fix("use observe_*, scene_validate, scene_run or test_run, or restart without --read-only")
            .into());
        }
        // A shared handle to the table lets the handler borrow the agent mutably (and lets
        // `batch_run` call other operations).
        let ops = std::sync::Arc::clone(&self.ops);
        let session = input.get("session").and_then(Value::as_str).map(str::to_string);
        // A panic is an engine bug, but it must not take down the process (an MCP server
        // serves many calls); decision 0032.
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (ops[index].handler)(self, input)));
        result.unwrap_or_else(|payload| {
            let msg = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown panic".into());
            let closed = session.filter(|id| self.sessions.remove(id).is_some());
            let mut d = Diagnostic::error(codes::PANIC, format!("`{name}` hit an engine bug: {msg}"));
            d = match closed {
                Some(id) => d.fix(format!(
                    "session `{id}` was closed because its state may be broken; start it again and report the bug"
                )),
                None => d.fix("report the bug with the operation and its input; the server keeps running"),
            };
            Err(d.into())
        })
    }

    /// Resolves a path from an operation. Relative paths are relative to the project root and
    /// may not climb out of it; absolute paths are used as given.
    pub fn resolve(&self, path: &str) -> Result<PathBuf, Errors> {
        let p = Path::new(path);
        if p.is_absolute() {
            return Ok(p.to_path_buf());
        }
        if p.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::Prefix(_) | Component::RootDir
            )
        }) {
            return Err(
                Diagnostic::error(codes::PATH, format!("path `{path}` leaves the project"))
                    .fix("use a path inside the project, relative to its root")
                    .into(),
            );
        }
        Ok(self.root.join(p))
    }

    pub(crate) fn gpu(&mut self) -> Result<&Gpu, Errors> {
        if self.gpu.is_none() {
            self.gpu = Some(Gpu::headless()?);
        }
        Ok(self.gpu.as_ref().expect("just set"))
    }
}

/// Reads a text file, with a diagnostic on failure.
pub(crate) fn read_text(path: &Path, shown: &str) -> Result<String, Errors> {
    std::fs::read_to_string(path).map_err(|e| {
        Errors::from(
            Diagnostic::error(
                marionette_core::error::codes::IO_READ,
                format!("cannot read {shown}: {e}"),
            )
            .file(shown)
            .fix("check the path exists and is readable (relative paths start at the project root)"),
        )
    })
}

/// Writes a file atomically (temporary file, then rename) so a crash never leaves half a
/// scene on disk.
pub(crate) fn write_atomic(path: &Path, shown: &str, bytes: &[u8]) -> Result<(), Errors> {
    let err = |e: std::io::Error| {
        Errors::from(
            Diagnostic::error(
                marionette_core::error::codes::IO_WRITE,
                format!("cannot write {shown}: {e}"),
            )
            .file(shown),
        )
    };
    // Only a person may write a license declaration (ASSETS-AND-LICENSES.md rule 9) or
    // approve an image for paid 3D generation (decision 0045).
    if let Some(refusal) = path
        .file_name()
        .and_then(|n| marionette_core::person_only::refuse_write(&n.to_string_lossy(), shown))
    {
        return Err(refusal.into());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(err)?;
    }
    let tmp = path.with_extension("tmp-write");
    std::fs::write(&tmp, bytes).map_err(err)?;
    std::fs::rename(&tmp, path).map_err(err)
}

/// Reads an environment variable, falling back to the person's keys file (outside the
/// project) for provider keys and the spend cap (decision 0045).
#[cfg(feature = "generate")]
pub(crate) fn env_lookup(root: &Path) -> impl Fn(&str) -> Option<String> + use<> {
    let root = root.to_path_buf();
    move |name| marionette_providers::keys::lookup(&|n| std::env::var(n).ok(), &root, name)
}

/// SHA-256 as lowercase hex.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Paging defaults for list results (keeps tool results small).
pub(crate) const DEFAULT_LIMIT: usize = 50;
pub(crate) const MAX_LIMIT: usize = 500;

pub(crate) fn page<T: Clone>(items: &[T], offset: usize, limit: Option<usize>) -> (Vec<T>, Option<usize>) {
    let limit = limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let end = (offset + limit).min(items.len());
    let slice = if offset < items.len() {
        items[offset..end].to_vec()
    } else {
        Vec::new()
    };
    (slice, (end < items.len()).then_some(end))
}

#[cfg(test)]
mod panic_tests {
    use super::*;
    use serde_json::json;

    fn boom(_: &mut Agent, _: session::SessionInput) -> Result<session::StopOutput, Errors> {
        panic!("deliberate test panic");
    }

    #[test]
    fn a_panicking_operation_is_an_error_and_the_agent_keeps_serving() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        let engine = Engine::builder()
            .op(op("boom", "test only", true, boom))
            .build()
            .unwrap();
        let mut agent = Agent::with_engine(root, false, engine);
        let started = agent
            .call("session_start", json!({ "scene": "drift.scene.json", "seed": 1 }))
            .unwrap();
        let id = started["session"].as_str().unwrap().to_string();
        let err = agent.call("boom", json!({ "session": id })).unwrap_err();
        let text = serde_json::to_string(&err).unwrap();
        assert!(
            text.contains(codes::PANIC) && text.contains("deliberate test panic"),
            "{text}"
        );
        assert!(
            agent.sessions.is_empty(),
            "the possibly broken session was closed"
        );
        // The server keeps working.
        assert!(agent.call("session_list", json!({})).is_ok());
        assert!(
            agent
                .call("scene_validate", json!({ "scene": "drift.scene.json" }))
                .is_ok()
        );
    }
}
