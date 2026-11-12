//! TypeScript to JavaScript by removing types (oxc). Nothing is type-checked here: an agent
//! that wants checking runs `tsc --noEmit` against the generated `marionette.d.ts`.
//!
//! One rewrite on top of type removal: `a ** b` becomes `Math.pow(a, b)`, so it uses the
//! engine's deterministic `Math.pow` instead of the C library's `pow`.

use oxc::allocator::Allocator;
use oxc::codegen::Codegen;
use oxc::diagnostics::OxcDiagnostic;
use oxc::parser::Parser;
use oxc::semantic::SemanticBuilder;
use oxc::span::SourceType;
use oxc::transformer::{TransformOptions, Transformer};
use std::path::Path;

/// A syntax error: message and 1-based line and column, where known.
#[derive(Debug, Clone, PartialEq)]
pub struct StripError {
    pub message: String,
    pub line: Option<usize>,
    pub column: Option<usize>,
}

fn position(source: &str, offset: usize) -> (usize, usize) {
    let before = &source[..offset.min(source.len())];
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    (line, column)
}

fn first_error(source: &str, diags: &[OxcDiagnostic]) -> StripError {
    let d = &diags[0];
    let pos = d.labels.first().map(|l| position(source, l.offset() as usize));
    StripError {
        message: d.message.to_string(),
        line: pos.map(|p| p.0),
        column: pos.map(|p| p.1),
    }
}

/// Converts one TypeScript module to JavaScript.
pub fn strip(source: &str, file: &str) -> Result<String, StripError> {
    let allocator = Allocator::default();
    let source_type = SourceType::ts().with_module(true);
    let parsed = Parser::new(&allocator, source, source_type).parse();
    if !parsed.diagnostics.is_empty() {
        return Err(first_error(source, &parsed.diagnostics));
    }
    let mut program = parsed.program;
    let semantic = SemanticBuilder::new()
        .with_check_syntax_error(true)
        .build(&program);
    if !semantic.diagnostics.is_empty() {
        return Err(first_error(source, &semantic.diagnostics));
    }
    let scoping = semantic.semantic.into_scoping();
    let mut options = TransformOptions::default();
    options.env.es2016.exponentiation_operator = true;
    let ret =
        Transformer::new(&allocator, Path::new(file), &options).build_with_scoping(scoping, &mut program);
    if ret.diagnostics.has_errors() {
        let errors: Vec<OxcDiagnostic> = ret.diagnostics.errors().cloned().collect();
        return Err(first_error(source, &errors));
    }
    Ok(Codegen::new().build(&program).code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types_are_removed_and_power_uses_math_pow() {
        let js = strip("export function f(a: number): number { return a ** 2; }", "t.ts").unwrap();
        assert!(!js.contains(": number"), "{js}");
        assert!(js.contains("Math.pow"), "{js}");
    }

    #[test]
    fn syntax_errors_have_positions() {
        let e = strip("let a = 1;\nlet b = ;\n", "t.ts").unwrap_err();
        assert_eq!(e.line, Some(2), "{e:?}");
        assert!(e.column.is_some());
    }
}
