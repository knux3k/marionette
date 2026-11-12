//! Job TypeScript to WebAssembly.
//!
//! One pass per function over the oxc syntax tree checks types and writes WebAssembly
//! instructions at the same time. The subset and its rules are documented in
//! `docs/ai/JOBS-TS.md`; the short version:
//!
//! - scalar types: `number` (64-bit float, as in JavaScript), `i32` (32-bit integer, declared by
//!   the job prelude as `type i32 = number`) and `boolean`;
//! - array types (parameters only): `Float32Array`, `Float64Array`, `Int32Array`;
//! - statements: `let`/`const`, assignment and compound assignment, `++`/`--`, `if`, `for`,
//!   `while`, `do`/`while`, `break`, `continue`, `return`, calls;
//! - expressions: arithmetic, comparison, bitwise, logical (on booleans), `?:`, calls to
//!   functions of the same file, `Math.*`, element access, `.length`.
//!
//! Numbers behave as in JavaScript: arithmetic on `number` is 64-bit; `Float32Array` elements
//! are widened on load and rounded on store, exactly like a JavaScript engine does. That is
//! why the same file gives bit-identical results compiled or interpreted (see the tests).
//! `i32` arithmetic wraps on overflow, where JavaScript would leave the integer range.
//!
//! An unannotated `let` that starts as an integer is `i32`; if the function later stores a
//! fraction in it, the compiler widens that variable to `number` and compiles the function
//! again (at most once per variable), so `let sum = 0; sum += a[i];` just works.

use marionette_core::Diagnostic;
use oxc::allocator::Allocator;
use oxc::ast::ast::*;
use oxc::parser::Parser;
use oxc::span::{GetSpan, SourceType, Span};
use oxc::syntax::operator::{
    AssignmentOperator, BinaryOperator, LogicalOperator, UnaryOperator, UpdateOperator,
};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use wasm_encoder::{
    BlockType, CodeSection, EntityType, ExportKind, ExportSection, Function as WFunction, FunctionSection,
    ImportSection, Instruction as I, MemArg, MemorySection, MemoryType, Module, TypeSection, ValType,
};

/// Stable error codes of this crate (errors are data: codes never change meaning).
pub mod codes {
    /// The file is not valid TypeScript.
    pub const SYNTAX: &str = "jobts.syntax";
    /// A construct outside the job subset (strings, objects, closures, classes, ...).
    pub const UNSUPPORTED: &str = "jobts.unsupported";
    /// A value has the wrong type for where it is used.
    pub const TYPE: &str = "jobts.type";
    /// A name that is not a variable, parameter, constant or function of this file.
    pub const UNKNOWN_NAME: &str = "jobts.unknown_name";
    /// A function signature is missing a type or uses a type outside the subset.
    pub const SIGNATURE: &str = "jobts.signature";
    /// A top-level statement other than a function, a constant or a type-only import.
    pub const TOP_LEVEL: &str = "jobts.top_level";
    /// The file cannot be read.
    pub const READ: &str = "jobts.read";
    /// `run` names a function the file does not export.
    pub const NO_FUNCTION: &str = "jobts.no_function";
    /// The arguments given to `run` do not match the function's parameters.
    pub const ARGS: &str = "jobts.args";
    /// The job stopped with an error (index out of range, missing return, remainder by zero).
    pub const TRAP: &str = "jobts.trap";
    /// The job ran longer than its time limit (endless loop?).
    pub const TIMEOUT: &str = "jobts.timeout";
    /// The WebAssembly runtime refused the compiled module (a compiler bug: please report).
    pub const INTERNAL: &str = "jobts.internal";
}

/// Element type of a typed array.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Elem {
    F32,
    F64,
    I32,
}

impl Elem {
    pub fn ts_name(self) -> &'static str {
        match self {
            Elem::F32 => "Float32Array",
            Elem::F64 => "Float64Array",
            Elem::I32 => "Int32Array",
        }
    }
    pub fn size(self) -> u32 {
        match self {
            Elem::F64 => 8,
            _ => 4,
        }
    }
    fn shift(self) -> i32 {
        match self {
            Elem::F64 => 3,
            _ => 2,
        }
    }
    /// The scalar type an element reads as.
    fn read_ty(self) -> Ty {
        match self {
            Elem::I32 => Ty::I32,
            _ => Ty::F64,
        }
    }
}

/// Type of a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ty {
    F64,
    I32,
    Bool,
    Void,
    Arr(Elem),
    /// An integer literal (or constant) whose type comes from where it is used.
    IntLit,
}

impl Ty {
    fn name(self) -> String {
        match self {
            Ty::F64 => "number".into(),
            Ty::I32 | Ty::IntLit => "i32".into(),
            Ty::Bool => "boolean".into(),
            Ty::Void => "void".into(),
            Ty::Arr(e) => e.ts_name().into(),
        }
    }
    fn val(self) -> ValType {
        match self {
            Ty::F64 => ValType::F64,
            _ => ValType::I32,
        }
    }
    fn is_int(self) -> bool {
        matches!(self, Ty::I32 | Ty::IntLit)
    }
    fn is_num(self) -> bool {
        matches!(self, Ty::I32 | Ty::IntLit | Ty::F64)
    }
}

/// A parameter or return type as the host sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamType {
    Number,
    I32,
    Boolean,
    Array(Elem),
    Void,
}

impl ParamType {
    fn ty(self) -> Ty {
        match self {
            ParamType::Number => Ty::F64,
            ParamType::I32 => Ty::I32,
            ParamType::Boolean => Ty::Bool,
            ParamType::Array(e) => Ty::Arr(e),
            ParamType::Void => Ty::Void,
        }
    }
    pub fn ts_name(self) -> &'static str {
        match self {
            ParamType::Number => "number",
            ParamType::I32 => "i32",
            ParamType::Boolean => "boolean",
            ParamType::Array(e) => e.ts_name(),
            ParamType::Void => "void",
        }
    }
}

/// One function of a job file.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FunctionInfo {
    pub name: String,
    pub params: Vec<(String, ParamType)>,
    pub returns: ParamType,
    pub exported: bool,
    /// Index of the function in the WebAssembly module.
    #[serde(skip)]
    pub(crate) index: u32,
}

/// A compiled job file: WebAssembly bytes plus the signatures of its functions.
#[derive(Debug, Clone)]
pub struct Compiled {
    pub wasm: Vec<u8>,
    pub functions: Vec<FunctionInfo>,
    pub file: String,
    pub safety: Safety,
}

/// Unary math functions the host provides (pure-Rust `libm`, the same ones scripts use).
pub(crate) const MATH1: &[&str] = &[
    "sin", "cos", "tan", "asin", "acos", "atan", "sinh", "cosh", "tanh", "asinh", "acosh", "atanh", "exp",
    "expm1", "log", "log1p", "log2", "log10", "cbrt",
];
/// Binary math functions the host provides (`fmod` is the `%` operator on numbers).
pub(crate) const MATH2: &[&str] = &["atan2", "pow", "fmod", "hypot"];

/// Trap kinds passed to the host's `trap` import.
pub(crate) mod trap {
    pub const INDEX: i32 = 0;
    pub const NO_RETURN: i32 = 1;
    pub const REM_ZERO: i32 = 2;
}

// Function index layout: imports first, then the internal helper, then the file's functions.
const F_TRAP: u32 = 0;
const F_TO_INT32_SLOW: u32 = 1;
const F_MATH_BASE: u32 = 2;
fn math_index(name: &str) -> Option<u32> {
    MATH1
        .iter()
        .chain(MATH2)
        .position(|n| *n == name)
        .map(|i| F_MATH_BASE + i as u32)
}
const N_IMPORTS: u32 = F_MATH_BASE + (MATH1.len() + MATH2.len()) as u32;
const F_TO_INT32: u32 = N_IMPORTS;
const F_USER_BASE: u32 = N_IMPORTS + 1;

fn line_col(source: &str, offset: u32) -> (usize, usize) {
    let before = &source[..(offset as usize).min(source.len())];
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    (line, column)
}

/// A compile error before it becomes a [`Diagnostic`].
struct CErr {
    code: &'static str,
    message: String,
    span: Span,
    fix: Option<String>,
}

enum Fail {
    Err(CErr),
    /// Widen the inferred `i32` variable declared at this offset to `number`, then retry.
    Widen(u32),
}

type R<T> = Result<T, Fail>;

fn err<T>(code: &'static str, span: Span, message: impl Into<String>, fix: &str) -> R<T> {
    Err(Fail::Err(CErr {
        code,
        message: message.into(),
        span,
        fix: (!fix.is_empty()).then(|| fix.to_string()),
    }))
}

fn type_err<T>(span: Span, message: impl Into<String>, fix: &str) -> R<T> {
    err(codes::TYPE, span, message, fix)
}

/// Safety checks, as in Burst: on while developing, off for shipping builds if wanted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Safety {
    /// Array indexes are checked (an out-of-range index stops the job with `jobts.trap` and
    /// its line) and each chunk call has a time limit (`jobts.timeout`). About 35% slower.
    #[default]
    Checked,
    /// No index checks and no time limit. Still sandboxed (a job can only touch its own
    /// WebAssembly memory) and still deterministic, but a wrong index reads or writes
    /// another array of the same call instead of stopping, and an endless loop hangs.
    Unchecked,
}

impl Safety {
    pub fn checked(self) -> bool {
        self == Safety::Checked
    }
}

/// Compiles a job file with safety checks. On failure every problem found is returned (one
/// per function at most, plus top-level problems), each with a line, column and a fix.
pub fn compile(source: &str, file: &str) -> Result<Compiled, Vec<Diagnostic>> {
    compile_with(source, file, Safety::Checked)
}

/// [`compile`] with a choice of [`Safety`].
pub fn compile_with(source: &str, file: &str, safety: Safety) -> Result<Compiled, Vec<Diagnostic>> {
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, SourceType::ts().with_module(true)).parse();
    let diag = |e: CErr| {
        let mut d = Diagnostic::error(e.code, e.message)
            .file(file)
            .at_pos(Some(line_col(source, e.span.start)));
        if let Some(f) = e.fix {
            d = d.fix(f);
        }
        d
    };
    if !parsed.diagnostics.is_empty() {
        return Err(parsed
            .diagnostics
            .iter()
            .map(|d| {
                let offset = d.labels.first().map(|l| l.offset());
                Diagnostic::error(codes::SYNTAX, d.message.to_string())
                    .file(file)
                    .at_pos(offset.map(|o| line_col(source, o)))
            })
            .collect());
    }
    let program = parsed.program;
    let mut errors = Vec::new();
    let mut consts: HashMap<String, Const> = HashMap::new();
    let mut funcs: Vec<(&Function, bool)> = Vec::new();
    for stmt in &program.body {
        match top_level(stmt, &mut consts, &mut funcs) {
            Ok(()) => {}
            Err(Fail::Err(e)) => errors.push(diag(e)),
            Err(Fail::Widen(_)) => unreachable!("no widening at top level"),
        }
    }
    // Signatures first, so functions may call functions declared later.
    let mut infos = Vec::new();
    let mut by_name: HashMap<String, usize> = HashMap::new();
    for (f, exported) in &funcs {
        match signature(f, *exported) {
            Ok(mut info) => {
                if by_name.contains_key(&info.name) || consts.contains_key(&info.name) {
                    errors.push(diag(CErr {
                        code: codes::TOP_LEVEL,
                        message: format!("`{}` is declared twice", info.name),
                        span: f.span,
                        fix: Some("rename one of the two declarations".into()),
                    }));
                    continue;
                }
                info.index = F_USER_BASE + infos.len() as u32;
                by_name.insert(info.name.clone(), infos.len());
                infos.push(info);
            }
            Err(Fail::Err(e)) => errors.push(diag(e)),
            Err(Fail::Widen(_)) => unreachable!(),
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let module = ModuleCtx {
        safety,
        consts: &consts,
        infos: &infos,
        by_name: &by_name,
        source,
    };
    let mut bodies = Vec::new();
    for ((f, _), info) in funcs.iter().zip(&infos) {
        let mut widen = HashSet::new();
        loop {
            match FnCtx::compile(&module, f, info, &widen) {
                Ok(body) => {
                    bodies.push(body);
                    break;
                }
                Err(Fail::Widen(at)) if widen.insert(at) => continue,
                Err(Fail::Widen(_)) => {
                    errors.push(diag(CErr {
                        code: codes::INTERNAL,
                        message: "type inference did not settle".into(),
                        span: f.span,
                        fix: Some("annotate the function's variables with `: number` or `: i32`".into()),
                    }));
                    break;
                }
                Err(Fail::Err(e)) => {
                    errors.push(diag(e));
                    break;
                }
            }
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(Compiled {
        wasm: assemble(&infos, bodies),
        functions: infos,
        file: file.to_string(),
        safety,
    })
}

#[derive(Clone, Copy)]
struct Const {
    value: f64,
    int_like: bool,
}

fn number_literal(n: &NumericLiteral) -> Const {
    let raw = n.raw.as_ref().map_or("", |r| r.as_str());
    let written_as_float = raw.contains('.') || raw.contains('e') || raw.contains('E');
    let int_like = !written_as_float
        && n.value.fract() == 0.0
        && n.value >= i32::MIN as f64
        && n.value <= i32::MAX as f64;
    Const {
        value: n.value,
        int_like,
    }
}

fn math_const(name: &str) -> Option<f64> {
    Some(match name {
        "PI" => std::f64::consts::PI,
        "E" => std::f64::consts::E,
        "LN2" => std::f64::consts::LN_2,
        "LN10" => std::f64::consts::LN_10,
        "LOG2E" => std::f64::consts::LOG2_E,
        "LOG10E" => std::f64::consts::LOG10_E,
        "SQRT2" => std::f64::consts::SQRT_2,
        "SQRT1_2" => std::f64::consts::FRAC_1_SQRT_2,
        _ => return None,
    })
}

/// A constant expression at top level: a number, `-number`, `Math.PI` and friends.
fn const_value(e: &Expression) -> Option<Const> {
    match e {
        Expression::NumericLiteral(n) => Some(number_literal(n)),
        Expression::ParenthesizedExpression(p) => const_value(&p.expression),
        Expression::UnaryExpression(u) if u.operator == UnaryOperator::UnaryNegation => {
            const_value(&u.argument).map(|c| Const {
                value: -c.value,
                int_like: c.int_like && c.value != 0.0,
            })
        }
        Expression::StaticMemberExpression(m) => match &m.object {
            Expression::Identifier(o) if o.name.as_str() == "Math" => math_const(m.property.name.as_str())
                .map(|value| Const {
                    value,
                    int_like: false,
                }),
            _ => None,
        },
        _ => None,
    }
}

const TOP_FIX: &str = "a job file holds only `function` declarations (exported ones are callable from the engine), `const NAME = <number>` constants, `type i32 = number` and `import type` lines";

fn top_level<'a>(
    stmt: &'a Statement<'a>,
    consts: &mut HashMap<String, Const>,
    funcs: &mut Vec<(&'a Function<'a>, bool)>,
) -> R<()> {
    let decl: &Declaration = match stmt {
        Statement::ExportNamedDeclaration(_) | Statement::ExportDefaultDeclaration(_) => {
            return err(
                codes::TOP_LEVEL,
                stmt.span(),
                "only `export function name(...) { ... }` is supported",
                "write `export` directly in front of the function declaration",
            );
        }
        Statement::ExportDeclaration(d) => {
            return match &d.declaration {
                Declaration::FunctionDeclaration(f) => {
                    funcs.push((f, true));
                    Ok(())
                }
                other => top_level_decl(other, consts, funcs, true),
            };
        }
        Statement::ImportDeclaration(i) if i.import_kind.is_type() => return Ok(()),
        Statement::ImportDeclaration(i) => {
            return err(
                codes::TOP_LEVEL,
                i.span,
                "job files cannot import code",
                "put every function the job needs in the same file (`import type` is allowed)",
            );
        }
        Statement::EmptyStatement(_) => return Ok(()),
        Statement::VariableDeclaration(_)
        | Statement::FunctionDeclaration(_)
        | Statement::TSTypeAliasDeclaration(_)
        | Statement::TSInterfaceDeclaration(_)
        | Statement::ClassDeclaration(_)
        | Statement::TSEnumDeclaration(_)
        | Statement::TSGlobalDeclaration(_)
        | Statement::TSImportEqualsDeclaration(_) => stmt.as_declaration().expect("declaration"),
        other => {
            return err(
                codes::TOP_LEVEL,
                other.span(),
                "statements cannot run at the top level of a job file",
                TOP_FIX,
            );
        }
    };
    top_level_decl(decl, consts, funcs, false)
}

fn top_level_decl<'a>(
    decl: &'a Declaration<'a>,
    consts: &mut HashMap<String, Const>,
    funcs: &mut Vec<(&'a Function<'a>, bool)>,
    exported: bool,
) -> R<()> {
    match decl {
        Declaration::FunctionDeclaration(f) => {
            funcs.push((f, exported));
            Ok(())
        }
        Declaration::VariableDeclaration(v) if v.kind == VariableDeclarationKind::Const && !exported => {
            for d in &v.declarations {
                let BindingPattern::BindingIdentifier(id) = &d.id else {
                    return err(
                        codes::UNSUPPORTED,
                        d.span,
                        "destructuring is not supported",
                        "declare one constant per name",
                    );
                };
                let Some(c) = d.init.as_ref().and_then(const_value) else {
                    return err(
                        codes::TOP_LEVEL,
                        d.span,
                        format!("top-level constant `{}` must be a number", id.name),
                        "write `const NAME = 1.5;` (a number, `-number` or `Math.PI`); arrays come in as parameters",
                    );
                };
                consts.insert(id.name.to_string(), c);
            }
            Ok(())
        }
        Declaration::VariableDeclaration(v) => err(
            codes::TOP_LEVEL,
            v.span,
            "job files have no global variables (jobs run in parallel and keep no state between calls)",
            "use `const NAME = <number>` for constants; pass state in and out through array parameters",
        ),
        Declaration::TSTypeAliasDeclaration(t)
            if t.id.name.as_str() == "i32" && matches!(t.type_annotation, TSType::TSNumberKeyword(_)) =>
        {
            Ok(())
        }
        Declaration::TSTypeAliasDeclaration(t) => err(
            codes::TOP_LEVEL,
            t.span,
            "the only type alias a job file may declare is `type i32 = number`",
            "use `number`, `i32`, `boolean`, `Float32Array`, `Float64Array` or `Int32Array` directly",
        ),
        Declaration::ClassDeclaration(c) => err(
            codes::UNSUPPORTED,
            c.span,
            "classes are not part of job TypeScript",
            "use top-level functions and pass data as typed arrays (one array per field)",
        ),
        other => err(
            codes::TOP_LEVEL,
            other.span(),
            "this declaration is not supported in a job file",
            TOP_FIX,
        ),
    }
}

const PARAM_FIX: &str =
    "annotate it as `number`, `i32`, `boolean`, `Float32Array`, `Float64Array` or `Int32Array`";

fn ts_type(t: &TSType, span: Span, what: &str, allow_void: bool) -> R<ParamType> {
    let p = match t {
        TSType::TSNumberKeyword(_) => ParamType::Number,
        TSType::TSBooleanKeyword(_) => ParamType::Boolean,
        TSType::TSVoidKeyword(_) if allow_void => ParamType::Void,
        TSType::TSTypeReference(r) => match &r.type_name {
            TSTypeName::IdentifierReference(id) if r.type_arguments.is_none() => match id.name.as_str() {
                "i32" => ParamType::I32,
                "Float32Array" => ParamType::Array(Elem::F32),
                "Float64Array" => ParamType::Array(Elem::F64),
                "Int32Array" => ParamType::Array(Elem::I32),
                other => {
                    return err(
                        codes::SIGNATURE,
                        span,
                        format!("{what} has type `{other}`, which job code does not know"),
                        PARAM_FIX,
                    );
                }
            },
            _ => {
                return err(
                    codes::SIGNATURE,
                    span,
                    format!("{what} has a type job code does not know"),
                    PARAM_FIX,
                );
            }
        },
        _ => {
            return err(
                codes::SIGNATURE,
                span,
                format!("{what} has a type job code does not know"),
                PARAM_FIX,
            );
        }
    };
    Ok(p)
}

fn local_type(t: &TSType, span: Span, name: &str) -> R<Ty> {
    match ts_type(t, span, &format!("variable `{name}`"), false)? {
        ParamType::Array(_) => err(
            codes::UNSUPPORTED,
            span,
            format!("variable `{name}` is an array; arrays can only be parameters"),
            "job code cannot allocate; pass the array in as a parameter",
        ),
        p => Ok(p.ty()),
    }
}

fn signature(f: &Function, exported: bool) -> R<FunctionInfo> {
    let name = f.id.as_ref().map(|i| i.name.to_string()).unwrap_or_default();
    if f.r#async || f.generator {
        return err(
            codes::UNSUPPORTED,
            f.span,
            format!("function `{name}` is async or a generator"),
            "job functions are plain synchronous functions",
        );
    }
    if f.type_parameters.is_some() {
        return err(
            codes::UNSUPPORTED,
            f.span,
            format!("function `{name}` is generic"),
            "write one function per concrete type",
        );
    }
    if f.body.is_none() {
        return err(
            codes::UNSUPPORTED,
            f.span,
            format!("function `{name}` has no body"),
            "overloads and `declare function` are not supported",
        );
    }
    if f.params.rest.is_some() {
        return err(
            codes::UNSUPPORTED,
            f.params.span,
            format!("function `{name}` has a rest parameter"),
            "list every parameter",
        );
    }
    let mut params = Vec::new();
    for p in &f.params.items {
        let BindingPattern::BindingIdentifier(id) = &p.pattern else {
            return err(
                codes::UNSUPPORTED,
                p.span,
                "destructured parameters are not supported",
                "use one plain parameter per value",
            );
        };
        if p.initializer.is_some() || p.optional {
            return err(
                codes::UNSUPPORTED,
                p.span,
                format!("parameter `{}` is optional or has a default", id.name),
                "every parameter is required; pass the value explicitly",
            );
        }
        let Some(t) = &p.type_annotation else {
            return err(
                codes::SIGNATURE,
                p.span,
                format!("parameter `{}` of `{name}` has no type", id.name),
                PARAM_FIX,
            );
        };
        params.push((
            id.name.to_string(),
            ts_type(
                &t.type_annotation,
                p.span,
                &format!("parameter `{}`", id.name),
                false,
            )?,
        ));
    }
    let returns = match &f.return_type {
        None => ParamType::Void,
        Some(t) => ts_type(
            &t.type_annotation,
            t.span,
            &format!("the return type of `{name}`"),
            true,
        )?,
    };
    if let ParamType::Array(_) = returns {
        return err(
            codes::SIGNATURE,
            f.span,
            format!("`{name}` returns an array"),
            "write results into an array parameter instead",
        );
    }
    Ok(FunctionInfo {
        name,
        params,
        returns,
        exported,
        index: 0,
    })
}

struct ModuleCtx<'m> {
    safety: Safety,
    consts: &'m HashMap<String, Const>,
    infos: &'m [FunctionInfo],
    by_name: &'m HashMap<String, usize>,
    source: &'m str,
}

#[derive(Clone, Copy)]
enum Var {
    Scalar {
        local: u32,
        ty: Ty,
        mutable: bool,
        /// Declaration offset when the type was inferred (it may be widened to `number`).
        inferred: Option<u32>,
    },
    Array {
        ptr: u32,
        len: u32,
        elem: Elem,
    },
}

struct Body {
    locals: Vec<ValType>,
    code: Vec<I<'static>>,
}

struct FnCtx<'m, 'w> {
    m: &'m ModuleCtx<'m>,
    widen: &'w HashSet<u32>,
    n_params: u32,
    locals: Vec<ValType>,
    scopes: Vec<HashMap<String, Var>>,
    code: Vec<I<'static>>,
    depth: u32,
    /// (break target, continue target) label depths of the enclosing loops.
    loops: Vec<(u32, u32)>,
    ret: Ty,
    name: String,
}

const MEM: MemArg = MemArg {
    offset: 0,
    align: 0,
    memory_index: 0,
};

fn mem(align: u32) -> MemArg {
    MemArg { align, ..MEM }
}

impl<'m, 'w> FnCtx<'m, 'w> {
    fn compile(m: &'m ModuleCtx<'m>, f: &Function, info: &FunctionInfo, widen: &'w HashSet<u32>) -> R<Body> {
        let mut cx = FnCtx {
            m,
            widen,
            n_params: 0,
            locals: Vec::new(),
            scopes: vec![HashMap::new()],
            code: Vec::new(),
            depth: 0,
            loops: Vec::new(),
            ret: info.returns.ty(),
            name: info.name.clone(),
        };
        let mut next = 0u32;
        for (name, p) in &info.params {
            let var = match p {
                ParamType::Array(elem) => {
                    next += 2;
                    Var::Array {
                        ptr: next - 2,
                        len: next - 1,
                        elem: *elem,
                    }
                }
                other => {
                    next += 1;
                    Var::Scalar {
                        local: next - 1,
                        ty: other.ty(),
                        mutable: true,
                        inferred: None,
                    }
                }
            };
            cx.scopes[0].insert(name.clone(), var);
        }
        cx.n_params = next;
        let body = f.body.as_ref().expect("checked in signature");
        for s in &body.statements {
            cx.stmt(s)?;
        }
        if cx.ret != Ty::Void {
            let line = cx.line(f.span.end.saturating_sub(1));
            cx.trap(trap::NO_RETURN, line);
        }
        cx.code.push(I::End);
        Ok(Body {
            locals: cx.locals,
            code: cx.code,
        })
    }

    fn line(&self, offset: u32) -> i32 {
        line_col(self.m.source, offset).0 as i32
    }

    fn new_local(&mut self, t: ValType) -> u32 {
        self.locals.push(t);
        self.n_params + self.locals.len() as u32 - 1
    }

    fn push(&mut self, i: I<'static>) {
        self.code.push(i);
    }

    fn trap(&mut self, kind: i32, line: i32) {
        self.push(I::I32Const(kind));
        self.push(I::I32Const(line));
        self.push(I::I32Const(0));
        self.push(I::I32Const(0));
        self.push(I::Call(F_TRAP));
        self.push(I::Unreachable);
    }

    fn lookup(&self, name: &str) -> Option<Var> {
        self.scopes.iter().rev().find_map(|s| s.get(name).copied())
    }

    fn open(&mut self, i: I<'static>) {
        self.push(i);
        self.depth += 1;
    }

    fn close(&mut self) {
        self.push(I::End);
        self.depth -= 1;
    }

    // ---------------------------------------------------------------- statements

    fn block(&mut self, stmts: &[Statement]) -> R<()> {
        self.scopes.push(HashMap::new());
        for s in stmts {
            self.stmt(s)?;
        }
        self.scopes.pop();
        Ok(())
    }

    fn scoped(&mut self, s: &Statement) -> R<()> {
        self.scopes.push(HashMap::new());
        let r = self.stmt(s);
        self.scopes.pop();
        r
    }

    fn stmt(&mut self, s: &Statement) -> R<()> {
        match s {
            Statement::BlockStatement(b) => self.block(&b.body),
            Statement::EmptyStatement(_) => Ok(()),
            Statement::VariableDeclaration(v) => self.var_decl(v),
            Statement::ExpressionStatement(e) => self.expr_stmt(&e.expression),
            Statement::IfStatement(i) => {
                self.expr_as(&i.test, Ty::Bool)?;
                self.open(I::If(BlockType::Empty));
                self.scoped(&i.consequent)?;
                if let Some(alt) = &i.alternate {
                    self.push(I::Else);
                    self.scoped(alt)?;
                }
                self.close();
                Ok(())
            }
            Statement::ForStatement(f) => {
                self.scopes.push(HashMap::new());
                match &f.init {
                    None => {}
                    Some(ForStatementInit::VariableDeclaration(v)) => self.var_decl(v)?,
                    Some(init) => self.expr_stmt(init.to_expression())?,
                }
                self.open(I::Block(BlockType::Empty));
                let brk = self.depth;
                self.open(I::Loop(BlockType::Empty));
                let top = self.depth;
                if let Some(test) = &f.test {
                    self.expr_as(test, Ty::Bool)?;
                    self.push(I::I32Eqz);
                    self.push(I::BrIf(self.depth - brk));
                }
                self.open(I::Block(BlockType::Empty));
                let cont = self.depth;
                self.loops.push((brk, cont));
                self.scoped(&f.body)?;
                self.loops.pop();
                self.close();
                if let Some(u) = &f.update {
                    self.expr_stmt(u)?;
                }
                self.push(I::Br(self.depth - top));
                self.close();
                self.close();
                self.scopes.pop();
                Ok(())
            }
            Statement::WhileStatement(w) => {
                self.open(I::Block(BlockType::Empty));
                let brk = self.depth;
                self.open(I::Loop(BlockType::Empty));
                let top = self.depth;
                self.expr_as(&w.test, Ty::Bool)?;
                self.push(I::I32Eqz);
                self.push(I::BrIf(self.depth - brk));
                self.loops.push((brk, top));
                self.scoped(&w.body)?;
                self.loops.pop();
                self.push(I::Br(self.depth - top));
                self.close();
                self.close();
                Ok(())
            }
            Statement::DoWhileStatement(w) => {
                self.open(I::Block(BlockType::Empty));
                let brk = self.depth;
                self.open(I::Loop(BlockType::Empty));
                let top = self.depth;
                self.open(I::Block(BlockType::Empty));
                let cont = self.depth;
                self.loops.push((brk, cont));
                self.scoped(&w.body)?;
                self.loops.pop();
                self.close();
                self.expr_as(&w.test, Ty::Bool)?;
                self.push(I::BrIf(self.depth - top));
                self.close();
                self.close();
                Ok(())
            }
            Statement::BreakStatement(b) => {
                if b.label.is_some() {
                    return err(
                        codes::UNSUPPORTED,
                        b.span,
                        "labelled `break` is not supported",
                        "move the inner loop into a function and `return` from it",
                    );
                }
                let Some(&(brk, _)) = self.loops.last() else {
                    return err(
                        codes::UNSUPPORTED,
                        b.span,
                        "`break` outside a loop",
                        "`switch` is not supported; use `if`/`else`",
                    );
                };
                self.push(I::Br(self.depth - brk));
                Ok(())
            }
            Statement::ContinueStatement(c) => {
                if c.label.is_some() {
                    return err(
                        codes::UNSUPPORTED,
                        c.span,
                        "labelled `continue` is not supported",
                        "move the inner loop into a function",
                    );
                }
                let Some(&(_, cont)) = self.loops.last() else {
                    return err(codes::UNSUPPORTED, c.span, "`continue` outside a loop", "");
                };
                self.push(I::Br(self.depth - cont));
                Ok(())
            }
            Statement::ReturnStatement(r) => {
                match (&r.argument, self.ret) {
                    (None, Ty::Void) => {}
                    (Some(e), Ty::Void) => {
                        return type_err(
                            e.span(),
                            format!("`{}` returns void but this returns a value", self.name),
                            "add a return type such as `: number` to the function",
                        );
                    }
                    (None, t) => {
                        return type_err(
                            r.span,
                            format!("`{}` must return a {}", self.name, t.name()),
                            "return a value",
                        );
                    }
                    (Some(e), t) => self.expr_as(e, t)?,
                }
                self.push(I::Return);
                Ok(())
            }
            Statement::FunctionDeclaration(f) => err(
                codes::UNSUPPORTED,
                f.span,
                "functions cannot be declared inside functions",
                "move it to the top level of the file",
            ),
            Statement::SwitchStatement(s) => err(
                codes::UNSUPPORTED,
                s.span,
                "`switch` is not supported",
                "use `if` / `else if` chains",
            ),
            Statement::ForOfStatement(s) => err(
                codes::UNSUPPORTED,
                s.span,
                "`for...of` is not supported",
                "use `for (let i = 0; i < a.length; i++)`",
            ),
            Statement::ForInStatement(s) => err(
                codes::UNSUPPORTED,
                s.span,
                "`for...in` is not supported",
                "use `for (let i = 0; i < a.length; i++)`",
            ),
            Statement::ThrowStatement(s) => err(
                codes::UNSUPPORTED,
                s.span,
                "`throw` is not supported in job code",
                "write an error flag into an output array, or return early",
            ),
            Statement::TryStatement(s) => err(
                codes::UNSUPPORTED,
                s.span,
                "`try` is not supported in job code",
                "job code cannot throw; remove the try block",
            ),
            other => err(
                codes::UNSUPPORTED,
                other.span(),
                "this statement is not part of job TypeScript",
                "see docs/ai/JOBS-TS.md for the supported statements",
            ),
        }
    }

    fn var_decl(&mut self, v: &VariableDeclaration) -> R<()> {
        let mutable = match v.kind {
            VariableDeclarationKind::Let => true,
            VariableDeclarationKind::Const => false,
            _ => {
                return err(
                    codes::UNSUPPORTED,
                    v.span,
                    "only `let` and `const` declare variables",
                    "replace `var` with `let`",
                );
            }
        };
        for d in &v.declarations {
            let BindingPattern::BindingIdentifier(id) = &d.id else {
                return err(
                    codes::UNSUPPORTED,
                    d.span,
                    "destructuring is not supported",
                    "declare one variable per value",
                );
            };
            let name = id.name.to_string();
            let declared = match &d.type_annotation {
                Some(t) => Some(local_type(&t.type_annotation, t.span, &name)?),
                None => None,
            };
            let (ty, inferred) = match (declared, &d.init) {
                (Some(t), _) => (t, None),
                (None, None) => {
                    return type_err(
                        d.span,
                        format!("`{name}` needs a type or a starting value"),
                        "write `let x: number = 0;` or `let i: i32 = 0;`",
                    );
                }
                (None, Some(init)) => match self.infer(init)? {
                    Ty::I32 | Ty::IntLit if self.widen.contains(&d.span.start) => {
                        (Ty::F64, Some(d.span.start))
                    }
                    Ty::I32 | Ty::IntLit => (Ty::I32, Some(d.span.start)),
                    Ty::Arr(_) => {
                        return err(
                            codes::UNSUPPORTED,
                            d.span,
                            format!("`{name}` would be a second name for an array"),
                            "use the parameter directly",
                        );
                    }
                    Ty::Void => {
                        return type_err(
                            init.span(),
                            "this call returns nothing",
                            "give the function a return type",
                        );
                    }
                    t => (t, None),
                },
            };
            let local = self.new_local(ty.val());
            match &d.init {
                Some(init) => self.assign_value(init, ty, inferred, &name)?,
                None => self.push(match ty {
                    Ty::F64 => I::F64Const(0.0.into()),
                    _ => I::I32Const(0),
                }),
            }
            self.push(I::LocalSet(local));
            self.scopes.last_mut().expect("scope").insert(
                name,
                Var::Scalar {
                    local,
                    ty,
                    mutable,
                    inferred,
                },
            );
        }
        Ok(())
    }

    /// Emits `value` converted to the variable type `ty`; widens an inferred `i32`.
    fn assign_value(&mut self, value: &Expression, ty: Ty, inferred: Option<u32>, name: &str) -> R<()> {
        if ty == Ty::I32 && self.infer(value)? == Ty::F64 {
            if let Some(at) = inferred {
                return Err(Fail::Widen(at));
            }
            return type_err(
                value.span(),
                format!("`{name}` is i32 but this value is a number (64-bit float)"),
                &format!("convert with `(value) | 0` (truncates toward zero), or declare `{name}: number`"),
            );
        }
        self.expr_as(value, ty)
    }

    fn expr_stmt(&mut self, e: &Expression) -> R<()> {
        match e {
            Expression::AssignmentExpression(a) => self.assign(a),
            Expression::UpdateExpression(u) => self.update(u),
            Expression::CallExpression(c) => {
                let t = self.call(c, None)?;
                if t != Ty::Void {
                    self.push(I::Drop);
                }
                Ok(())
            }
            Expression::ParenthesizedExpression(p) => self.expr_stmt(&p.expression),
            Expression::SequenceExpression(s) => {
                for e in &s.expressions {
                    self.expr_stmt(e)?;
                }
                Ok(())
            }
            other => err(
                codes::UNSUPPORTED,
                other.span(),
                "this expression does nothing as a statement",
                "assign it to a variable or array element",
            ),
        }
    }

    fn scalar_target(&self, id: &IdentifierReference) -> R<(u32, Ty, Option<u32>)> {
        match self.lookup(id.name.as_str()) {
            Some(Var::Scalar {
                local,
                ty,
                mutable,
                inferred,
            }) => {
                if !mutable {
                    return type_err(
                        id.span,
                        format!("`{}` is a const", id.name),
                        "declare it with `let`",
                    );
                }
                Ok((local, ty, inferred))
            }
            Some(Var::Array { .. }) => type_err(
                id.span,
                format!("array `{}` cannot be reassigned", id.name),
                "assign to its elements: `a[i] = v`",
            ),
            None if self.m.consts.contains_key(id.name.as_str()) => {
                type_err(id.span, format!("`{}` is a constant", id.name), "")
            }
            None => err(
                codes::UNKNOWN_NAME,
                id.span,
                format!("`{}` is not declared", id.name),
                "declare it with `let` first",
            ),
        }
    }

    fn assign(&mut self, a: &AssignmentExpression) -> R<()> {
        let op = match a.operator {
            AssignmentOperator::Assign => None,
            o => match o.to_binary_operator() {
                Some(b) => Some(b),
                None => {
                    return err(
                        codes::UNSUPPORTED,
                        a.span,
                        format!("`{}` is not supported", o.as_str()),
                        "booleans use `x = x && y` / `x = x || y`",
                    );
                }
            },
        };
        match &a.left {
            AssignmentTarget::AssignmentTargetIdentifier(id) => {
                let (local, ty, inferred) = self.scalar_target(id)?;
                match op {
                    None => self.assign_value(&a.right, ty, inferred, id.name.as_str())?,
                    Some(op) => {
                        let rt = self.infer(&a.right)?;
                        let result = self.binary_type(op, ty, rt, a.span)?;
                        if ty == Ty::I32 && result == Ty::F64 {
                            if let Some(at) = inferred {
                                return Err(Fail::Widen(at));
                            }
                            return type_err(
                                a.span,
                                format!(
                                    "`{}` is i32 but `{}` makes a number (64-bit float)",
                                    id.name,
                                    a.operator.as_str()
                                ),
                                &format!("declare `{}: number`, or convert the result with `| 0`", id.name),
                            );
                        }
                        if ty == Ty::Bool {
                            return type_err(a.span, format!("`{}` on a boolean", a.operator.as_str()), "");
                        }
                        let opty = self.operand_type(op, ty, rt);
                        self.push(I::LocalGet(local));
                        self.convert(ty, opty, a.span)?;
                        self.expr_as(&a.right, opty)?;
                        let out = self.binary_op(op, opty, a.span)?;
                        self.convert(out, ty, a.span)?;
                    }
                }
                self.push(I::LocalSet(local));
                Ok(())
            }
            AssignmentTarget::ComputedMemberExpression(m) => {
                let (addr, elem) = self.element_addr(m)?;
                match op {
                    None => {
                        self.push(I::LocalGet(addr));
                        self.store_value(&a.right, elem)?;
                    }
                    Some(op) => {
                        let lt = elem.read_ty();
                        let rt = self.infer(&a.right)?;
                        let opty = self.operand_type(op, lt, rt);
                        self.push(I::LocalGet(addr));
                        self.push(I::LocalGet(addr));
                        self.load(elem);
                        self.convert(lt, opty, a.span)?;
                        self.expr_as(&a.right, opty)?;
                        let out = self.binary_op(op, opty, a.span)?;
                        self.store_top(out, elem);
                    }
                }
                Ok(())
            }
            AssignmentTarget::StaticMemberExpression(m) => type_err(
                m.span,
                format!("`.{}` cannot be assigned", m.property.name),
                "only array elements and variables can be assigned",
            ),
            other => err(
                codes::UNSUPPORTED,
                other.span(),
                "this assignment target is not supported",
                "assign to a variable or `array[index]`",
            ),
        }
    }

    fn update(&mut self, u: &UpdateExpression) -> R<()> {
        let delta = |ty: Ty| -> I<'static> {
            match ty {
                Ty::F64 => I::F64Const(1.0.into()),
                _ => I::I32Const(1),
            }
        };
        let op = |ty: Ty, inc: bool| -> I<'static> {
            match (ty, inc) {
                (Ty::F64, true) => I::F64Add,
                (Ty::F64, false) => I::F64Sub,
                (_, true) => I::I32Add,
                (_, false) => I::I32Sub,
            }
        };
        let inc = u.operator == UpdateOperator::Increment;
        match &u.argument {
            SimpleAssignmentTarget::AssignmentTargetIdentifier(id) => {
                let (local, ty, _) = self.scalar_target(id)?;
                if ty == Ty::Bool {
                    return type_err(u.span, "`++`/`--` on a boolean", "");
                }
                self.push(I::LocalGet(local));
                self.push(delta(ty));
                self.push(op(ty, inc));
                self.push(I::LocalSet(local));
                Ok(())
            }
            SimpleAssignmentTarget::ComputedMemberExpression(m) => {
                let (addr, elem) = self.element_addr(m)?;
                let ty = elem.read_ty();
                self.push(I::LocalGet(addr));
                self.push(I::LocalGet(addr));
                self.load(elem);
                self.push(delta(ty));
                self.push(op(ty, inc));
                self.store_top(ty, elem);
                Ok(())
            }
            other => err(
                codes::UNSUPPORTED,
                other.span(),
                "`++`/`--` needs a variable or array element",
                "",
            ),
        }
    }

    // ---------------------------------------------------------------- arrays

    fn array_of(&self, e: &Expression) -> R<(u32, u32, Elem)> {
        match e {
            Expression::Identifier(id) => match self.lookup(id.name.as_str()) {
                Some(Var::Array { ptr, len, elem }) => Ok((ptr, len, elem)),
                Some(_) => type_err(
                    id.span,
                    format!("`{}` is not an array", id.name),
                    "only Float32Array, Float64Array and Int32Array parameters can be indexed",
                ),
                None => err(
                    codes::UNKNOWN_NAME,
                    id.span,
                    format!("`{}` is not declared", id.name),
                    "",
                ),
            },
            Expression::ParenthesizedExpression(p) => self.array_of(&p.expression),
            other => err(
                codes::UNSUPPORTED,
                other.span(),
                "only array parameters can be indexed",
                "index a parameter directly: `a[i]`",
            ),
        }
    }

    /// Emits the bounds-checked byte address of `m` into a new local; returns it.
    fn element_addr(&mut self, m: &ComputedMemberExpression) -> R<(u32, Elem)> {
        let (ptr, len, elem) = self.array_of(&m.object)?;
        let it = self.infer(&m.expression)?;
        if !it.is_int() {
            return type_err(
                m.expression.span(),
                format!("array index must be an i32, but it is {}", it.name()),
                "count with an integer (`for (let i = 0; ...)`), or convert with `(x) | 0`",
            );
        }
        self.expr_as(&m.expression, Ty::I32)?;
        let idx = self.new_local(ValType::I32);
        if !self.m.safety.checked() {
            self.push(I::LocalSet(idx));
        } else {
            self.push(I::LocalTee(idx));
            self.push(I::LocalGet(len));
            self.push(I::I32GeU);
            self.open(I::If(BlockType::Empty));
            let line = self.line(m.span.start);
            self.push(I::I32Const(trap::INDEX));
            self.push(I::I32Const(line));
            self.push(I::LocalGet(idx));
            self.push(I::LocalGet(len));
            self.push(I::Call(F_TRAP));
            self.push(I::Unreachable);
            self.close();
        }
        self.push(I::LocalGet(ptr));
        self.push(I::LocalGet(idx));
        self.push(I::I32Const(elem.shift()));
        self.push(I::I32Shl);
        self.push(I::I32Add);
        let addr = self.new_local(ValType::I32);
        self.push(I::LocalSet(addr));
        Ok((addr, elem))
    }

    fn load(&mut self, elem: Elem) {
        match elem {
            Elem::F32 => {
                self.push(I::F32Load(mem(2)));
                self.push(I::F64PromoteF32);
            }
            Elem::F64 => self.push(I::F64Load(mem(3))),
            Elem::I32 => self.push(I::I32Load(mem(2))),
        }
    }

    /// Stores the value on top of the stack (of type `ty`) into the address below it, with
    /// JavaScript's conversion for the array type.
    fn store_top(&mut self, ty: Ty, elem: Elem) {
        match (elem, ty) {
            (Elem::F32, Ty::F64) => {
                self.push(I::F32DemoteF64);
                self.canonical_nan(ValType::F32);
                self.push(I::F32Store(mem(2)));
            }
            (Elem::F32, _) => {
                self.push(I::F32ConvertI32S);
                self.push(I::F32Store(mem(2)));
            }
            (Elem::F64, Ty::F64) => {
                self.canonical_nan(ValType::F64);
                self.push(I::F64Store(mem(3)));
            }
            (Elem::F64, _) => {
                self.push(I::F64ConvertI32S);
                self.push(I::F64Store(mem(3)));
            }
            (Elem::I32, Ty::F64) => {
                self.push(I::Call(F_TO_INT32));
                self.push(I::I32Store(mem(2)));
            }
            (Elem::I32, _) => self.push(I::I32Store(mem(2))),
        }
    }

    /// Replaces a NaN on top of the stack by the canonical NaN. NaN bits are the one thing
    /// CPUs disagree on (x86 and ARM make different NaNs), and they only become visible once
    /// stored, so stores do this instead of every operation (which costs 2.5x run time).
    fn canonical_nan(&mut self, t: ValType) {
        let v = self.new_local(t);
        self.push(I::LocalTee(v));
        if t == ValType::F32 {
            self.push(I::F32Const(f32::NAN.into()));
            self.push(I::LocalGet(v));
            self.push(I::LocalGet(v));
            self.push(I::F32Eq);
        } else {
            self.push(I::F64Const(f64::NAN.into()));
            self.push(I::LocalGet(v));
            self.push(I::LocalGet(v));
            self.push(I::F64Eq);
        }
        // select(v, NaN, v == v): v unless v is NaN.
        self.push(I::Select);
    }

    fn store_value(&mut self, value: &Expression, elem: Elem) -> R<()> {
        let vt = self.infer(value)?;
        let t = match (elem, vt) {
            (_, Ty::F64) => Ty::F64,
            (_, t) if t.is_int() => Ty::I32,
            (_, t) => {
                return type_err(
                    value.span(),
                    format!("cannot store a {} in a {}", t.name(), elem.ts_name()),
                    "store a number",
                );
            }
        };
        // Float arrays: store integers as numbers (exactly as JavaScript would).
        let t = if elem != Elem::I32 && t == Ty::I32 && vt == Ty::IntLit {
            Ty::F64
        } else {
            t
        };
        self.expr_as(value, t)?;
        self.store_top(t, elem);
        Ok(())
    }

    // ---------------------------------------------------------------- expressions

    /// The type an expression has, without emitting anything.
    fn infer(&self, e: &Expression) -> R<Ty> {
        Ok(match e {
            Expression::NumericLiteral(n) => {
                if number_literal(n).int_like {
                    Ty::IntLit
                } else {
                    Ty::F64
                }
            }
            Expression::BooleanLiteral(_) => Ty::Bool,
            Expression::ParenthesizedExpression(p) => self.infer(&p.expression)?,
            Expression::Identifier(id) => match id.name.as_str() {
                n if self.lookup(n).is_some() => match self.lookup(n).expect("found") {
                    Var::Scalar { ty, .. } => ty,
                    Var::Array { elem, .. } => Ty::Arr(elem),
                },
                n if self.m.consts.contains_key(n) => {
                    if self.m.consts[n].int_like {
                        Ty::IntLit
                    } else {
                        Ty::F64
                    }
                }
                "Infinity" | "NaN" => Ty::F64,
                _ => return self.unknown(id),
            },
            Expression::UnaryExpression(u) => match u.operator {
                UnaryOperator::UnaryNegation | UnaryOperator::UnaryPlus => {
                    let t = self.infer(&u.argument)?;
                    if !t.is_num() {
                        return type_err(
                            u.span,
                            format!("`{}` needs a number, got {}", u.operator.as_str(), t.name()),
                            "",
                        );
                    }
                    // `-0` must stay a float zero.
                    if t == Ty::IntLit
                        && matches!(&u.argument, Expression::NumericLiteral(n) if n.value == 0.0)
                        && u.operator == UnaryOperator::UnaryNegation
                    {
                        Ty::F64
                    } else {
                        t
                    }
                }
                UnaryOperator::LogicalNot => Ty::Bool,
                UnaryOperator::BitwiseNot => Ty::I32,
                _ => return self.unsupported(e),
            },
            Expression::BinaryExpression(b) => {
                let l = self.infer(&b.left)?;
                let r = self.infer(&b.right)?;
                self.binary_type(b.operator, l, r, b.span)?
            }
            Expression::LogicalExpression(_) => Ty::Bool,
            Expression::ConditionalExpression(c) => {
                let a = self.infer(&c.consequent)?;
                let b = self.infer(&c.alternate)?;
                unify(a, b).ok_or(()).or_else(|_| {
                    type_err(
                        c.span,
                        format!(
                            "the two branches have different types ({} and {})",
                            a.name(),
                            b.name()
                        ),
                        "make both branches numbers or both booleans",
                    )
                })?
            }
            Expression::CallExpression(c) => self.call_type(c)?,
            Expression::ComputedMemberExpression(m) => {
                let (_, _, elem) = self.array_of(&m.object)?;
                elem.read_ty()
            }
            Expression::StaticMemberExpression(m) => self.static_member_type(m)?,
            _ => return self.unsupported(e),
        })
    }

    fn unknown<T>(&self, id: &IdentifierReference) -> R<T> {
        let n = id.name.as_str();
        let fix = match n {
            "undefined" | "null" => "job code has no undefined or null; use a number such as -1 for \"none\"",
            "console" => "no logging inside jobs; write values into an output array",
            "Number" | "String" | "Object" | "Array" | "JSON" | "Date" | "performance" => {
                "only numbers, typed array parameters and `Math` are available"
            }
            "engine" => "the `engine` object belongs to scripts; pass what the job needs as parameters",
            _ if self.m.by_name.contains_key(n) => "functions can only be called, not used as values",
            _ => "declare it with `let`/`const`, make it a parameter, or a top-level `const NAME = <number>`",
        };
        err(
            codes::UNKNOWN_NAME,
            id.span,
            format!("`{n}` is not available in job code"),
            fix,
        )
    }

    fn unsupported<T>(&self, e: &Expression) -> R<T> {
        let (what, fix) = match e {
            Expression::StringLiteral(_) | Expression::TemplateLiteral(_) => {
                ("strings", "job code works on numbers only; use numeric ids")
            }
            Expression::ArrayExpression(_) | Expression::NewExpression(_) => (
                "allocation (`[]`, `new`)",
                "job code cannot allocate; pass every array in as a parameter",
            ),
            Expression::ObjectExpression(_) => (
                "objects",
                "split the data into one typed array per field (structure of arrays)",
            ),
            Expression::ArrowFunctionExpression(_) | Expression::FunctionExpression(_) => {
                ("closures", "declare a top-level function and call it")
            }
            Expression::ThisExpression(_) | Expression::ClassExpression(_) | Expression::Super(_) => {
                ("classes and `this`", "use top-level functions with parameters")
            }
            Expression::AwaitExpression(_) | Expression::YieldExpression(_) => {
                ("`await`/`yield`", "job code is synchronous")
            }
            Expression::NullLiteral(_) => ("`null`", "use a number such as -1 for \"none\""),
            Expression::AssignmentExpression(_) => {
                ("assignment inside an expression", "assign in its own statement")
            }
            Expression::UpdateExpression(_) => (
                "`++`/`--` inside an expression",
                "increment in its own statement: `a[i] = v; i++;`",
            ),
            Expression::TSAsExpression(_)
            | Expression::TSSatisfiesExpression(_)
            | Expression::TSTypeAssertion(_) => (
                "type assertions",
                "convert explicitly: `x | 0` makes an i32; i32 values widen to number by themselves",
            ),
            Expression::TSNonNullExpression(_) | Expression::ChainExpression(_) => {
                ("`!`/`?.` on values", "job values are never null; remove it")
            }
            Expression::SequenceExpression(_) => ("the comma operator", "write separate statements"),
            Expression::UnaryExpression(u) => match u.operator {
                UnaryOperator::Typeof => ("`typeof`", "types are fixed at compile time"),
                _ => ("`delete`/`void`", ""),
            },
            Expression::BigIntLiteral(_) => ("BigInt", "use i32 or number"),
            Expression::RegExpLiteral(_) => ("regular expressions", "job code works on numbers only"),
            _ => (
                "this expression",
                "see docs/ai/JOBS-TS.md for the supported expressions",
            ),
        };
        err(
            codes::UNSUPPORTED,
            e.span(),
            format!("{what} cannot be used in job code"),
            fix,
        )
    }

    fn static_member_type(&self, m: &StaticMemberExpression) -> R<Ty> {
        let prop = m.property.name.as_str();
        if let Expression::Identifier(o) = &m.object
            && o.name.as_str() == "Math"
            && self.lookup("Math").is_none()
        {
            return match math_const(prop) {
                Some(_) => Ok(Ty::F64),
                None => err(
                    codes::UNKNOWN_NAME,
                    m.span,
                    format!("`Math.{prop}` is not a constant"),
                    "constants: Math.PI, E, LN2, LN10, LOG2E, LOG10E, SQRT2, SQRT1_2",
                ),
            };
        }
        let (_, _, _) = self.array_of(&m.object)?;
        if prop == "length" {
            Ok(Ty::I32)
        } else {
            err(
                codes::UNSUPPORTED,
                m.span,
                format!("typed arrays have no `.{prop}` in job code"),
                "only `.length` and indexing are available",
            )
        }
    }

    /// Result type of a binary operator.
    fn binary_type(&self, op: BinaryOperator, l: Ty, r: Ty, span: Span) -> R<Ty> {
        use BinaryOperator as B;
        let both_num = l.is_num() && r.is_num();
        Ok(match op {
            B::Addition | B::Subtraction | B::Multiplication | B::Remainder => {
                if !both_num {
                    return type_err(
                        span,
                        format!(
                            "`{}` needs numbers, got {} and {}",
                            op.as_str(),
                            l.name(),
                            r.name()
                        ),
                        "",
                    );
                }
                match (l, r) {
                    (Ty::IntLit, Ty::IntLit) => Ty::IntLit,
                    (a, b) if a.is_int() && b.is_int() => Ty::I32,
                    _ => Ty::F64,
                }
            }
            B::Division | B::Exponential => {
                if !both_num {
                    return type_err(
                        span,
                        format!(
                            "`{}` needs numbers, got {} and {}",
                            op.as_str(),
                            l.name(),
                            r.name()
                        ),
                        "",
                    );
                }
                Ty::F64
            }
            B::LessThan | B::LessEqualThan | B::GreaterThan | B::GreaterEqualThan => {
                if !both_num {
                    return type_err(
                        span,
                        format!(
                            "`{}` compares numbers, got {} and {}",
                            op.as_str(),
                            l.name(),
                            r.name()
                        ),
                        "",
                    );
                }
                Ty::Bool
            }
            B::Equality | B::Inequality | B::StrictEquality | B::StrictInequality => {
                if !(both_num || (l == Ty::Bool && r == Ty::Bool)) {
                    return type_err(
                        span,
                        format!("`{}` compares {} with {}", op.as_str(), l.name(), r.name()),
                        "compare numbers with numbers or booleans with booleans",
                    );
                }
                Ty::Bool
            }
            B::BitwiseOR | B::BitwiseXOR | B::BitwiseAnd | B::ShiftLeft | B::ShiftRight => {
                if !both_num {
                    return type_err(
                        span,
                        format!(
                            "`{}` needs numbers, got {} and {}",
                            op.as_str(),
                            l.name(),
                            r.name()
                        ),
                        "",
                    );
                }
                Ty::I32
            }
            B::ShiftRightZeroFill => {
                if !both_num {
                    return type_err(
                        span,
                        format!("`>>>` needs numbers, got {} and {}", l.name(), r.name()),
                        "",
                    );
                }
                Ty::F64
            }
            B::In | B::Instanceof => {
                return err(
                    codes::UNSUPPORTED,
                    span,
                    format!("`{}` is not available in job code", op.as_str()),
                    "",
                );
            }
        })
    }

    /// The type both operands are converted to before the operator runs.
    fn operand_type(&self, op: BinaryOperator, l: Ty, r: Ty) -> Ty {
        use BinaryOperator as B;
        match op {
            B::Division | B::Exponential => Ty::F64,
            B::BitwiseOR
            | B::BitwiseXOR
            | B::BitwiseAnd
            | B::ShiftLeft
            | B::ShiftRight
            | B::ShiftRightZeroFill => Ty::I32,
            B::Equality | B::Inequality | B::StrictEquality | B::StrictInequality if l == Ty::Bool => {
                Ty::Bool
            }
            _ => {
                if l.is_int() && r.is_int() {
                    Ty::I32
                } else {
                    Ty::F64
                }
            }
        }
    }

    /// Emits the operator for operands of type `t` already on the stack; returns the result.
    fn binary_op(&mut self, op: BinaryOperator, t: Ty, span: Span) -> R<Ty> {
        use BinaryOperator as B;
        let f = t == Ty::F64;
        let i = match (op, f) {
            (B::Addition, true) => I::F64Add,
            (B::Addition, false) => I::I32Add,
            (B::Subtraction, true) => I::F64Sub,
            (B::Subtraction, false) => I::I32Sub,
            (B::Multiplication, true) => I::F64Mul,
            (B::Multiplication, false) => I::I32Mul,
            (B::Division, _) => I::F64Div,
            (B::Remainder, true) => I::Call(math_index("fmod").expect("fmod")),
            (B::Remainder, false) => {
                // JavaScript gives NaN for `x % 0`; an i32 cannot hold NaN, so stop with an error.
                let b = self.new_local(ValType::I32);
                self.push(I::LocalTee(b));
                self.push(I::I32Eqz);
                self.open(I::If(BlockType::Empty));
                let line = self.line(span.start);
                self.trap(trap::REM_ZERO, line);
                self.close();
                self.push(I::LocalGet(b));
                I::I32RemS
            }
            (B::Exponential, _) => I::Call(math_index("pow").expect("pow")),
            (B::LessThan, true) => I::F64Lt,
            (B::LessThan, false) => I::I32LtS,
            (B::LessEqualThan, true) => I::F64Le,
            (B::LessEqualThan, false) => I::I32LeS,
            (B::GreaterThan, true) => I::F64Gt,
            (B::GreaterThan, false) => I::I32GtS,
            (B::GreaterEqualThan, true) => I::F64Ge,
            (B::GreaterEqualThan, false) => I::I32GeS,
            (B::Equality | B::StrictEquality, true) => I::F64Eq,
            (B::Equality | B::StrictEquality, false) => I::I32Eq,
            (B::Inequality | B::StrictInequality, true) => I::F64Ne,
            (B::Inequality | B::StrictInequality, false) => I::I32Ne,
            (B::BitwiseOR, _) => I::I32Or,
            (B::BitwiseXOR, _) => I::I32Xor,
            (B::BitwiseAnd, _) => I::I32And,
            (B::ShiftLeft, _) => I::I32Shl,
            (B::ShiftRight, _) => I::I32ShrS,
            (B::ShiftRightZeroFill, _) => {
                self.push(I::I32ShrU);
                self.push(I::F64ConvertI32U);
                return Ok(Ty::F64);
            }
            (B::In | B::Instanceof, _) => return err(codes::UNSUPPORTED, span, "operator not available", ""),
        };
        self.push(i);
        Ok(match op {
            B::Division | B::Exponential => Ty::F64,
            B::LessThan
            | B::LessEqualThan
            | B::GreaterThan
            | B::GreaterEqualThan
            | B::Equality
            | B::Inequality
            | B::StrictEquality
            | B::StrictInequality => Ty::Bool,
            _ => t,
        })
    }

    /// Converts the value on the stack from `from` to `to`.
    fn convert(&mut self, from: Ty, to: Ty, span: Span) -> R<()> {
        match (from, to) {
            (a, b) if a == b => {}
            (Ty::IntLit, Ty::I32) | (Ty::I32, Ty::IntLit) => {}
            (Ty::I32 | Ty::IntLit, Ty::F64) => self.push(I::F64ConvertI32S),
            (a, b) => {
                let fix = match (a, b) {
                    (Ty::F64, Ty::I32) => {
                        "convert with `(value) | 0` (truncates toward zero), or use `number`"
                    }
                    (_, Ty::Bool) => "compare explicitly, e.g. `x !== 0`",
                    (Ty::Bool, _) => "use `cond ? 1 : 0`",
                    _ => "",
                };
                return type_err(span, format!("expected {} but found {}", b.name(), a.name()), fix);
            }
        }
        Ok(())
    }

    /// Emits `e` converted to `want`.
    fn expr_as(&mut self, e: &Expression, want: Ty) -> R<()> {
        let got = self.expr(e, Some(want))?;
        self.convert(got, want, e.span())
    }

    /// Emits `e`; `hint` decides the type of integer literals. Returns the type emitted.
    fn expr(&mut self, e: &Expression, hint: Option<Ty>) -> R<Ty> {
        match e {
            Expression::NumericLiteral(n) => Ok(self.literal(number_literal(n), hint)),
            Expression::BooleanLiteral(b) => {
                self.push(I::I32Const(b.value as i32));
                Ok(Ty::Bool)
            }
            Expression::ParenthesizedExpression(p) => self.expr(&p.expression, hint),
            Expression::Identifier(id) => {
                let n = id.name.as_str();
                match self.lookup(n) {
                    Some(Var::Scalar { local, ty, .. }) => {
                        self.push(I::LocalGet(local));
                        Ok(ty)
                    }
                    Some(Var::Array { .. }) => type_err(
                        id.span,
                        format!("array `{n}` used as a value"),
                        "index it (`a[i]`), read `.length`, or pass it to a function",
                    ),
                    None => match (self.m.consts.get(n).copied(), n) {
                        (Some(c), _) => Ok(self.literal(c, hint)),
                        (None, "Infinity") => Ok(self.literal(
                            Const {
                                value: f64::INFINITY,
                                int_like: false,
                            },
                            hint,
                        )),
                        (None, "NaN") => Ok(self.literal(
                            Const {
                                value: f64::NAN,
                                int_like: false,
                            },
                            hint,
                        )),
                        _ => self.unknown(id),
                    },
                }
            }
            Expression::UnaryExpression(u) => {
                let t = self.infer(e)?;
                match u.operator {
                    UnaryOperator::UnaryNegation => {
                        if (t == Ty::IntLit || (t == Ty::F64 && const_value(&u.argument).is_some()))
                            && let Some(c) = const_value(e)
                        {
                            return Ok(self.literal(c, hint));
                        }
                        if t == Ty::I32 || (t == Ty::IntLit && hint != Some(Ty::F64)) {
                            self.push(I::I32Const(0));
                            self.expr_as(&u.argument, Ty::I32)?;
                            self.push(I::I32Sub);
                            Ok(Ty::I32)
                        } else {
                            self.expr_as(&u.argument, Ty::F64)?;
                            self.push(I::F64Neg);
                            Ok(Ty::F64)
                        }
                    }
                    UnaryOperator::UnaryPlus => self.expr(&u.argument, hint),
                    UnaryOperator::LogicalNot => {
                        self.expr_as(&u.argument, Ty::Bool)?;
                        self.push(I::I32Eqz);
                        Ok(Ty::Bool)
                    }
                    UnaryOperator::BitwiseNot => {
                        self.emit_int32(&u.argument)?;
                        self.push(I::I32Const(-1));
                        self.push(I::I32Xor);
                        Ok(Ty::I32)
                    }
                    _ => self.unsupported(e),
                }
            }
            Expression::BinaryExpression(b) => {
                let l = self.infer(&b.left)?;
                let r = self.infer(&b.right)?;
                let result = self.binary_type(b.operator, l, r, b.span)?;
                let mut opty = self.operand_type(b.operator, l, r);
                // Integer literals on both sides follow the hint (`x: number = 1 + 2`).
                if result == Ty::IntLit && hint == Some(Ty::F64) {
                    opty = Ty::F64;
                }
                if opty == Ty::I32
                    && matches!(
                        b.operator,
                        BinaryOperator::BitwiseOR
                            | BinaryOperator::BitwiseXOR
                            | BinaryOperator::BitwiseAnd
                            | BinaryOperator::ShiftLeft
                            | BinaryOperator::ShiftRight
                            | BinaryOperator::ShiftRightZeroFill
                    )
                {
                    self.emit_int32(&b.left)?;
                    self.emit_int32(&b.right)?;
                } else {
                    self.expr_as(&b.left, opty)?;
                    self.expr_as(&b.right, opty)?;
                }
                self.binary_op(b.operator, opty, b.span)
            }
            Expression::LogicalExpression(l) => {
                if l.operator == LogicalOperator::Coalesce {
                    return err(
                        codes::UNSUPPORTED,
                        l.span,
                        "`??` is not available (job values are never null)",
                        "",
                    );
                }
                self.expr_as(&l.left, Ty::Bool)?;
                self.open(I::If(BlockType::Result(ValType::I32)));
                if l.operator == LogicalOperator::And {
                    self.expr_as(&l.right, Ty::Bool)?;
                    self.push(I::Else);
                    self.push(I::I32Const(0));
                } else {
                    self.push(I::I32Const(1));
                    self.push(I::Else);
                    self.expr_as(&l.right, Ty::Bool)?;
                }
                self.close();
                Ok(Ty::Bool)
            }
            Expression::ConditionalExpression(c) => {
                let mut t = self.infer(e)?;
                if t == Ty::IntLit {
                    t = if hint == Some(Ty::F64) { Ty::F64 } else { Ty::I32 };
                }
                if matches!(t, Ty::Arr(_) | Ty::Void) {
                    return type_err(c.span, "the branches must be numbers or booleans", "");
                }
                self.expr_as(&c.test, Ty::Bool)?;
                self.open(I::If(BlockType::Result(t.val())));
                self.expr_as(&c.consequent, t)?;
                self.push(I::Else);
                self.expr_as(&c.alternate, t)?;
                self.close();
                Ok(t)
            }
            Expression::CallExpression(c) => self.call(c, hint),
            Expression::ComputedMemberExpression(m) => {
                let (addr, elem) = self.element_addr(m)?;
                self.push(I::LocalGet(addr));
                self.load(elem);
                Ok(elem.read_ty())
            }
            Expression::StaticMemberExpression(m) => {
                let t = self.static_member_type(m)?;
                if let Expression::Identifier(o) = &m.object
                    && o.name.as_str() == "Math"
                    && self.lookup("Math").is_none()
                {
                    let v = math_const(m.property.name.as_str()).expect("checked");
                    self.push(I::F64Const(v.into()));
                    return Ok(Ty::F64);
                }
                let (_, len, _) = self.array_of(&m.object)?;
                self.push(I::LocalGet(len));
                Ok(t)
            }
            _ => self.unsupported(e),
        }
    }

    fn literal(&mut self, c: Const, hint: Option<Ty>) -> Ty {
        if c.int_like && hint != Some(Ty::F64) {
            self.push(I::I32Const(c.value as i32));
            Ty::I32
        } else {
            self.push(I::F64Const(c.value.into()));
            Ty::F64
        }
    }

    /// Emits `e` as an i32 with JavaScript's ToInt32 (what `|`, `&`, `^`, `<<`, `>>` do).
    fn emit_int32(&mut self, e: &Expression) -> R<()> {
        match self.infer(e)? {
            t if t.is_int() => self.expr_as(e, Ty::I32),
            Ty::F64 => {
                self.expr_as(e, Ty::F64)?;
                self.push(I::Call(F_TO_INT32));
                Ok(())
            }
            t => type_err(
                e.span(),
                format!("bitwise operators need numbers, got {}", t.name()),
                "",
            ),
        }
    }

    // ---------------------------------------------------------------- calls

    fn math_name<'e>(&self, c: &'e CallExpression) -> Option<&'e str> {
        if let Expression::StaticMemberExpression(m) = &c.callee
            && let Expression::Identifier(o) = &m.object
            && o.name.as_str() == "Math"
            && self.lookup("Math").is_none()
        {
            return Some(m.property.name.as_str());
        }
        None
    }

    fn call_type(&self, c: &CallExpression) -> R<Ty> {
        if let Some(name) = self.math_name(c) {
            return Ok(match name {
                "imul" => Ty::I32,
                _ => Ty::F64,
            });
        }
        match &c.callee {
            Expression::Identifier(id) => match self.m.by_name.get(id.name.as_str()) {
                Some(&i) => Ok(self.m.infos[i].returns.ty()),
                None => self.unknown(id),
            },
            other => err(
                codes::UNSUPPORTED,
                other.span(),
                "only functions of this file and `Math.*` can be called",
                "declare the function at the top level of this file",
            ),
        }
    }

    fn args<'e>(&self, c: &'e CallExpression) -> R<Vec<&'e Expression<'e>>> {
        c.arguments
            .iter()
            .map(|a| match a {
                Argument::SpreadElement(s) => err(
                    codes::UNSUPPORTED,
                    s.span,
                    "spread arguments are not supported",
                    "pass each argument",
                ),
                a => Ok(a.to_expression()),
            })
            .collect()
    }

    fn call(&mut self, c: &CallExpression, _hint: Option<Ty>) -> R<Ty> {
        let args = self.args(c)?;
        if let Some(name) = self.math_name(c) {
            return self.math_call(name, &args, c.span);
        }
        let Expression::Identifier(id) = &c.callee else {
            return self.call_type(c).map(|_| Ty::Void);
        };
        let Some(&i) = self.m.by_name.get(id.name.as_str()) else {
            return self.unknown(id);
        };
        let info = &self.m.infos[i];
        if args.len() != info.params.len() {
            return type_err(
                c.span,
                format!(
                    "`{}` takes {} arguments, got {}",
                    info.name,
                    info.params.len(),
                    args.len()
                ),
                "",
            );
        }
        let params = info.params.clone();
        let (index, ret) = (info.index, info.returns.ty());
        for (a, (pname, p)) in args.iter().zip(&params) {
            match p {
                ParamType::Array(elem) => {
                    let (ptr, len, e2) = self.array_of(a)?;
                    if e2 != *elem {
                        return type_err(
                            a.span(),
                            format!(
                                "parameter `{pname}` is a {} but this is a {}",
                                elem.ts_name(),
                                e2.ts_name()
                            ),
                            "",
                        );
                    }
                    self.push(I::LocalGet(ptr));
                    self.push(I::LocalGet(len));
                }
                p => {
                    let want = p.ty();
                    if want == Ty::I32 && self.infer(a)? == Ty::F64 {
                        return type_err(
                            a.span(),
                            format!("parameter `{pname}` is i32 but this is a number"),
                            "convert with `(value) | 0`",
                        );
                    }
                    self.expr_as(a, want)?;
                }
            }
        }
        self.push(I::Call(index));
        Ok(ret)
    }

    fn math_call(&mut self, name: &str, args: &[&Expression], span: Span) -> R<Ty> {
        let arity = |n: usize| -> R<()> {
            if args.len() != n {
                return type_err(
                    span,
                    format!("`Math.{name}` takes {n} argument(s), got {}", args.len()),
                    "",
                );
            }
            Ok(())
        };
        let num_args = |cx: &mut Self| -> R<()> {
            for a in args {
                cx.expr_as(a, Ty::F64)?;
            }
            Ok(())
        };
        match name {
            "sqrt" | "abs" | "floor" | "ceil" | "trunc" => {
                arity(1)?;
                num_args(self)?;
                self.push(match name {
                    "sqrt" => I::F64Sqrt,
                    "abs" => I::F64Abs,
                    "floor" => I::F64Floor,
                    "ceil" => I::F64Ceil,
                    _ => I::F64Trunc,
                });
            }
            "fround" => {
                arity(1)?;
                num_args(self)?;
                self.push(I::F32DemoteF64);
                self.push(I::F64PromoteF32);
            }
            "round" => {
                arity(1)?;
                num_args(self)?;
                let x = self.new_local(ValType::F64);
                let r = self.new_local(ValType::F64);
                self.push(I::LocalTee(x));
                self.push(I::F64Floor);
                self.push(I::LocalTee(r));
                self.push(I::F64Const(1.0.into()));
                self.push(I::F64Add);
                self.push(I::LocalGet(r));
                self.push(I::LocalGet(x));
                self.push(I::LocalGet(r));
                self.push(I::F64Sub);
                self.push(I::F64Const(0.5.into()));
                self.push(I::F64Ge);
                self.push(I::Select);
                self.push(I::LocalGet(x));
                self.push(I::F64Copysign);
            }
            "sign" => {
                arity(1)?;
                num_args(self)?;
                let x = self.new_local(ValType::F64);
                self.push(I::LocalSet(x));
                self.push(I::F64Const(1.0.into()));
                self.push(I::F64Const((-1.0).into()));
                self.push(I::LocalGet(x));
                self.push(I::LocalGet(x));
                self.push(I::F64Const(0.0.into()));
                self.push(I::F64Lt);
                self.push(I::Select);
                self.push(I::LocalGet(x));
                self.push(I::F64Const(0.0.into()));
                self.push(I::F64Gt);
                self.push(I::Select);
            }
            "min" | "max" | "hypot" => {
                if args.is_empty() {
                    return type_err(span, format!("`Math.{name}` needs at least one argument"), "");
                }
                self.expr_as(args[0], Ty::F64)?;
                if name == "hypot" {
                    self.push(I::F64Abs);
                }
                for a in &args[1..] {
                    self.expr_as(a, Ty::F64)?;
                    self.push(match name {
                        "min" => I::F64Min,
                        "max" => I::F64Max,
                        _ => I::Call(math_index("hypot").expect("hypot")),
                    });
                }
            }
            "imul" => {
                arity(2)?;
                self.emit_int32(args[0])?;
                self.emit_int32(args[1])?;
                self.push(I::I32Mul);
                return Ok(Ty::I32);
            }
            "random" => {
                return err(
                    codes::UNKNOWN_NAME,
                    span,
                    "`Math.random` is not available in job code (results must not depend on luck)",
                    "pass random numbers or a seed in through a parameter",
                );
            }
            n if MATH1.contains(&n) => {
                arity(1)?;
                num_args(self)?;
                self.push(I::Call(math_index(n).expect("math")));
            }
            n if MATH2.contains(&n) && n != "fmod" => {
                arity(2)?;
                num_args(self)?;
                self.push(I::Call(math_index(n).expect("math")));
            }
            n => {
                return err(
                    codes::UNKNOWN_NAME,
                    span,
                    format!("`Math.{n}` is not available in job code"),
                    "see docs/ai/JOBS-TS.md for the Math functions jobs can use",
                );
            }
        }
        Ok(Ty::F64)
    }
}

fn unify(a: Ty, b: Ty) -> Option<Ty> {
    match (a, b) {
        (x, y) if x == y => Some(x),
        (x, y) if x.is_int() && y.is_int() => Some(Ty::I32),
        (x, y) if x.is_num() && y.is_num() => Some(Ty::F64),
        _ => None,
    }
}

/// Puts the module together: imports, one memory, the helper and the file's functions.
fn assemble(infos: &[FunctionInfo], bodies: Vec<Body>) -> Vec<u8> {
    let mut types = TypeSection::new();
    let mut type_ids: BTreeMap<(Vec<ValType>, Vec<ValType>), u32> = BTreeMap::new();
    let mut ty = |params: Vec<ValType>, results: Vec<ValType>| -> u32 {
        let n = type_ids.len() as u32;
        *type_ids.entry((params, results)).or_insert(n)
    };
    let t_trap = ty(vec![ValType::I32; 4], vec![]);
    let t_f2i = ty(vec![ValType::F64], vec![ValType::I32]);
    let t_m1 = ty(vec![ValType::F64], vec![ValType::F64]);
    let t_m2 = ty(vec![ValType::F64; 2], vec![ValType::F64]);
    let user_types: Vec<u32> = infos
        .iter()
        .map(|f| {
            let mut params = Vec::new();
            for (_, p) in &f.params {
                match p {
                    ParamType::Array(_) => params.extend([ValType::I32, ValType::I32]),
                    p => params.push(p.ty().val()),
                }
            }
            let results = match f.returns {
                ParamType::Void => vec![],
                r => vec![r.ty().val()],
            };
            ty(params, results)
        })
        .collect();
    let mut ordered: Vec<_> = type_ids.iter().collect();
    ordered.sort_by_key(|(_, id)| **id);
    for ((p, r), _) in ordered {
        types.ty().function(p.iter().copied(), r.iter().copied());
    }

    let mut imports = ImportSection::new();
    imports.import("env", "trap", EntityType::Function(t_trap));
    imports.import("env", "to_int32", EntityType::Function(t_f2i));
    for n in MATH1 {
        imports.import("math", n, EntityType::Function(t_m1));
    }
    for n in MATH2 {
        imports.import("math", n, EntityType::Function(t_m2));
    }

    let mut functions = FunctionSection::new();
    functions.function(t_f2i);
    for t in &user_types {
        functions.function(*t);
    }

    let mut memories = MemorySection::new();
    memories.memory(MemoryType {
        minimum: 1,
        maximum: None,
        memory64: false,
        shared: false,
        page_size_log2: None,
    });

    let mut exports = ExportSection::new();
    exports.export("memory", ExportKind::Memory, 0);
    for f in infos.iter().filter(|f| f.exported) {
        exports.export(&f.name, ExportKind::Func, f.index);
    }

    let mut code = CodeSection::new();
    // ToInt32: fast path when |x| < 2^63 (then truncating and wrapping is exact), host otherwise.
    let mut helper = WFunction::new([]);
    for i in [
        I::LocalGet(0),
        I::F64Abs,
        I::F64Const(9.0e18.into()),
        I::F64Lt,
        I::If(BlockType::Result(ValType::I32)),
        I::LocalGet(0),
        I::I64TruncSatF64S,
        I::I32WrapI64,
        I::Else,
        I::LocalGet(0),
        I::Call(F_TO_INT32_SLOW),
        I::End,
        I::End,
    ] {
        helper.instruction(&i);
    }
    code.function(&helper);
    for b in bodies {
        let mut f = WFunction::new_with_locals_types(b.locals);
        for i in &b.code {
            f.instruction(i);
        }
        code.function(&f);
    }

    let mut module = Module::new();
    module
        .section(&types)
        .section(&imports)
        .section(&functions)
        .section(&memories)
        .section(&exports)
        .section(&code);
    module.finish()
}
