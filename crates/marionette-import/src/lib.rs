//! The owned asset library and the importers that fill it (plan line 8a).
//!
//! - [`library`]: the per-user owned library (a folder shared by all of one user's projects,
//!   located from configuration, never inside the engine repository), its records and lookups.
//! - [`license`]: license declarations written by a person, license records, the engine-locked
//!   content rules and the pasted-license-text check.
//! - [`unity`]: reading Unity project folders and `.unitypackage` archives, `.meta` files and
//!   `.mat` materials (mapped to glTF-style PBR).
//! - [`fbx`]: FBX (2011+, binary or ASCII) to binary glTF conversion (decision 0027).
//! - [`import`]: the import pipeline that applies every rule and writes the import report.
//! - [`prefab`]: Unity prefabs (text YAML, nested prefabs, variants) to Marionette prefabs
//!   (decision 0040); [`unity_yaml`] reads Unity's text serialization.
//! - [`pool`]: the admission gate the CC0 pool must call; it refuses owned assets.
//!
//! The rules are normative in `docs/ai/ASSETS-AND-LICENSES.md`; the design is decision 0018.

pub mod fbx;
pub mod import;
pub mod library;
pub mod license;
pub mod pool;
pub mod prefab;
pub mod tar;
pub mod unity;
pub mod unity_yaml;

pub use import::{ImportOptions, ImportReport, import};
pub use library::Library;

/// Stable error and refusal codes added by this crate. Codes never change meaning.
pub mod codes {
    /// No owned library is configured (input, `MARIONETTE_LIBRARY`, or the user config file).
    pub const LIBRARY_NOT_CONFIGURED: &str = "library.not_configured";
    /// The library path is relative; it must be absolute.
    pub const LIBRARY_RELATIVE: &str = "library.relative_path";
    /// The library path is inside the engine repository.
    pub const LIBRARY_INSIDE_ENGINE: &str = "library.inside_engine";
    /// The library path is inside the current project.
    pub const LIBRARY_INSIDE_PROJECT: &str = "library.inside_project";
    /// The folder exists, is not empty, and is not an owned library.
    pub const LIBRARY_NOT_A_LIBRARY: &str = "library.not_a_library";
    /// No asset with that id in the library.
    pub const LIBRARY_NO_ASSET: &str = "library.no_asset";
    /// A record in the library could not be read.
    pub const LIBRARY_BAD_RECORD: &str = "library.bad_record";

    /// The import source does not exist or cannot be read.
    pub const IMPORT_SOURCE: &str = "import.source";
    /// The archive is not a valid `.unitypackage` (gzip + tar).
    pub const IMPORT_BAD_ARCHIVE: &str = "import.bad_archive";
    /// The archive or one entry is larger than the import limits.
    pub const IMPORT_TOO_LARGE: &str = "import.too_large";
    /// A declaration file is malformed.
    pub const IMPORT_BAD_DECLARATION: &str = "import.bad_declaration";
    /// Writing a license declaration was attempted by a tool; only a person may write one.
    pub const DECLARATION_WRITE_REFUSED: &str = "import.declaration_write_refused";

    // Refusal reasons (one per refused asset in the import report).
    /// No declaration covers the asset, so its license is unknown.
    pub const REFUSED_LICENSE_UNKNOWN: &str = "import.license_unknown";
    /// The declaration says use in other engines is not allowed.
    pub const REFUSED_OTHER_ENGINES_NO: &str = "import.other_engines_not_allowed";
    /// The pasted license text limits use to one engine.
    pub const REFUSED_ENGINE_LOCKED_LICENSE: &str = "import.engine_locked_license";
    /// Content published by Unity or Epic, or engine sample or starter content.
    pub const REFUSED_ENGINE_CONTENT: &str = "import.engine_content";
    /// The asset's path inside the source is unsafe (absolute, `..`, or not under Assets/).
    pub const REFUSED_BAD_PATH: &str = "import.bad_path";

    // Skip reasons (assets not imported for a reason other than their license).
    /// A file type this importer does not bring in (scripts, scenes, prefabs, audio, ...).
    pub const SKIPPED_UNSUPPORTED: &str = "import.unsupported_type";
    /// A file the importer brings in together with another one (glTF buffers).
    pub const SKIPPED_COMPANION: &str = "import.companion_file";
    /// A Unity file saved in binary serialization, which this importer does not read.
    pub const SKIPPED_BINARY_SERIALIZED: &str = "import.binary_serialized";

    /// Pool admission refused: the candidate is (inside) an owned-library asset.
    pub const POOL_OWNED_ASSET: &str = "pool.owned_asset";
    /// Pool admission refused: a file has the same content as an owned-library file.
    pub const POOL_OWNED_CONTENT: &str = "pool.owned_content";
    /// Pool admission refused: the candidate carries an owned or non-CC0 license record.
    pub const POOL_NOT_CC0: &str = "pool.not_cc0";
}

/// SHA-256 as lowercase hex.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// True when `name` is a license declaration file name (`*.license.json`). Tools must never
/// write such a file: only a person may declare a license (ASSETS-AND-LICENSES.md rule 9).
pub fn is_declaration_file_name(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".license.json")
}

/// The refusal for a tool that tries to write a declaration file.
pub fn declaration_write_refused(shown: &str) -> marionette_core::Diagnostic {
    marionette_core::Diagnostic::error(
        codes::DECLARATION_WRITE_REFUSED,
        format!("`{shown}` is a license declaration; only a person may write one, never a tool or an AI agent"),
    )
    .file(shown)
    .fix("ask the person who bought the asset to write the declaration (format in docs/ai/ASSETS-AND-LICENSES.md)")
}
