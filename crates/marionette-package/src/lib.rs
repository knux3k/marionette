//! Game packaging (decision 0140): one call turns a game folder into a folder a player can run,
//! plus a zip of it, for each target:
//!
//! | Target | Rust target | Notes |
//! | --- | --- | --- |
//! | `windows-x64` | `x86_64-pc-windows-msvc` | built on a Windows machine |
//! | `linux-x64` | `x86_64-unknown-linux-gnu` | needs glibc 2.31 or newer |
//! | `steamdeck` | `x86_64-unknown-linux-gnu` | same binary as `linux-x64`; Steam Linux Runtime 3.0 (sniper) |
//! | `steam-frame` | `aarch64-unknown-linux-gnu` | Linux ARM64; Steam Linux Runtime 3.0 ARM64 (sniper) |
//!
//! A package holds the game binary (named after the game), the game's runtime files (scenes,
//! scripts, assets), `game.package.json` (entry scene, window, per-file SHA-256), the engine
//! licenses and the licenses of every Rust crate linked in. Started with no arguments, the
//! binary finds `game.package.json` beside itself and plays the entry scene
//! (`marionette_cli::main_with`, which reads the manifest without this crate).
//!
//! The binary is built with cargo, run through `marionette_host::spawn`. Linux targets are
//! held to glibc 2.31 (the Steam Linux Runtime 3.0 floor): when the build machine's glibc is
//! newer, or the target is another architecture, the build goes through `cargo zigbuild`.
//! Every Linux binary is checked afterwards: the newest `GLIBC_` symbol version it asks for
//! must not pass the floor.

use marionette_core::{Diagnostic, Errors};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

mod files;
mod licenses;
pub mod zip;

pub use files::{DEFAULT_EXCLUDES, PackageFile, glob_match};

/// Stable error codes of this crate.
pub mod codes {
    pub const UNKNOWN_TARGET: &str = "package.unknown_target";
    pub const GAME: &str = "package.game";
    pub const TOOLCHAIN: &str = "package.toolchain";
    pub const BUILD: &str = "package.build";
    pub const GLIBC: &str = "package.glibc";
    pub const SMOKE: &str = "package.smoke";
    pub const IO: &str = "package.io";
}

/// The file beside a packaged binary that says what to play.
pub const MANIFEST: &str = "game.package.json";
/// `format` field of [`MANIFEST`].
pub const FORMAT: &str = "marionette.package";
/// Oldest glibc a Linux package may need: Steam Linux Runtime 3.0 (sniper) ships 2.31.
pub const GLIBC_FLOOR: (u32, u32) = (2, 31);
/// Engine features a packaged game may get when it has no binary of its own: gamepads always,
/// and saving always, scripts, job TypeScript and OpenXR only when the game uses them ([`detect_features`]). Import, pool,
/// generation, porting and packaging are editor tools and stay out.
pub const RUNTIME_FEATURES: &[&str] = &["gamepad", "save", "script", "jobs-ts", "xr"];

/// The runtime features a game's files use: `gamepad` always (Deck and Frame players hold one,
/// decision 0172), `save` always (autosave, F5 and F9; decision 0076), `script` when it has TypeScript files,
/// `jobs-ts` when any text names `JobData`, `JobSchedule` or `engine.jobs` (job TypeScript
/// brings wasmtime, about 12 MB of binary), `xr` when a scene has an `XrRig` (decision 0190).
pub fn detect_features(dir: &Path, files: &[String]) -> Vec<String> {
    let mut out = vec!["gamepad".to_string(), "save".to_string()];
    if files.iter().any(|f| f.ends_with(".ts")) {
        out.push("script".to_string());
    }
    let uses_jobs = files.iter().filter(|f| files::is_text(f)).any(|f| {
        std::fs::read_to_string(dir.join(f))
            .is_ok_and(|t| t.contains("JobData") || t.contains("JobSchedule") || t.contains("engine.jobs"))
    });
    if uses_jobs {
        out.push("jobs-ts".to_string());
    }
    let uses_xr = files
        .iter()
        .filter(|f| f.ends_with(".scene.json"))
        .any(|f| std::fs::read_to_string(dir.join(f)).is_ok_and(|t| t.contains("\"XrRig\"")));
    if uses_xr {
        out.push("xr".to_string());
    }
    out
}

/// One package target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Target {
    /// Name used on the command line.
    pub name: &'static str,
    /// Rust target triple.
    pub triple: &'static str,
    /// Linux (glibc) or Windows.
    pub linux: bool,
    /// Default window size when the game does not set one.
    pub window: &'static str,
    /// One line on what it is for.
    pub summary: &'static str,
}

/// Every target, in the order `all` builds them.
pub const TARGETS: &[Target] = &[
    Target {
        name: "windows-x64",
        triple: "x86_64-pc-windows-msvc",
        linux: false,
        window: "1280x720",
        summary: "Windows 10 or 11 on x86-64; build on a Windows machine",
    },
    Target {
        name: "linux-x64",
        triple: "x86_64-unknown-linux-gnu",
        linux: true,
        window: "1280x720",
        summary: "Linux on x86-64 with glibc 2.31 or newer and a Vulkan driver",
    },
    Target {
        name: "steamdeck",
        triple: "x86_64-unknown-linux-gnu",
        linux: true,
        window: "1280x800",
        summary: "Steam Deck (SteamOS, x86-64); runs in Steam Linux Runtime 3.0 (sniper)",
    },
    Target {
        name: "steam-frame",
        triple: "aarch64-unknown-linux-gnu",
        linux: true,
        window: "1920x1080",
        summary: "Steam Frame (SteamOS on Linux ARM64); runs in Steam Linux Runtime 3.0 ARM64 (sniper); VR through OpenXR when the game has an XrRig, else a flat window",
    },
];

/// Looks a target up by name; `all` is handled by [`targets`].
pub fn target(name: &str) -> Result<Target, Errors> {
    TARGETS.iter().copied().find(|t| t.name == name).ok_or_else(|| {
        Diagnostic::error(codes::UNKNOWN_TARGET, format!("unknown package target `{name}`"))
            .fix(format!(
                "targets: {}, or all",
                TARGETS.iter().map(|t| t.name).collect::<Vec<_>>().join(", ")
            ))
            .into()
    })
}

/// Resolves target names (`all` = every target this machine can build, see [`can_build`]).
pub fn targets(names: &[String]) -> Result<Vec<Target>, Errors> {
    let mut out = Vec::new();
    for name in names {
        if name == "all" {
            out.extend(TARGETS.iter().copied().filter(|t| can_build(t).is_ok()));
        } else {
            out.push(target(name)?);
        }
    }
    out.dedup_by_key(|t| t.name);
    Ok(out)
}

/// The Rust target triple of the machine this code runs on (the build machine).
pub fn host_triple() -> &'static str {
    if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        "x86_64-pc-windows-msvc"
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "x86_64-unknown-linux-gnu"
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        "aarch64-unknown-linux-gnu"
    } else {
        "unsupported"
    }
}

/// How a target's binary is built on this machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Builder {
    /// `cargo build --target <triple>`.
    Cargo,
    /// `cargo zigbuild --target <triple>.2.31` (glibc floor, cross-architecture).
    Zigbuild,
}

/// Picks the builder for `target` on this machine, or says why it cannot be built here.
pub fn can_build(target: &Target) -> Result<Builder, Errors> {
    let host = host_triple();
    if !target.linux {
        return if host == target.triple {
            Ok(Builder::Cargo)
        } else {
            Err(Diagnostic::error(
                codes::TOOLCHAIN,
                format!(
                    "`{}` packages are built on Windows (this machine is {host})",
                    target.name
                ),
            )
            .fix("run the same command on a Windows machine, or take the zip from the CI job `package`")
            .into())
        };
    }
    let native_ok = host == target.triple && host_glibc().is_some_and(|v| v <= GLIBC_FLOOR);
    if native_ok {
        return Ok(Builder::Cargo);
    }
    if has_zigbuild() {
        return Ok(Builder::Zigbuild);
    }
    let why = if host == target.triple {
        format!(
            "this machine's glibc ({}) is newer than the {}.{} floor, so a plain build would not run on older systems or in the Steam runtime",
            host_glibc()
                .map(|(a, b)| format!("{a}.{b}"))
                .unwrap_or_else(|| "unknown".into()),
            GLIBC_FLOOR.0,
            GLIBC_FLOOR.1
        )
    } else {
        format!(
            "`{}` is another architecture than this machine ({host})",
            target.triple
        )
    };
    Err(Diagnostic::error(codes::TOOLCHAIN, format!("cannot build `{}` here: {why}", target.name))
        .fix(format!(
            "install zig and cargo-zigbuild (`pip install ziglang`, `cargo install cargo-zigbuild --locked`) and `rustup target add {}` (docs/ai/PACKAGING.md)",
            target.triple
        ))
        .into())
}

fn has_zigbuild() -> bool {
    let mut cmd = Command::new("cargo");
    cmd.args(["zigbuild", "--help"]);
    marionette_host::output(cmd).is_ok_and(|o| o.status.success())
}

/// glibc version of this machine (`ldd --version`), Linux only.
pub fn host_glibc() -> Option<(u32, u32)> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let mut cmd = Command::new("ldd");
    cmd.arg("--version");
    let out = marionette_host::output(cmd).ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let first = text.lines().next()?;
    parse_version(first.split_whitespace().last()?)
}

fn parse_version(s: &str) -> Option<(u32, u32)> {
    let mut it = s.split('.');
    let major = it.next()?.trim().parse().ok()?;
    let minor = it.next()?.trim().parse().ok()?;
    Some((major, minor))
}

/// The newest `GLIBC_x.y` symbol version an ELF binary asks for (from its version-needed
/// strings), or `None` if it names none.
pub fn glibc_required(binary: &[u8]) -> Option<(u32, u32)> {
    const TAG: &[u8] = b"GLIBC_";
    let mut best = None;
    let mut i = 0;
    while i + TAG.len() < binary.len() {
        if &binary[i..i + TAG.len()] == TAG {
            let rest = &binary[i + TAG.len()..];
            let end = rest
                .iter()
                .position(|b| !(b.is_ascii_digit() || *b == b'.'))
                .unwrap_or(rest.len());
            if let Some(v) = std::str::from_utf8(&rest[..end]).ok().and_then(parse_version) {
                best = best.max(Some(v));
            }
            i += TAG.len() + end;
        } else {
            i += 1;
        }
    }
    best
}

/// Package settings a game may give in its `marionette.json` under `"package"`.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GameSettings {
    /// Scene to play, relative to the game folder (default `scenes/main.scene.json`).
    #[serde(default)]
    pub entry: Option<String>,
    /// Window title (default: the game name).
    #[serde(default)]
    pub title: Option<String>,
    /// Window size, e.g. `1280x800` (default: per target).
    #[serde(default)]
    pub window: Option<String>,
    /// More paths to leave out, as globs relative to the game folder (`*`, `**`).
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Engine features for the default binary (default: what the game's files use, see
    /// [`detect_features`]); ignored when the game has its own `Cargo.toml`.
    #[serde(default)]
    pub features: Option<Vec<String>>,
    /// Keep files under `assets/` that no scene, script or other text file names (default:
    /// leave them out and list them in `left_out`).
    #[serde(default)]
    pub keep_unreferenced: bool,
}

/// What to package.
#[derive(Debug, Clone)]
pub struct Options {
    /// Directory cargo runs in: the engine workspace (or any workspace that has
    /// `marionette-cli`).
    pub workspace: PathBuf,
    /// The game folder (holds `marionette.json`).
    pub game: PathBuf,
    /// Output folder; each package becomes `<out>/<game>-<target>/` and `<out>/<game>-<target>.zip`.
    pub out: PathBuf,
    pub targets: Vec<Target>,
    /// Release (default) or debug build.
    pub release: bool,
    /// Write the zip next to the folder.
    pub archive: bool,
    /// Start the packaged binary headless for a few ticks when this machine can run it.
    pub smoke: bool,
    /// Link-time sysroots per architecture (`x86_64`, `aarch64`) holding the glibc 2.31
    /// release's `libasound.so` and `alsa.pc`, used by zigbuild builds (default:
    /// `MARIONETTE_SYSROOT_X86_64`, `MARIONETTE_SYSROOT_AARCH64`; made by
    /// `scripts/package-sysroots.sh`).
    pub sysroots: BTreeMap<String, PathBuf>,
}

/// A game folder read for packaging.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Game {
    pub name: String,
    pub version: Option<u64>,
    pub settings: GameSettings,
    /// The game's own Rust package (from its `Cargo.toml`), or `None` for the engine binary.
    pub rust_package: Option<String>,
    /// Runtime files, relative to the game folder, sorted.
    pub files: Vec<String>,
    /// Files under `assets/` left out because nothing names them.
    pub left_out: Vec<String>,
    pub entry: String,
    /// Engine features of the default binary (empty for a game with its own binary).
    pub features: Vec<String>,
}

/// Reads a game folder: name, settings, runtime files.
pub fn read_game(dir: &Path) -> Result<Game, Errors> {
    let project = dir.join("marionette.json");
    let text = std::fs::read_to_string(&project).map_err(|e| {
        Errors::from(
            Diagnostic::error(codes::GAME, format!("no game here: {e}"))
                .file(project.display().to_string())
                .fix("point at a game folder: one that holds marionette.json (e.g. games/cloudstep)"),
        )
    })?;
    let v: Value = serde_json::from_str(&text).map_err(|e| {
        Errors::from(
            Diagnostic::error(codes::GAME, format!("marionette.json: {e}"))
                .file(project.display().to_string()),
        )
    })?;
    let fallback = dir
        .canonicalize()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "game".into());
    let name = v["name"].as_str().map(str::to_string).unwrap_or(fallback);
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(
            Diagnostic::error(codes::GAME, format!("game name `{name}` cannot name a binary"))
                .file(project.display().to_string())
                .fix("use letters, digits, `-` or `_` in marionette.json `name`")
                .into(),
        );
    }
    let settings: GameSettings = match v.get("package") {
        Some(p) => serde_json::from_value(p.clone()).map_err(|e| {
            Errors::from(
                Diagnostic::error(codes::GAME, format!("marionette.json `package`: {e}"))
                    .file(project.display().to_string()),
            )
        })?,
        None => GameSettings::default(),
    };
    let rust_package = if dir.join("Cargo.toml").is_file() {
        Some(cargo_package_name(dir)?)
    } else {
        None
    };
    let mut files = files::runtime_files(dir, &settings.exclude, rust_package.is_some())?;
    let left_out = if settings.keep_unreferenced {
        Vec::new()
    } else {
        files::drop_unreferenced(dir, &mut files)
    };
    let features = match (&rust_package, &settings.features) {
        (Some(_), _) => Vec::new(),
        (None, Some(f)) => f.clone(),
        (None, None) => detect_features(dir, &files),
    };
    let entry = settings
        .entry
        .clone()
        .unwrap_or_else(|| "scenes/main.scene.json".into());
    if !files.contains(&entry) {
        return Err(Diagnostic::error(
            codes::GAME,
            format!("entry scene `{entry}` is not among the game's files"),
        )
        .file(project.display().to_string())
        .fix("set marionette.json `package.entry` to the scene to play, relative to the game folder")
        .into());
    }
    Ok(Game {
        name,
        version: v["version"].as_u64(),
        settings,
        rust_package,
        files,
        left_out,
        entry,
        features,
    })
}

fn cargo_package_name(dir: &Path) -> Result<String, Errors> {
    let mut cmd = Command::new("cargo");
    cmd.args([
        "metadata",
        "--no-deps",
        "--format-version",
        "1",
        "--manifest-path",
    ])
    .arg(dir.join("Cargo.toml"));
    let out = marionette_host::output(cmd).map_err(|e| toolchain(format!("cannot run cargo: {e}")))?;
    if !out.status.success() {
        return Err(build_error("cargo metadata", &out.stderr));
    }
    let v: Value = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
    let wanted = dir.join("Cargo.toml").canonicalize().ok();
    v["packages"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|p| {
            p["manifest_path"]
                .as_str()
                .map(|m| Path::new(m).canonicalize().ok())
                == Some(wanted.clone())
        })
        .and_then(|p| p["name"].as_str())
        .map(str::to_string)
        .ok_or_else(|| toolchain("cargo metadata did not list the game's package".into()))
}

fn toolchain(msg: String) -> Errors {
    Diagnostic::error(codes::TOOLCHAIN, msg)
        .fix("install Rust with rustup (docs/ai/ENVIRONMENT.md)")
        .into()
}

fn build_error(what: &str, stderr: &[u8]) -> Errors {
    let text = String::from_utf8_lossy(stderr);
    // The last lines carry the error; keep the result small (focus 2).
    let tail: Vec<&str> = text.lines().rev().take(12).collect();
    let tail: Vec<&str> = tail.into_iter().rev().collect();
    Diagnostic::error(codes::BUILD, format!("{what} failed:\n{}", tail.join("\n")))
        .fix("fix the build error above; rerun with the same input")
        .into()
}

/// The result for one target.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Built {
    pub target: String,
    /// Package folder.
    pub dir: String,
    /// Zip next to it, if written.
    pub zip: Option<String>,
    pub binary: String,
    pub builder: Builder,
    pub files: usize,
    pub bytes: u64,
    /// Newest glibc version the binary needs (Linux).
    pub glibc_required: Option<String>,
    /// `passed`, or why the smoke run was skipped.
    pub smoke: String,
    /// Third-party crates whose licenses were bundled.
    pub crates: usize,
    /// Things the package may miss (for example the OpenXR loader on Windows).
    pub warnings: Vec<String>,
}

/// A compiled binary, kept so two targets with the same triple build once.
#[derive(Debug, Clone)]
struct Binary {
    path: PathBuf,
    builder: Builder,
}

/// Builds every requested target.
pub fn package(opts: &Options) -> Result<Vec<Built>, Errors> {
    let game = read_game(&opts.game)?;
    let mut binaries: BTreeMap<&'static str, Binary> = BTreeMap::new();
    let mut out = Vec::new();
    for target in &opts.targets {
        let builder = can_build(target)?;
        let binary = match binaries.get(target.triple) {
            Some(b) => b.clone(),
            None => {
                let b = Binary {
                    path: build_binary(opts, &game, target, &builder)?,
                    builder,
                };
                binaries.insert(target.triple, b.clone());
                b
            }
        };
        out.push(assemble(opts, &game, target, &binary)?);
    }
    Ok(out)
}

/// Cargo arguments that select the game's binary (shared by build and license listing).
fn select_args(opts: &Options, game: &Game) -> Vec<String> {
    match &game.rust_package {
        Some(pkg) => vec![
            "--manifest-path".into(),
            opts.game.join("Cargo.toml").display().to_string(),
            "-p".into(),
            pkg.clone(),
        ],
        None => vec![
            "-p".into(),
            "marionette-cli".into(),
            "--no-default-features".into(),
            "--features".into(),
            game.features.join(","),
        ],
    }
}

fn build_binary(opts: &Options, game: &Game, target: &Target, builder: &Builder) -> Result<PathBuf, Errors> {
    let mut cmd = Command::new("cargo");
    cmd.current_dir(&opts.workspace);
    let triple = match builder {
        Builder::Cargo => {
            cmd.arg("build");
            target.triple.to_string()
        }
        Builder::Zigbuild => {
            cmd.arg("zigbuild");
            format!("{}.{}.{}", target.triple, GLIBC_FLOOR.0, GLIBC_FLOOR.1)
        }
    };
    cmd.args([
        "--locked",
        "--message-format=json-render-diagnostics",
        "--target",
        &triple,
    ]);
    if opts.release {
        // Shipped games use the workspace's `dist` profile (fat LTO, decision 0101) when it
        // has one, else release.
        if has_dist_profile(&opts.workspace) {
            cmd.args(["--profile", "dist"]);
        } else {
            cmd.arg("--release");
        }
    }
    cmd.args(select_args(opts, game));
    if *builder == Builder::Zigbuild {
        // The build machine's own libasound may need a newer glibc (or be another
        // architecture): link against the target's copy from the floor release.
        let arch = target.triple.split('-').next().unwrap_or_default();
        let sysroot = sysroot(opts, arch)?;
        let pc = sysroot.join(format!("usr/lib/{arch}-linux-gnu/pkgconfig"));
        cmd.env("PKG_CONFIG_ALLOW_CROSS", "1")
            .env("PKG_CONFIG_SYSROOT_DIR", &sysroot)
            .env("PKG_CONFIG_LIBDIR", &pc);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = marionette_host::spawn(cmd).map_err(|e| toolchain(format!("cannot run cargo: {e}")))?;
    let out = child
        .wait_with_output()
        .map_err(|e| toolchain(format!("cargo did not finish: {e}")))?;
    if !out.status.success() {
        return Err(build_error(
            &format!("cargo build for {}", target.name),
            &out.stderr,
        ));
    }
    let want_bin = if game.rust_package.is_some() {
        None
    } else {
        Some("marionette")
    };
    let mut exe = None;
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let Ok(msg) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if msg["reason"] != "compiler-artifact" {
            continue;
        }
        let Some(path) = msg["executable"].as_str() else {
            continue;
        };
        let is_bin = msg["target"]["kind"]
            .as_array()
            .is_some_and(|k| k.iter().any(|k| k == "bin"));
        // `-p` builds only the selected package's binaries; dependencies add no `bin`.
        let name_ok = want_bin.is_none_or(|w| msg["target"]["name"] == w);
        if is_bin && name_ok {
            if exe.is_some() {
                return Err(
                    Diagnostic::error(codes::BUILD, "the game's package has more than one binary")
                        .fix("give the game's Cargo.toml exactly one [[bin]] (see examples/rust-module)")
                        .into(),
                );
            }
            exe = Some(PathBuf::from(path));
        }
    }
    exe.ok_or_else(|| {
        Diagnostic::error(codes::BUILD, "cargo built no binary for the game")
            .fix("give the game's Cargo.toml exactly one [[bin]] (see examples/rust-module)")
            .into()
    })
}

/// Whether the workspace manifest defines `[profile.dist]`.
pub fn has_dist_profile(workspace: &Path) -> bool {
    std::fs::read_to_string(workspace.join("Cargo.toml"))
        .is_ok_and(|t| t.lines().any(|l| l.trim() == "[profile.dist]"))
}

/// Link-time sysroot for `arch` (`x86_64` or `aarch64`): the option, else
/// `MARIONETTE_SYSROOT_X86_64` / `MARIONETTE_SYSROOT_AARCH64`.
fn sysroot(opts: &Options, arch: &str) -> Result<PathBuf, Errors> {
    let var = format!("MARIONETTE_SYSROOT_{}", arch.to_ascii_uppercase());
    let dir = opts
        .sysroots
        .get(arch)
        .cloned()
        .or_else(|| std::env::var_os(&var).map(PathBuf::from));
    let pc = |d: &PathBuf, lib: &str| d.join(format!("usr/lib/{arch}-linux-gnu/pkgconfig/{lib}.pc"));
    match dir {
        Some(d) if pc(&d, "alsa").is_file() && pc(&d, "libudev").is_file() => Ok(d),
        found => Err(Diagnostic::error(
            codes::TOOLCHAIN,
            match found {
                Some(d) => format!(
                    "sysroot `{}` lacks usr/lib/{arch}-linux-gnu/pkgconfig/alsa.pc or libudev.pc",
                    d.display()
                ),
                None => format!(
                    "a {arch} Linux package needs a sysroot with the ALSA and udev libraries of glibc 2.31 to link against"
                ),
            },
        )
        .fix(format!("run scripts/package-sysroots.sh and set {var} to the folder it prints (docs/ai/PACKAGING.md)"))
        .into()),
    }
}

fn io(path: &Path, e: std::io::Error) -> Errors {
    Diagnostic::error(codes::IO, format!("{}: {e}", path.display())).into()
}

fn assemble(opts: &Options, game: &Game, target: &Target, binary: &Binary) -> Result<Built, Errors> {
    let name = format!("{}-{}", game.name, target.name);
    std::fs::create_dir_all(&opts.out).map_err(|e| io(&opts.out, e))?;
    files::sweep_partials(&opts.out);
    // Assemble beside the final folder and rename at the end, so a crash never leaves a
    // half-written package under the final name (decision 0032).
    let staging = opts.out.join(format!(".{name}.partial-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| io(&staging, e))?;
    let result = fill(opts, game, target, binary, &staging);
    let (manifest, crates) = match result {
        Ok(v) => v,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(e);
        }
    };
    let dir = opts.out.join(&name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::rename(&staging, &dir).map_err(|e| io(&dir, e))?;
    let bin_name = manifest["binary"].as_str().unwrap_or_default().to_string();
    let smoke = if opts.smoke {
        smoke(&dir, &bin_name, game, target)?
    } else {
        "skipped: not asked".into()
    };
    let zip = if opts.archive {
        let path = opts.out.join(format!("{name}.zip"));
        zip::write_dir(&dir, &name, &path).map_err(|e| io(&path, e))?;
        Some(path.display().to_string())
    } else {
        None
    };
    let listed = manifest["files"].as_array().map(Vec::len).unwrap_or(0);
    let bytes = manifest["files"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|f| f["bytes"].as_u64())
        .sum();
    Ok(Built {
        target: target.name.into(),
        dir: dir.display().to_string(),
        zip,
        binary: bin_name,
        builder: binary.builder.clone(),
        files: listed + 1,
        bytes,
        glibc_required: manifest["glibc_required"].as_str().map(str::to_string),
        smoke,
        crates,
        warnings: warnings(game, target),
    })
}

/// Whether the game plays in VR (its default binary has the `xr` feature).
pub fn is_vr(game: &Game) -> bool {
    game.features.iter().any(|f| f == "xr")
}

fn warnings(game: &Game, target: &Target) -> Vec<String> {
    let mut out = Vec::new();
    if is_vr(game)
        && !target.linux
        && !game
            .files
            .iter()
            .any(|f| f.eq_ignore_ascii_case("openxr_loader.dll"))
    {
        out.push(
            "VR game without openxr_loader.dll: put the OpenXR loader in the game folder (beside marionette.json) so it ships next to the executable; until then the game starts flat".into(),
        );
    }
    out
}

fn fill(
    opts: &Options,
    game: &Game,
    target: &Target,
    binary: &Binary,
    dir: &Path,
) -> Result<(Value, usize), Errors> {
    let bin_name = if target.linux {
        game.name.clone()
    } else {
        format!("{}.exe", game.name)
    };
    let bytes = std::fs::read(&binary.path).map_err(|e| io(&binary.path, e))?;
    let glibc = if target.linux {
        let need = glibc_required(&bytes);
        if let Some(v) = need.filter(|v| *v > GLIBC_FLOOR) {
            return Err(Diagnostic::error(
                codes::GLIBC,
                format!(
                    "the {} binary needs glibc {}.{}, newer than the {}.{} floor of the Steam runtime",
                    target.name, v.0, v.1, GLIBC_FLOOR.0, GLIBC_FLOOR.1
                ),
            )
            .fix("build through cargo zigbuild (install zig and cargo-zigbuild), which pins the floor")
            .into());
        }
        need.map(|(a, b)| format!("{a}.{b}"))
    } else {
        None
    };
    files::write_executable(&dir.join(&bin_name), &bytes).map_err(|e| io(&dir.join(&bin_name), e))?;
    for f in &game.files {
        let to = dir.join(f);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io(parent, e))?;
        }
        std::fs::copy(opts.game.join(f), &to).map_err(|e| io(&to, e))?;
    }
    let crates = licenses::write(opts, game, target, &dir.join("licenses"))?;
    let readme = files::readme(game, target, &bin_name);
    std::fs::write(dir.join("README.txt"), readme).map_err(|e| io(&dir.join("README.txt"), e))?;
    let listed = files::list(dir)?;
    let manifest = json!({
        "format": FORMAT,
        "version": 1,
        "game": game.name,
        "game_version": game.version,
        "title": game.settings.title.clone().unwrap_or_else(|| game.name.clone()),
        "target": target.name,
        "triple": target.triple,
        "binary": bin_name,
        "entry": game.entry,
        "window": game.settings.window.clone().unwrap_or_else(|| target.window.to_string()),
        "vr": is_vr(game),
        "engine_version": env!("CARGO_PKG_VERSION"),
        "features": if game.rust_package.is_some() { Value::Null } else { json!(game.features) },
        "glibc_required": glibc,
        "files": listed,
    });
    let text = serde_json::to_string_pretty(&manifest).unwrap_or_default() + "\n";
    std::fs::write(dir.join(MANIFEST), text).map_err(|e| io(&dir.join(MANIFEST), e))?;
    Ok((manifest, crates))
}

/// Ticks the smoke run simulates.
pub const SMOKE_TICKS: u64 = 60;

fn smoke(dir: &Path, bin: &str, game: &Game, target: &Target) -> Result<String, Errors> {
    if target.triple != host_triple() {
        return Ok(format!("skipped: {} cannot run on this machine", target.triple));
    }
    let dir = &dir.canonicalize().map_err(|e| io(dir, e))?;
    let mut cmd = Command::new(dir.join(bin));
    cmd.current_dir(dir).args([
        "--json",
        "run",
        &game.entry,
        "--headless",
        "--ticks",
        &SMOKE_TICKS.to_string(),
    ]);
    let out = marionette_host::output(cmd).map_err(|e| {
        Errors::from(Diagnostic::error(
            codes::SMOKE,
            format!("cannot start the packaged binary: {e}"),
        ))
    })?;
    let v: Value = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
    if out.status.success() && v["ok"] == true {
        Ok("passed".into())
    } else {
        Err(Diagnostic::error(
            codes::SMOKE,
            format!(
                "the packaged {} game did not run its entry scene: {}{}",
                target.name,
                String::from_utf8_lossy(&out.stdout)
                    .chars()
                    .take(800)
                    .collect::<String>(),
                String::from_utf8_lossy(&out.stderr)
                    .chars()
                    .take(800)
                    .collect::<String>()
            ),
        )
        .fix("run the same command in the package folder to see the full error")
        .into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glibc_versions_are_read_from_binary_bytes() {
        let bytes = b"\0libc.so.6\0GLIBC_2.17\0GLIBC_2.28\0GLIBC_2.3.4\0GLIBC_PRIVATE\0GLIBC_2.30\0";
        assert_eq!(glibc_required(bytes), Some((2, 30)));
        assert_eq!(glibc_required(b"no versions"), None);
    }

    #[test]
    fn targets_resolve_by_name_and_unknown_names_are_errors() {
        assert_eq!(target("steam-frame").unwrap().triple, "aarch64-unknown-linux-gnu");
        assert_eq!(target("steamdeck").unwrap().window, "1280x800");
        let e = target("quest").unwrap_err();
        assert_eq!(e.0[0].code, codes::UNKNOWN_TARGET);
        let two = targets(&["linux-x64".into(), "linux-x64".into()]).unwrap();
        assert_eq!(two.len(), 1);
    }

    #[test]
    fn windows_packages_need_a_windows_machine() {
        let w = target("windows-x64").unwrap();
        let r = can_build(&w);
        if cfg!(windows) {
            assert_eq!(r.unwrap(), Builder::Cargo);
        } else {
            assert_eq!(r.unwrap_err().0[0].code, codes::TOOLCHAIN);
        }
    }
}
