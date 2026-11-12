//! Files only a person may write. Every write helper in the engine refuses these names, so an
//! agent cannot declare a license or approve its own spending by accident. The rules live in
//! core so they hold in every build, whichever optional crates a game leaves out (decision
//! 0062). They are not a security boundary against a program that writes files directly.

use crate::Diagnostic;

/// Error code: a tool tried to write a license declaration (`*.license.json`).
pub const DECLARATION_WRITE_REFUSED: &str = "import.declaration_write_refused";
/// Error code: a tool tried to write an image approval (`*.approval.json`).
pub const APPROVAL_WRITE_REFUSED: &str = "generate.approval_write_refused";
/// File name suffix of an image approval.
pub const APPROVAL_SUFFIX: &str = ".approval.json";

/// True when `name` is a license declaration file name (`*.license.json`). Tools must never
/// write such a file: only a person may declare a license (ASSETS-AND-LICENSES.md rule 9).
pub fn is_declaration_file_name(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".license.json")
}

/// The refusal for a tool that tries to write a declaration file.
pub fn declaration_write_refused(shown: &str) -> Diagnostic {
    Diagnostic::error(
        DECLARATION_WRITE_REFUSED,
        format!("`{shown}` is a license declaration; only a person may write one, never a tool or an AI agent"),
    )
    .file(shown)
    .fix("ask the person who bought the asset to write the declaration (format in docs/ai/ASSETS-AND-LICENSES.md)")
}

/// True when `name` is an image approval file name (`*.approval.json`).
pub fn is_approval_file_name(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(APPROVAL_SUFFIX)
}

/// The refusal for a tool that tries to write an approval file.
pub fn approval_write_refused(shown: &str) -> Diagnostic {
    Diagnostic::error(
        APPROVAL_WRITE_REFUSED,
        format!(
            "`{shown}` is an image approval; only a person may approve, with `marionette generate approve`"
        ),
    )
    .file(shown)
    .fix("ask the project owner to look at the image and run `marionette generate approve <name>`")
}

/// The refusal for writing a file named `file_name`, if only a person may write it.
pub fn refuse_write(file_name: &str, shown: &str) -> Option<Diagnostic> {
    if is_declaration_file_name(file_name) {
        Some(declaration_write_refused(shown))
    } else if is_approval_file_name(file_name) {
        Some(approval_write_refused(shown))
    } else {
        None
    }
}
