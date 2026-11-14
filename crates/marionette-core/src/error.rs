//! Errors as data. Every failure is a [`Diagnostic`] with a stable code, an optional
//! location (file, line), the entity involved, and a suggested fix. All of it serializes
//! to JSON so agents can act on it without parsing prose.

use serde::Serialize;
use std::fmt;

/// Stable error codes. Codes never change meaning once released; new ones are added.
pub mod codes {
    pub const IO_READ: &str = "io.read";
    pub const IO_WRITE: &str = "io.write";
    pub const SCENE_PARSE: &str = "scene.parse";
    pub const SCENE_FORMAT: &str = "scene.format";
    pub const SCENE_VERSION: &str = "scene.version";
    pub const SCENE_SHAPE: &str = "scene.shape";
    pub const SCENE_DUPLICATE_ID: &str = "scene.duplicate_id";
    pub const SCENE_INVALID_ID: &str = "scene.invalid_id";
    pub const SCENE_UNKNOWN_COMPONENT: &str = "scene.unknown_component";
    pub const SCENE_UNKNOWN_RESOURCE: &str = "scene.unknown_resource";
    pub const SCENE_MISSING_FIELD: &str = "scene.missing_field";
    pub const SCENE_UNKNOWN_FIELD: &str = "scene.unknown_field";
    pub const SCENE_FIELD_TYPE: &str = "scene.field_type";
    pub const SCENE_DANGLING_REF: &str = "scene.dangling_ref";
    pub const SCENE_PARENT_CYCLE: &str = "scene.parent_cycle";
    pub const SCENE_NOT_CANONICAL: &str = "scene.not_canonical";
    pub const REGISTRY_DUPLICATE: &str = "registry.duplicate";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

/// One structured error or warning.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Diagnostic {
    pub code: &'static str,
    pub severity: Severity,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<usize>,
    /// Stable scene id of the entity involved, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity: Option<u64>,
    /// Component or resource type name involved, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component: Option<String>,
    /// Field name involved, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    /// JSON Pointer (RFC 6901) to the value involved, e.g. `/entities/2/components/Follow`.
    /// Present when the problem is in a document; stable even when no text exists yet
    /// (for example an edit that was refused before it was written).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pointer: Option<String>,
    /// A concrete suggestion for fixing the problem.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
}

impl Diagnostic {
    pub fn error(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            severity: Severity::Error,
            message: message.into(),
            file: None,
            line: None,
            column: None,
            entity: None,
            component: None,
            field: None,
            pointer: None,
            fix: None,
        }
    }
    pub fn warning(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            ..Self::error(code, message)
        }
    }
    pub fn file(mut self, file: impl Into<String>) -> Self {
        self.file = Some(file.into());
        self
    }
    pub fn at(mut self, line: Option<usize>, column: Option<usize>) -> Self {
        self.line = line;
        self.column = column;
        self
    }
    /// Sets line and column from a `(line, column)` pair, if known.
    pub fn at_pos(self, pos: Option<(usize, usize)>) -> Self {
        self.at(pos.map(|p| p.0), pos.map(|p| p.1))
    }
    pub fn entity(mut self, id: u64) -> Self {
        self.entity = Some(id);
        self
    }
    pub fn component(mut self, name: impl Into<String>) -> Self {
        self.component = Some(name.into());
        self
    }
    pub fn field(mut self, name: impl Into<String>) -> Self {
        self.field = Some(name.into());
        self
    }
    pub fn pointer(mut self, pointer: impl Into<String>) -> Self {
        self.pointer = Some(pointer.into());
        self
    }
    pub fn fix(mut self, fix: impl Into<String>) -> Self {
        self.fix = Some(fix.into());
        self
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(file) = &self.file {
            write!(f, "{file}")?;
            if let Some(line) = self.line {
                write!(f, ":{line}")?;
                if let Some(col) = self.column {
                    write!(f, ":{col}")?;
                }
            }
            write!(f, ": ")?;
        }
        let sev = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        write!(f, "{sev}[{}]: {}", self.code, self.message)?;
        if let Some(id) = self.entity {
            write!(f, " (entity {id})")?;
        }
        if let Some(fix) = &self.fix {
            write!(f, "\n  fix: {fix}")?;
        }
        Ok(())
    }
}

/// A list of diagnostics; the error type of every fallible core operation.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(transparent)]
pub struct Errors(pub Vec<Diagnostic>);

impl Errors {
    pub fn has_errors(&self) -> bool {
        self.0.iter().any(|d| d.severity == Severity::Error)
    }
    /// Sets `file` on every diagnostic that does not have one yet.
    pub fn with_file(mut self, file: &str) -> Self {
        for d in &mut self.0 {
            if d.file.is_none() {
                d.file = Some(file.to_string());
            }
        }
        self
    }
}

impl From<Diagnostic> for Errors {
    fn from(d: Diagnostic) -> Self {
        Errors(vec![d])
    }
}

impl fmt::Display for Errors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, d) in self.0.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "{d}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Errors {}

pub type Result<T> = std::result::Result<T, Errors>;
