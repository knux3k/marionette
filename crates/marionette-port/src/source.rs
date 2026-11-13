//! Reading source files: classes, fields, methods and engine API call chains, for C# (Unity)
//! and C++ (Unreal), plus Blueprint graphs copied from the Unreal editor as text.

use crate::lex::{Kind, Token, lex, matching};
use crate::map::Engine;
use std::collections::BTreeMap;

/// One segment of a call chain such as `GetWorld()->GetTimerManager().SetTimer(...)`.
#[derive(Debug, Clone)]
pub struct Seg {
    pub name: String,
    /// Followed by `(` (a call) or by `<T>(`.
    pub call: bool,
    /// Followed by `<T>` (a generic): `GetComponent<Rigidbody>()`.
    pub generic: Option<String>,
    /// The call's arguments as compact text (at most 120 characters).
    pub args: Option<String>,
    /// The first argument when it is a string literal, without quotes (`"Horizontal"`).
    pub arg0: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Chain {
    pub segs: Vec<Seg>,
    pub line: u32,
    pub token: usize,
    /// The first segment is itself a member (`(a + b).Normalize()`).
    pub leading_member: bool,
    /// Inside `[ ... ]` right after `[` (a C# attribute).
    pub attribute: bool,
    /// Preceded by a type name, so it declares or defines something (`void Update()`).
    pub declaration: bool,
}

const NOT_TYPES: [&str; 30] = [
    "return",
    "new",
    "else",
    "case",
    "await",
    "yield",
    "throw",
    "in",
    "is",
    "as",
    "delete",
    "co_await",
    "co_return",
    "typeof",
    "sizeof",
    "nameof",
    "out",
    "ref",
    "params",
    "checked",
    "unchecked",
    "lock",
    "using",
    "if",
    "while",
    "for",
    "foreach",
    "switch",
    "when",
    "not",
];

fn is_sep(t: Option<&Token>) -> bool {
    t.is_some_and(|t| t.is(".") || t.is("->") || t.is("::"))
}

/// Joins tokens into compact, readable code text.
pub fn join(tokens: &[Token]) -> String {
    let mut s = String::new();
    let mut prev: Option<&Token> = None;
    for t in tokens {
        let tight_before = matches!(
            t.text.as_str(),
            "." | "->" | "::" | ")" | "]" | "," | ";" | ">" | "(" | "["
        ) && t.kind == Kind::Punct;
        let tight_after = prev.is_some_and(|p| {
            p.kind == Kind::Punct
                && matches!(p.text.as_str(), "." | "->" | "::" | "(" | "[" | "<" | "!" | "~")
        });
        let generic_open = t.is("<") && prev.is_some_and(|p| p.kind == Kind::Ident);
        if !s.is_empty() && !tight_before && !tight_after && !generic_open {
            s.push(' ');
        }
        if t.is("(")
            && prev.is_some_and(|p| p.kind == Kind::Punct && !matches!(p.text.as_str(), ">" | ")" | "]"))
        {
            s.push(' ');
        }
        s.push_str(&t.text);
        prev = Some(t);
    }
    s.replace(" (", "(").replace("( ", "(")
}

fn unquote(s: &str) -> String {
    let s = s.trim_start_matches(['@', '$']);
    s.trim_matches(|c| c == '"' || c == '\'').to_string()
}

/// The first argument of a call opened at `open`, if it is a string literal (also inside
/// Unreal's `TEXT("...")`).
fn first_string_arg(tokens: &[Token], open: usize, close: usize) -> Option<String> {
    let a = tokens.get(open + 1)?;
    if open + 1 >= close {
        return None;
    }
    if a.kind == Kind::Str {
        return Some(unquote(&a.text));
    }
    if a.is("TEXT") && tokens.get(open + 2)?.is("(") {
        let s = tokens.get(open + 3)?;
        if s.kind == Kind::Str {
            return Some(unquote(&s.text));
        }
    }
    None
}

/// Every call chain in a token stream.
pub fn chains(tokens: &[Token]) -> Vec<Chain> {
    let n = tokens.len();
    let mut used = vec![false; n];
    let mut out = Vec::new();
    for i in 0..n {
        if used[i] || tokens[i].kind != Kind::Ident {
            continue;
        }
        let prev = i.checked_sub(1).map(|k| &tokens[k]);
        let leading_member = is_sep(prev) || prev.is_some_and(|p| p.is("?"));
        let attribute = prev.is_some_and(|p| p.is("[") || p.is(","))
            && tokens[..i]
                .iter()
                .rev()
                .take(64)
                .find(|t| t.is("[") || t.is("]") || t.is(";") || t.is("{") || t.is("}"))
                .is_some_and(|t| t.is("["));
        let declaration =
            prev.is_some_and(|p| p.kind == Kind::Ident && !NOT_TYPES.contains(&p.text.as_str()));
        let mut segs = vec![Seg {
            name: tokens[i].text.clone(),
            call: false,
            generic: None,
            args: None,
            arg0: None,
        }];
        used[i] = true;
        let mut j = i + 1;
        loop {
            // Separators: `.`, `->`, `::`, and C#'s `?.`.
            let skip = if is_sep(tokens.get(j)) {
                1
            } else if tokens.get(j).is_some_and(|t| t.is("?")) && tokens.get(j + 1).is_some_and(|t| t.is("."))
            {
                2
            } else {
                0
            };
            if skip > 0 {
                if let Some(t) = tokens.get(j + skip).filter(|t| t.kind == Kind::Ident) {
                    used[j + skip] = true;
                    segs.push(Seg {
                        name: t.text.clone(),
                        call: false,
                        generic: None,
                        args: None,
                        arg0: None,
                    });
                    j += skip + 1;
                    continue;
                }
                break;
            }
            let last = segs.last_mut().expect("non-empty");
            if tokens.get(j).is_some_and(|t| t.is("<")) {
                match matching(tokens, j) {
                    Some(close) if close - j <= 16 => {
                        last.generic = tokens[j + 1..close]
                            .iter()
                            .rfind(|t| {
                                t.kind == Kind::Ident
                                    && !matches!(t.text.as_str(), "class" | "const" | "TObjectPtr")
                            })
                            .map(|t| t.text.clone());
                        for (k, u) in used.iter_mut().enumerate().take(close + 1).skip(j) {
                            // Generic arguments are types, not chains of their own.
                            let _ = k;
                            *u = true;
                        }
                        j = close + 1;
                        if !tokens.get(j).is_some_and(|t| t.is("(")) {
                            break;
                        }
                    }
                    _ => break,
                }
            }
            if tokens.get(j).is_some_and(|t| t.is("(")) {
                let Some(close) = matching(tokens, j) else { break };
                last.call = true;
                let mut args = join(&tokens[j + 1..close]);
                if args.len() > 120 {
                    let mut cut = 117;
                    while !args.is_char_boundary(cut) {
                        cut -= 1;
                    }
                    args.truncate(cut);
                    args.push_str("...");
                }
                last.args = Some(args);
                last.arg0 = first_string_arg(tokens, j, close);
                if is_sep(tokens.get(close + 1))
                    && tokens.get(close + 2).is_some_and(|t| t.kind == Kind::Ident)
                {
                    j = close + 1;
                    continue;
                }
                break;
            }
            break;
        }
        out.push(Chain {
            segs,
            line: tokens[i].line,
            token: i,
            leading_member,
            attribute,
            declaration,
        });
    }
    out
}

// ---------------------------------------------------------------------------------------
// Classes

#[derive(Debug, Clone)]
pub struct RawField {
    pub name: String,
    pub line: u32,
    pub ty: String,
    pub modifiers: Vec<String>,
    /// Attribute names (`SerializeField`) or Unreal property specifiers (`EditAnywhere`).
    pub attributes: Vec<String>,
    /// Initializer tokens as text, if any.
    pub init: Option<String>,
    pub(crate) init_tokens: Vec<Token>,
}

#[derive(Debug, Clone)]
pub struct RawMethod {
    pub name: String,
    pub line: u32,
    pub ret: String,
    pub params: String,
    /// Body as token indices `{`..`}` in `file`, if this file holds the body.
    pub body: Option<(usize, usize)>,
    /// Index into the analysis's file list where the body is.
    pub file: usize,
    pub end_line: u32,
}

#[derive(Debug, Clone)]
pub struct RawClass {
    pub name: String,
    pub engine: Engine,
    pub file: usize,
    pub line: u32,
    pub base: Option<String>,
    pub fields: Vec<RawField>,
    pub methods: Vec<RawMethod>,
    pub blueprint: bool,
    /// Declared with `struct` (C# value type or C++ struct).
    pub is_struct: bool,
    /// C# `partial`: other parts of the class may be in other files.
    pub partial: bool,
    /// C# attributes on the class (`Serializable`), last name segment only.
    pub attributes: Vec<String>,
}

const MODIFIERS: [&str; 22] = [
    "public",
    "private",
    "protected",
    "internal",
    "static",
    "readonly",
    "const",
    "override",
    "virtual",
    "abstract",
    "new",
    "sealed",
    "extern",
    "unsafe",
    "volatile",
    "async",
    "partial",
    "mutable",
    "inline",
    "explicit",
    "constexpr",
    "FORCEINLINE",
];

/// Splits the attributes in `[ ... ]` at `open` into names.
fn attribute_names(tokens: &[Token], open: usize, close: usize) -> Vec<String> {
    let mut names = Vec::new();
    let mut depth = 0;
    let mut expect = true;
    for t in &tokens[open + 1..close] {
        if t.is("(") {
            depth += 1;
        } else if t.is(")") {
            depth -= 1;
        } else if depth == 0 && t.is(",") {
            expect = true;
        } else if depth == 0 && expect && t.kind == Kind::Ident {
            names.push(t.text.clone());
            expect = false;
        }
    }
    names
}

/// Unreal specifiers inside `UPROPERTY( ... )`.
fn specifier_names(tokens: &[Token], open: usize, close: usize) -> Vec<String> {
    let mut names = Vec::new();
    let mut depth = 0;
    let mut expect = true;
    for t in &tokens[open + 1..close] {
        if t.is("(") {
            depth += 1;
        } else if t.is(")") {
            depth -= 1;
        } else if depth == 0 && t.is(",") {
            expect = true;
        } else if depth == 0 && expect && t.kind == Kind::Ident {
            names.push(t.text.clone());
            expect = false;
        }
    }
    names
}

fn find_at_depth0(tokens: &[Token], from: usize, to: usize, wanted: &[&str]) -> Option<usize> {
    let mut depth = 0i32;
    let mut k = from;
    while k < to {
        let t = &tokens[k];
        if t.kind != Kind::Str {
            if depth == 0 && wanted.iter().any(|w| t.is(w)) {
                return Some(k);
            }
            if t.is("(") || t.is("[") {
                depth += 1;
            } else if t.is(")") || t.is("]") {
                depth -= 1;
            } else if t.is("<")
                && let Some(c) = matching(tokens, k)
            {
                k = c + 1;
                continue;
            }
        }
        k += 1;
    }
    None
}

/// The `(` that opens a method's parameter list in a member header, if any. A `(` right
/// after the modifiers, or after anything but a name, opens a C# tuple type
/// (`(int a, int b) Split(..)`, `static readonly (string, float)[] table = ..`), not a
/// parameter list, so it is skipped together with its contents; so are generic arguments.
fn parameter_paren(header: &[Token], h: usize) -> Option<usize> {
    let mut k = h;
    while k < header.len() {
        let t = &header[k];
        if t.is("<") {
            k = matching(header, k).map_or(k + 1, |c| c + 1);
            continue;
        }
        if t.is("(") {
            let named = k > h && header[k - 1].kind == Kind::Ident;
            if named {
                return Some(k);
            }
            k = matching(header, k).map_or(k + 1, |c| c + 1);
            continue;
        }
        k += 1;
    }
    None
}

/// Parses the members of a class body `{`..`}`.
fn members(
    tokens: &[Token],
    open: usize,
    close: usize,
    cpp: bool,
    file: usize,
) -> (Vec<RawField>, Vec<RawMethod>) {
    let mut fields = Vec::new();
    let mut methods = Vec::new();
    let mut j = open + 1;
    let mut attrs: Vec<String> = Vec::new();
    while j < close {
        let t = &tokens[j];
        // C# attributes.
        if !cpp
            && t.is("[")
            && let Some(c) = matching(tokens, j)
        {
            attrs.extend(attribute_names(tokens, j, c));
            j = c + 1;
            continue;
        }
        // Unreal macros and access specifiers.
        if cpp {
            if matches!(t.text.as_str(), "public" | "private" | "protected")
                && tokens.get(j + 1).is_some_and(|t| t.is(":"))
            {
                j += 2;
                continue;
            }
            if t.kind == Kind::Ident
                && t.text.chars().all(|c| c.is_ascii_uppercase() || c == '_')
                && tokens.get(j + 1).is_some_and(|t| t.is("("))
            {
                let c = matching(tokens, j + 1).unwrap_or(j + 1);
                if matches!(t.text.as_str(), "UPROPERTY" | "UFUNCTION") {
                    attrs.push(t.text.clone());
                    attrs.extend(specifier_names(tokens, j + 1, c));
                }
                j = c + 1;
                if tokens.get(j).is_some_and(|t| t.is(";")) {
                    j += 1;
                }
                continue;
            }
        }
        if t.is(";") {
            j += 1;
            continue;
        }
        // Nested types: skip their bodies (they are found separately).
        if matches!(
            t.text.as_str(),
            "class"
                | "struct"
                | "enum"
                | "interface"
                | "delegate"
                | "event"
                | "typedef"
                | "using"
                | "friend"
                | "template"
                | "operator"
        ) {
            let end = find_at_depth0(tokens, j, close, &[";", "{"]).unwrap_or(close);
            j = if tokens[end].is("{") {
                let c = matching(tokens, end).unwrap_or(close);
                if tokens.get(c + 1).is_some_and(|t| t.is(";")) {
                    c + 2
                } else {
                    c + 1
                }
            } else {
                end + 1
            };
            attrs.clear();
            continue;
        }
        let Some(term) = find_at_depth0(tokens, j, close, &[";", "{", "=>", "="]) else {
            break;
        };
        let header = &tokens[j..term];
        let mut modifiers = Vec::new();
        let mut h = 0;
        while h < header.len()
            && header[h].kind == Kind::Ident
            && MODIFIERS.contains(&header[h].text.as_str())
        {
            modifiers.push(header[h].text.clone());
            h += 1;
        }
        let paren = parameter_paren(header, h);
        if let Some(p) = paren {
            // A method (or constructor / destructor).
            let name_tok = p.checked_sub(1).map(|k| &header[k]);
            let name = name_tok.map(|t| t.text.clone()).unwrap_or_default();
            let ret = join(&header[h..p.saturating_sub(1)]);
            let pclose = matching(tokens, j + p).unwrap_or(term);
            let params = join(&tokens[j + p + 1..pclose]);
            let line = name_tok.map_or(t.line, |t| t.line);
            let (body, next, end_line) = if tokens[term].is("{") {
                let c = matching(tokens, term).unwrap_or(close);
                (Some((term, c)), c + 1, tokens[c].line)
            } else if tokens[term].is("=>") {
                let e = find_at_depth0(tokens, term, close, &[";"]).unwrap_or(close);
                (Some((term, e)), e + 1, tokens[e].line)
            } else if tokens[term].is("=") {
                // `= 0;` / `= default;` / `= delete;`
                let e = find_at_depth0(tokens, term, close, &[";"]).unwrap_or(close);
                (None, e + 1, tokens[e].line)
            } else {
                (None, term + 1, tokens[term].line)
            };
            // C++ constructor initializer lists put `:` before `{`; handled because `{` is
            // the terminator. Destructors keep their `~` in the name.
            let name = if j + p >= 2 && tokens[j + p - 2].is("~") {
                format!("~{name}")
            } else {
                name
            };
            if !name.is_empty() && name != "GENERATED_BODY" {
                methods.push(RawMethod {
                    name,
                    line,
                    ret,
                    params,
                    body,
                    file,
                    end_line,
                });
            }
            j = next;
        } else {
            // Fields (possibly several declarators) or a C# property.
            let is_property = !cpp && tokens[term].is("{");
            let decl_end = if tokens[term].is("{") {
                let c = matching(tokens, term).unwrap_or(close);
                if !cpp {
                    // `int X { get; set; } = 5;`
                    if tokens.get(c + 1).is_some_and(|t| t.is("=")) {
                        find_at_depth0(tokens, c + 1, close, &[";"]).unwrap_or(close)
                    } else {
                        c
                    }
                } else {
                    find_at_depth0(tokens, c, close, &[";"]).unwrap_or(c)
                }
            } else {
                find_at_depth0(tokens, term, close, &[";"]).unwrap_or(close)
            };
            if !is_property || attrs.iter().any(|a| a == "field") {
                let decl = &tokens[j + h..decl_end];
                // Split at depth-0 commas.
                let mut parts: Vec<&[Token]> = Vec::new();
                let mut start = 0;
                let mut depth = 0i32;
                for (k, tk) in decl.iter().enumerate() {
                    if tk.is("(") || tk.is("{") || tk.is("[") {
                        depth += 1;
                    } else if tk.is(")") || tk.is("}") || tk.is("]") {
                        depth -= 1;
                    } else if tk.is("<") {
                        // Generic commas (Dictionary<int, string>) are inside < >.
                        depth += 1;
                    } else if tk.is(">") {
                        depth -= 1;
                    } else if tk.is(">>") {
                        depth -= 2;
                    } else if depth == 0 && tk.is(",") {
                        parts.push(&decl[start..k]);
                        start = k + 1;
                    }
                }
                parts.push(&decl[start..]);
                let mut ty = String::new();
                for (pi, part) in parts.iter().enumerate() {
                    let eq = part.iter().position(|t| t.is("=")).or_else(|| {
                        if cpp {
                            part.iter().position(|t| t.is("{"))
                        } else {
                            None
                        }
                    });
                    let head = &part[..eq.unwrap_or(part.len())];
                    let Some(name_idx) = head.iter().rposition(|t| t.kind == Kind::Ident) else {
                        continue;
                    };
                    if pi == 0 {
                        ty = join(&head[..name_idx]);
                    }
                    let init_tokens: Vec<Token> = match eq {
                        Some(e) if part[e].is("{") => part[e + 1..part.len().saturating_sub(1)].to_vec(),
                        Some(e) => part[e + 1..].to_vec(),
                        None => Vec::new(),
                    };
                    let init = (!init_tokens.is_empty()).then(|| join(&init_tokens));
                    if ty.is_empty() {
                        continue;
                    }
                    fields.push(RawField {
                        name: head[name_idx].text.clone(),
                        line: head[name_idx].line,
                        ty: ty.clone(),
                        modifiers: modifiers.clone(),
                        attributes: attrs.clone(),
                        init,
                        init_tokens,
                    });
                }
            }
            j = decl_end + 1;
        }
        attrs.clear();
    }
    (fields, methods)
}

/// Attribute names in the `[ ... ]` groups right before the class keyword at `kw` (modifiers
/// may sit between them and the keyword).
fn class_attributes(tokens: &[Token], kw: usize) -> Vec<String> {
    let mut k = kw;
    while k > 0
        && tokens[k - 1].kind == Kind::Ident
        && matches!(
            tokens[k - 1].text.as_str(),
            "public"
                | "internal"
                | "private"
                | "protected"
                | "static"
                | "sealed"
                | "abstract"
                | "partial"
                | "readonly"
                | "ref"
                | "unsafe"
                | "new"
        )
    {
        k -= 1;
    }
    let mut names = Vec::new();
    while k > 0 && tokens[k - 1].is("]") {
        let close = k - 1;
        let mut depth = 0i32;
        let mut open = None;
        for j in (0..=close).rev() {
            if tokens[j].is("]") {
                depth += 1;
            } else if tokens[j].is("[") {
                depth -= 1;
                if depth == 0 {
                    open = Some(j);
                    break;
                }
            }
        }
        let Some(open) = open else { break };
        names.extend(attribute_names(tokens, open, close));
        k = open;
    }
    names
}

/// An enum declared in C# source: members with their integer values.
#[derive(Debug, Clone)]
pub struct RawEnum {
    pub name: String,
    pub line: u32,
    /// Underlying integer type (`int` unless declared).
    pub underlying: String,
    pub members: Vec<(String, i64)>,
}

/// Evaluates a C# enum member initializer: integers (decimal, hex), `<<`, `|`, `+`, `-`,
/// parentheses and names of earlier members.
fn eval_enum(tokens: &[Token], known: &[(String, i64)]) -> Option<i64> {
    fn atom(t: &[Token], i: &mut usize, known: &[(String, i64)]) -> Option<i64> {
        let tk = t.get(*i)?;
        *i += 1;
        if tk.is("(") {
            let v = expr(t, i, known)?;
            if !t.get(*i)?.is(")") {
                return None;
            }
            *i += 1;
            return Some(v);
        }
        if tk.is("-") {
            return atom(t, i, known).map(|v| -v);
        }
        if tk.kind == Kind::Number {
            let s = tk.text.trim_end_matches(['u', 'U', 'l', 'L']);
            return match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
                Some(h) => i64::from_str_radix(&h.replace('_', ""), 16).ok(),
                None => s.replace('_', "").parse().ok(),
            };
        }
        if tk.kind == Kind::Ident {
            // `Name` or `Enum.Name`: the last segment names a member.
            let mut name = tk.text.clone();
            while t.get(*i).is_some_and(|x| x.is(".")) {
                name = t.get(*i + 1)?.text.clone();
                *i += 2;
            }
            return known.iter().find(|(n, _)| *n == name).map(|(_, v)| *v);
        }
        None
    }
    fn expr(t: &[Token], i: &mut usize, known: &[(String, i64)]) -> Option<i64> {
        let mut v = atom(t, i, known)?;
        while let Some(op) = t.get(*i) {
            if op.is("|") {
                *i += 1;
                v |= atom(t, i, known)?;
            } else if op.is("+") {
                *i += 1;
                v += atom(t, i, known)?;
            } else if op.is("-") {
                *i += 1;
                v -= atom(t, i, known)?;
            } else if op.is("<") && t.get(*i + 1).is_some_and(|x| x.is("<")) {
                *i += 2;
                v = v.checked_shl(u32::try_from(atom(t, i, known)?).ok()?)?;
            } else {
                break;
            }
        }
        Some(v)
    }
    let mut i = 0;
    let v = expr(tokens, &mut i, known)?;
    (i == tokens.len()).then_some(v)
}

/// C# enums with their members and values. The flag is false when some member's value could
/// not be evaluated.
pub fn enum_decls(tokens: &[Token]) -> Vec<(RawEnum, bool)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        if !tokens[i].is("enum") {
            i += 1;
            continue;
        }
        let Some(name) = tokens.get(i + 1).filter(|t| t.kind == Kind::Ident) else {
            i += 1;
            continue;
        };
        let mut k = i + 2;
        let mut underlying = "int".to_string();
        if tokens.get(k).is_some_and(|t| t.is(":")) {
            if let Some(u) = tokens.get(k + 1) {
                underlying = u.text.clone();
            }
            k += 2;
        }
        if !tokens.get(k).is_some_and(|t| t.is("{")) {
            i += 1;
            continue;
        }
        let close = matching(tokens, k).unwrap_or(tokens.len() - 1);
        let mut members: Vec<(String, i64)> = Vec::new();
        let mut complete = true;
        let mut next = 0i64;
        let body = &tokens[k + 1..close];
        let mut start = 0;
        let mut depth = 0i32;
        let mut parts: Vec<&[Token]> = Vec::new();
        for (j, t) in body.iter().enumerate() {
            if t.is("(") || t.is("[") {
                depth += 1;
            } else if t.is(")") || t.is("]") {
                depth -= 1;
            } else if depth == 0 && t.is(",") {
                parts.push(&body[start..j]);
                start = j + 1;
            }
        }
        parts.push(&body[start..]);
        for part in parts {
            let mut p = part;
            while p.first().is_some_and(|t| t.is("[")) {
                match matching(p, 0) {
                    Some(c) => p = &p[c + 1..],
                    None => break,
                }
            }
            let Some(first) = p.first().filter(|t| t.kind == Kind::Ident) else {
                continue;
            };
            let value = if p.get(1).is_some_and(|t| t.is("=")) {
                match eval_enum(&p[2..], &members) {
                    Some(v) => v,
                    None => {
                        complete = false;
                        next
                    }
                }
            } else {
                next
            };
            members.push((first.text.clone(), value));
            next = value.wrapping_add(1);
        }
        out.push((
            RawEnum {
                name: name.text.clone(),
                line: name.line,
                underlying,
                members,
            },
            complete,
        ));
        i = close + 1;
    }
    out
}

/// Finds classes in C# or C++ tokens. C++ method bodies defined outside the class
/// (`void AFoo::Tick(...) { }`) are returned separately by [`cpp_definitions`].
pub fn classes(tokens: &[Token], engine: Engine, file: usize) -> Vec<RawClass> {
    let cpp = engine == Engine::Unreal;
    let mut out = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let t = &tokens[i];
        if !(t.is("class") || t.is("struct")) || i > 0 && tokens[i - 1].is("enum") {
            i += 1;
            continue;
        }
        // C# `where T : struct` / `where T : class` constraints.
        if !cpp && i > 0 && tokens[i - 1].is(":") {
            i += 1;
            continue;
        }
        // Skip `template <class T>` and `typeof(class)`-like uses.
        if i > 0 && (tokens[i - 1].is("<") || tokens[i - 1].is(",")) {
            i += 1;
            continue;
        }
        let mut k = i + 1;
        // C++: `class MYGAME_API AFoo`, C#: `class Foo<T>`.
        let mut name = None;
        while let Some(tk) = tokens.get(k) {
            if tk.kind == Kind::Ident
                && !(cpp && tk.text.ends_with("_API"))
                && !matches!(tk.text.as_str(), "final" | "sealed")
            {
                name = Some(tk.text.clone());
                k += 1;
                break;
            }
            if tk.kind != Kind::Ident {
                break;
            }
            k += 1;
        }
        let Some(name) = name else {
            i += 1;
            continue;
        };
        if tokens.get(k).is_some_and(|t| t.is("<")) {
            k = matching(tokens, k).map_or(k, |c| c + 1);
        }
        while tokens.get(k).is_some_and(|t| t.is("final") || t.is("sealed")) {
            k += 1;
        }
        let mut base = None;
        if tokens.get(k).is_some_and(|t| t.is(":")) {
            k += 1;
            while tokens
                .get(k)
                .is_some_and(|t| matches!(t.text.as_str(), "public" | "protected" | "private" | "virtual"))
            {
                k += 1;
            }
            // Qualified base: take the last segment (`UnityEngine.MonoBehaviour`).
            let mut last = None;
            while let Some(tk) = tokens.get(k) {
                if tk.kind == Kind::Ident {
                    last = Some(tk.text.clone());
                    k += 1;
                    if is_sep(tokens.get(k)) {
                        k += 1;
                        continue;
                    }
                }
                break;
            }
            base = last;
        }
        // Find the body (skip `where` clauses and further bases).
        let Some(open) = tokens[k..]
            .iter()
            .position(|t| t.is("{") || t.is(";"))
            .map(|p| p + k)
        else {
            break;
        };
        if tokens[open].is(";") {
            // Forward declaration.
            i = open + 1;
            continue;
        }
        let close = matching(tokens, open).unwrap_or(tokens.len() - 1);
        let (fields, methods) = members(tokens, open, close, cpp, file);
        // Modifiers before the keyword (`public sealed partial class`).
        let mut b = i;
        let mut partial = false;
        while b > 0 && tokens[b - 1].kind == Kind::Ident && !cpp {
            b -= 1;
            if tokens[b].is("partial") {
                partial = true;
            }
            if !matches!(
                tokens[b].text.as_str(),
                "public"
                    | "internal"
                    | "private"
                    | "protected"
                    | "static"
                    | "sealed"
                    | "abstract"
                    | "partial"
                    | "readonly"
                    | "ref"
                    | "unsafe"
                    | "new"
            ) {
                break;
            }
        }
        let attributes = if cpp {
            Vec::new()
        } else {
            class_attributes(tokens, i)
        };
        out.push(RawClass {
            name,
            engine,
            file,
            line: t.line,
            base,
            fields,
            methods,
            blueprint: false,
            is_struct: t.is("struct"),
            partial,
            attributes,
        });
        // Continue inside the body too, so nested classes are found.
        i = open + 1;
    }
    out
}

/// C++ out-of-class definitions: `Ret AFoo::Method(params) [const] { ... }`.
pub fn cpp_definitions(tokens: &[Token], file: usize) -> Vec<(String, RawMethod)> {
    let mut out = Vec::new();
    let mut i = 1;
    while i + 2 < tokens.len() {
        if !(tokens[i].is("::") && tokens[i - 1].kind == Kind::Ident) {
            i += 1;
            continue;
        }
        let class = &tokens[i - 1];
        let mut m = i + 1;
        let destructor = tokens[m].is("~");
        if destructor {
            m += 1;
        }
        let Some(method) = tokens.get(m).filter(|t| t.kind == Kind::Ident) else {
            i += 1;
            continue;
        };
        if !tokens.get(m + 1).is_some_and(|t| t.is("(")) {
            i += 1;
            continue;
        }
        // Not a qualified call: the token before the class is a return type, `}` / `;` (a
        // constructor at file level) or the start of the file; and the class is not itself
        // qualified (`Super::BeginPlay`).
        let before = i.checked_sub(2).map(|k| &tokens[k]);
        let def_context = match before {
            None => true,
            Some(b) => {
                (b.kind == Kind::Ident && !NOT_TYPES.contains(&b.text.as_str()))
                    || b.is("}")
                    || b.is(";")
                    || b.is("*")
                    || b.is("&")
                    || b.is(">")
            }
        };
        let Some(pclose) = matching(tokens, m + 1) else {
            break;
        };
        let mut b = pclose + 1;
        while tokens
            .get(b)
            .is_some_and(|t| matches!(t.text.as_str(), "const" | "override" | "noexcept" | "final"))
        {
            b += 1;
        }
        // Constructor initializer list.
        if tokens.get(b).is_some_and(|t| t.is(":")) {
            while b < tokens.len() && !tokens[b].is("{") {
                if tokens[b].is("(") {
                    b = matching(tokens, b).unwrap_or(b);
                }
                b += 1;
            }
        }
        if def_context && class.text != "Super" && tokens.get(b).is_some_and(|t| t.is("{")) {
            let close = matching(tokens, b).unwrap_or(tokens.len() - 1);
            let ret = if before.is_some_and(|b| b.kind == Kind::Ident) {
                before.map(|b| b.text.clone()).unwrap_or_default()
            } else {
                String::new()
            };
            let name = if destructor {
                format!("~{}", method.text)
            } else {
                method.text.clone()
            };
            out.push((
                class.text.clone(),
                RawMethod {
                    name,
                    line: method.line,
                    ret,
                    params: join(&tokens[m + 2..pclose]),
                    body: Some((b, close)),
                    file,
                    end_line: tokens[close].line,
                },
            ));
            i = close + 1;
            continue;
        }
        i += 1;
    }
    out
}

/// Names in a Blueprint graph copied as text (T3D): `MemberName="X"` and custom event
/// names, with their lines.
pub fn blueprint_names(src: &str) -> Vec<(u32, String)> {
    let mut out = Vec::new();
    for (n, line) in src.lines().enumerate() {
        for key in ["MemberName=\"", "CustomFunctionName=\""] {
            let mut rest = line;
            while let Some(p) = rest.find(key) {
                rest = &rest[p + key.len()..];
                if let Some(e) = rest.find('"') {
                    out.push((n as u32 + 1, rest[..e].to_string()));
                    rest = &rest[e..];
                }
            }
        }
    }
    out
}

/// Enum names declared in tokens (`enum State { ... }`, `enum class EState : uint8`).
pub fn enums(tokens: &[Token]) -> Vec<String> {
    let mut out = Vec::new();
    for (i, t) in tokens.iter().enumerate() {
        if t.is("enum") {
            let mut k = i + 1;
            if tokens.get(k).is_some_and(|t| t.is("class") || t.is("struct")) {
                k += 1;
            }
            if let Some(n) = tokens.get(k).filter(|t| t.kind == Kind::Ident) {
                out.push(n.text.clone());
            }
        }
    }
    out
}

/// A lexed file.
pub struct SourceFile {
    pub path: String,
    pub engine: Engine,
    pub blueprint: bool,
    pub tokens: Vec<Token>,
    pub lines: Vec<String>,
    pub includes: Vec<String>,
}

impl SourceFile {
    pub fn new(path: String, engine: Engine, blueprint: bool, text: &str) -> Self {
        if blueprint {
            return Self {
                path,
                engine,
                blueprint,
                tokens: Vec::new(),
                lines: text.lines().map(str::to_string).collect(),
                includes: Vec::new(),
            };
        }
        let lexed = lex(text);
        let includes = lexed
            .directives
            .iter()
            .filter_map(|(_, d)| {
                d.strip_prefix("#include")
                    .map(|s| s.trim().trim_matches(['"', '<', '>']).to_string())
            })
            .collect();
        Self {
            path,
            engine,
            blueprint,
            tokens: lexed.tokens,
            lines: lexed.lines,
            includes,
        }
    }
}

/// Literal default values for fields (numbers, booleans, strings, vectors, colors).
pub fn literal(tokens: &[Token]) -> Option<serde_json::Value> {
    use serde_json::{Value, json};
    let num = |t: &[Token]| -> Option<f64> {
        let (neg, t) = match t {
            [m, rest @ ..] if m.is("-") => (true, rest),
            _ => (false, t),
        };
        let [n] = t else { return None };
        if n.kind != Kind::Number {
            return None;
        }
        let s = n
            .text
            .trim_end_matches(['f', 'F', 'd', 'D', 'm', 'M', 'u', 'U', 'l', 'L']);
        let v: f64 = s.trim_end_matches('.').parse().ok()?;
        Some(if neg { -v } else { v })
    };
    let to_json = |v: f64| -> Value {
        if v.fract() == 0.0 && v.abs() < 1e15 {
            json!(v as i64)
        } else {
            json!(v)
        }
    };
    if let Some(v) = num(tokens) {
        return Some(to_json(v));
    }
    match tokens {
        [t] if t.is("true") => return Some(json!(true)),
        [t] if t.is("false") => return Some(json!(false)),
        [t] if t.kind == Kind::Str => return Some(json!(unquote(&t.text))),
        [t, o, s, c] if t.is("TEXT") && o.is("(") && s.kind == Kind::Str && c.is(")") => {
            return Some(json!(unquote(&s.text)));
        }
        _ => {}
    }
    // Named constants.
    let text = join(tokens);
    let named: BTreeMap<&str, Value> = [
        ("Vector3.zero", json!({"x": 0, "y": 0, "z": 0})),
        ("Vector3.one", json!({"x": 1, "y": 1, "z": 1})),
        ("Vector3.up", json!({"x": 0, "y": 1, "z": 0})),
        ("Vector2.zero", json!({"x": 0, "y": 0})),
        ("Vector2.one", json!({"x": 1, "y": 1})),
        ("Quaternion.identity", json!({"x": 0, "y": 0, "z": 0, "w": 1})),
        ("Color.white", json!({"r": 1, "g": 1, "b": 1, "a": 1})),
        ("Color.black", json!({"r": 0, "g": 0, "b": 0, "a": 1})),
        ("Color.red", json!({"r": 1, "g": 0, "b": 0, "a": 1})),
        ("FVector::ZeroVector", json!({"x": 0, "y": 0, "z": 0})),
        ("FVector::OneVector", json!({"x": 1, "y": 1, "z": 1})),
        ("FRotator::ZeroRotator", json!({"x": 0, "y": 0, "z": 0, "w": 1})),
        ("FLinearColor::White", json!({"r": 1, "g": 1, "b": 1, "a": 1})),
        ("FLinearColor::Black", json!({"r": 0, "g": 0, "b": 0, "a": 1})),
    ]
    .into_iter()
    .collect();
    if let Some(v) = named.get(text.as_str()) {
        return Some(v.clone());
    }
    // Constructors: new Vector3(1, 2, 3), FVector(1.f, 2.f, 3.f), new Color(1, 0, 0, 1).
    let start = tokens
        .iter()
        .position(|t| t.kind == Kind::Ident && !t.is("new"))?;
    let ctor = tokens[start].text.as_str();
    let keys: &[&str] = match ctor {
        "Vector3" | "FVector" | "Vector3Int" => &["x", "y", "z"],
        "Vector2" | "FVector2D" | "Vector2Int" => &["x", "y"],
        "Color" | "FLinearColor" => &["r", "g", "b", "a"],
        _ => return None,
    };
    let open = start + 1;
    if !tokens.get(open)?.is("(") || matching(tokens, open)? != tokens.len() - 1 {
        return None;
    }
    let inner = &tokens[open + 1..tokens.len() - 1];
    let parts: Vec<&[Token]> = inner.split(|t| t.is(",")).collect();
    if parts.len() > keys.len() || parts.len() < keys.len().min(3) {
        return None;
    }
    let mut obj = serde_json::Map::new();
    for (k, p) in keys.iter().zip(parts.iter()) {
        obj.insert((*k).to_string(), to_json(num(p)?));
    }
    if keys.len() == 4 && parts.len() == 3 {
        obj.insert("a".into(), json!(1));
    }
    Some(Value::Object(obj))
}
