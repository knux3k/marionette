//! Rust serde types from C# `[Serializable]` classes and structs (decision 0240).
//!
//! Unity's `JsonUtility` saves a game's data as JSON with its own rules. This module reads the
//! declarations and writes Rust types that load and write the same JSON, so a port can read
//! the old files and write files the old game still reads. The rules it follows:
//!
//! - A field is saved when it is public (or `[SerializeField]`) and not static, const,
//!   readonly or `[NonSerialized]`. Properties are never saved. The JSON key is the field name.
//! - Every field is written, in declaration order, base class first. A missing key keeps the
//!   value the constructor gave it (the field initializer, else zero, empty, false), so every
//!   type is `#[serde(default)]` with a `Default` that carries the initializers.
//! - Nested objects are never null; arrays and lists are `Vec`; `Dictionary` is not saved.
//! - Enums are saved as their integer. They become a newtype over the integer so an unknown
//!   value survives a round trip.
//! - `UnityEngine` value types (vectors, `Color`, ...) are written out as plain structs.
//!
//! Anything that cannot be mapped is reported in `notes` and, if it is a field of an unknown
//! type, kept as `serde_json::Value` so no data is lost.

use crate::source::{RawClass, RawEnum, RawField, enum_decls};
use crate::{Analysis, lex::Kind};
use schemars::JsonSchema;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

/// What to generate.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Only these types and what they reference. Empty: every `[Serializable]` type.
    pub roots: Vec<String>,
    /// Extra derives on every generated type (for example `Reflect`).
    pub derives: Vec<String>,
    /// C# type name -> Rust path of a type the project already has (a hand-written type, or its
    /// own vector type). Such a type is used as is and never generated.
    pub external: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct TypeInfo {
    pub name: String,
    /// `struct`, `enum` or `unity` (a `UnityEngine` value type written out).
    pub kind: String,
    pub file: String,
    pub line: u32,
    pub fields: usize,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Note {
    /// `skipped` (field not saved by JsonUtility), `unresolved` (kept as raw JSON),
    /// `initializer` (default value not translated, zero used), `duplicate`, `enum`.
    pub kind: String,
    pub file: String,
    pub line: u32,
    /// `Type.field` or the type name.
    pub at: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Generated {
    /// The Rust source of one module.
    pub rust: String,
    pub types: Vec<TypeInfo>,
    pub notes: Vec<Note>,
}

#[derive(Debug, Clone, PartialEq)]
enum Ty {
    Prim(&'static str),
    Str,
    Vec(Box<Ty>),
    Class(String),
    Enum(String),
    Unity(&'static str),
    /// A type the project provides (`Options::external`): its Rust path.
    External(String, Option<&'static str>),
    /// A reference to a `UnityEngine.Object`: saved as `{"instanceID": n}`.
    ObjectRef,
    Raw,
}

enum Resolved {
    Ty(Ty),
    /// A usable type plus a note (a list whose elements are unknown: `Vec<Value>`).
    Partial(Ty, String),
    Skip(String),
    Unresolved(String),
}

/// Unity value types: name, fields (name, Rust type).
const UNITY_TYPES: [(&str, &[(&str, &str)]); 9] = [
    ("Vector2", &[("x", "f32"), ("y", "f32")]),
    ("Vector3", &[("x", "f32"), ("y", "f32"), ("z", "f32")]),
    (
        "Vector4",
        &[("x", "f32"), ("y", "f32"), ("z", "f32"), ("w", "f32")],
    ),
    (
        "Quaternion",
        &[("x", "f32"), ("y", "f32"), ("z", "f32"), ("w", "f32")],
    ),
    ("Color", &[("r", "f32"), ("g", "f32"), ("b", "f32"), ("a", "f32")]),
    ("Color32", &[("r", "u8"), ("g", "u8"), ("b", "u8"), ("a", "u8")]),
    ("Vector2Int", &[("x", "i32"), ("y", "i32")]),
    ("Vector3Int", &[("x", "i32"), ("y", "i32"), ("z", "i32")]),
    (
        "Rect",
        &[("x", "f32"), ("y", "f32"), ("width", "f32"), ("height", "f32")],
    ),
];

/// Named constants of the value types, in field order.
const UNITY_CONSTS: [(&str, &[f64]); 26] = [
    ("Vector2.zero", &[0.0, 0.0]),
    ("Vector2.one", &[1.0, 1.0]),
    ("Vector2.up", &[0.0, 1.0]),
    ("Vector2.down", &[0.0, -1.0]),
    ("Vector2.left", &[-1.0, 0.0]),
    ("Vector2.right", &[1.0, 0.0]),
    ("Vector3.zero", &[0.0, 0.0, 0.0]),
    ("Vector3.one", &[1.0, 1.0, 1.0]),
    ("Vector3.up", &[0.0, 1.0, 0.0]),
    ("Vector3.down", &[0.0, -1.0, 0.0]),
    ("Vector3.left", &[-1.0, 0.0, 0.0]),
    ("Vector3.right", &[1.0, 0.0, 0.0]),
    ("Vector3.forward", &[0.0, 0.0, 1.0]),
    ("Vector3.back", &[0.0, 0.0, -1.0]),
    ("Quaternion.identity", &[0.0, 0.0, 0.0, 1.0]),
    ("Color.white", &[1.0, 1.0, 1.0, 1.0]),
    ("Color.black", &[0.0, 0.0, 0.0, 1.0]),
    ("Color.red", &[1.0, 0.0, 0.0, 1.0]),
    ("Color.green", &[0.0, 1.0, 0.0, 1.0]),
    ("Color.blue", &[0.0, 0.0, 1.0, 1.0]),
    ("Color.yellow", &[1.0, 0.92, 0.016, 1.0]),
    ("Color.clear", &[0.0, 0.0, 0.0, 0.0]),
    ("Color.gray", &[0.5, 0.5, 0.5, 1.0]),
    ("Color.cyan", &[0.0, 1.0, 1.0, 1.0]),
    ("Color.grey", &[0.5, 0.5, 0.5, 1.0]),
    ("Color.magenta", &[1.0, 0.0, 1.0, 1.0]),
];

/// `UnityEngine.Object` types a field can point at.
const OBJECT_TYPES: [&str; 14] = [
    "GameObject",
    "Transform",
    "Component",
    "MonoBehaviour",
    "ScriptableObject",
    "Material",
    "Mesh",
    "Sprite",
    "Texture",
    "Texture2D",
    "AudioClip",
    "Shader",
    "RectTransform",
    "Object",
];

const KEYWORDS: [&str; 38] = [
    "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn", "for", "if",
    "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self",
    "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where", "while", "async",
    "await", "dyn",
];

fn prim(name: &str) -> Option<&'static str> {
    Some(match name {
        "bool" | "Boolean" => "bool",
        "int" | "Int32" => "i32",
        "uint" | "UInt32" => "u32",
        "long" | "Int64" => "i64",
        "ulong" | "UInt64" => "u64",
        "short" | "Int16" => "i16",
        "ushort" | "UInt16" => "u16",
        "byte" | "Byte" => "u8",
        "sbyte" | "SByte" => "i8",
        "float" | "Single" => "f32",
        "double" | "Double" => "f64",
        _ => return None,
    })
}

fn last_segment(s: &str) -> &str {
    s.rsplit('.').next().unwrap_or(s)
}

/// Splits `Outer<A,B>` into `Outer` and the top-level arguments.
fn generic(s: &str) -> Option<(&str, Vec<&str>)> {
    let open = s.find('<')?;
    if !s.ends_with('>') {
        return None;
    }
    let inner = &s[open + 1..s.len() - 1];
    let mut args = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    for (i, c) in inner.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => depth -= 1,
            ',' if depth == 0 => {
                args.push(&inner[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    args.push(&inner[start..]);
    Some((&s[..open], args))
}

fn snake(name: &str) -> String {
    let chars: Vec<char> = name.trim_start_matches('_').chars().collect();
    let mut out = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if c.is_uppercase() {
            let prev = i.checked_sub(1).map(|p| chars[p]);
            let next = chars.get(i + 1);
            let boundary = prev.is_some_and(|p| p.is_lowercase() || p.is_ascii_digit())
                || (prev.is_some_and(char::is_uppercase) && next.is_some_and(|n| n.is_lowercase()));
            if boundary && !out.ends_with('_') {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    if out.is_empty() {
        out.push_str("field");
    }
    if out.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        out.insert(0, 'f');
    }
    if KEYWORDS.contains(&out.as_str()) {
        out.push('_');
    }
    out
}

struct Gen<'a> {
    a: &'a Analysis,
    classes: BTreeMap<String, RawClass>,
    /// Every class by name (for base-class fields), serializable or not.
    all: BTreeMap<String, &'a RawClass>,
    enums: BTreeMap<String, (RawEnum, usize)>,
    external: BTreeMap<String, String>,
    /// Static and const initializers of the class being generated.
    consts: BTreeMap<String, Vec<crate::lex::Token>>,
    notes: Vec<Note>,
    noted: BTreeSet<(String, String)>,
}

impl<'a> Gen<'a> {
    fn file_of(&self, i: usize) -> String {
        self.a.files.get(i).map(|f| f.path.clone()).unwrap_or_default()
    }

    fn note(&mut self, kind: &str, file: usize, line: u32, at: &str, message: String) {
        if !self.noted.insert((at.to_string(), message.clone())) {
            return;
        }
        let file = self.file_of(file);
        self.notes.push(Note {
            kind: kind.into(),
            file,
            line,
            at: at.into(),
            message,
        });
    }

    fn resolve(&self, ty: &str) -> Resolved {
        let ty: String = ty.chars().filter(|c| !c.is_whitespace()).collect();
        if let Some(inner) = ty.strip_suffix("[]") {
            return match self.resolve(inner) {
                Resolved::Ty(Ty::Vec(_)) => {
                    Resolved::Skip("nested arrays are not saved by JsonUtility".into())
                }
                Resolved::Ty(t) => Resolved::Ty(Ty::Vec(Box::new(t))),
                Resolved::Unresolved(why) => Resolved::Partial(Ty::Vec(Box::new(Ty::Raw)), why),
                other => other,
            };
        }
        if ty.contains("[,") || ty.ends_with("?") {
            return Resolved::Skip(
                "multi-dimensional arrays and nullable types are not saved by JsonUtility".into(),
            );
        }
        if let Some((outer, args)) = generic(&ty) {
            let outer = last_segment(outer);
            return match (outer, args.as_slice()) {
                ("List", [t]) => match self.resolve(t) {
                    Resolved::Ty(Ty::Vec(_)) => {
                        Resolved::Skip("nested lists are not saved by JsonUtility".into())
                    }
                    Resolved::Ty(t) => Resolved::Ty(Ty::Vec(Box::new(t))),
                    Resolved::Unresolved(why) => Resolved::Partial(Ty::Vec(Box::new(Ty::Raw)), why),
                    other => other,
                },
                ("Dictionary" | "HashSet" | "Queue" | "Stack" | "SortedList", _) => {
                    Resolved::Skip(format!("{outer} is not saved by JsonUtility"))
                }
                _ => Resolved::Unresolved(format!("generic type {outer}<..> is not mapped")),
            };
        }
        let name = last_segment(&ty);
        if let Some(path) = self.external.get(name) {
            let unity = UNITY_TYPES.iter().find(|(n, _)| *n == name).map(|(n, _)| *n);
            return Resolved::Ty(Ty::External(path.clone(), unity));
        }
        if let Some(p) = prim(name) {
            return Resolved::Ty(Ty::Prim(p));
        }
        if name == "string" || name == "String" {
            return Resolved::Ty(Ty::Str);
        }
        if self.classes.contains_key(name) {
            return Resolved::Ty(Ty::Class(name.into()));
        }
        if self.enums.contains_key(name) {
            return Resolved::Ty(Ty::Enum(name.into()));
        }
        if self.all.contains_key(name) {
            return Resolved::Skip(format!("class `{name}` is not [Serializable]"));
        }
        if let Some((u, _)) = UNITY_TYPES.iter().find(|(n, _)| *n == name) {
            return Resolved::Ty(Ty::Unity(u));
        }
        if OBJECT_TYPES.contains(&name) {
            return Resolved::Ty(Ty::ObjectRef);
        }
        Resolved::Unresolved(format!("type `{name}` is not in the scanned source"))
    }

    fn rust_ty(&self, t: &Ty) -> String {
        match t {
            Ty::Prim(p) => (*p).into(),
            Ty::Str => "String".into(),
            Ty::Vec(i) => format!("Vec<{}>", self.rust_ty(i)),
            Ty::Class(n) | Ty::Enum(n) => n.clone(),
            Ty::Unity(n) => (*n).into(),
            Ty::External(p, _) => p.clone(),
            Ty::ObjectRef => "ObjectRef".into(),
            Ty::Raw => "serde_json::Value".into(),
        }
    }

    /// A Unity value type's constant or constructor as a struct literal of `path`.
    fn unity_init(&self, n: &str, path: &str, text: &str) -> Option<String> {
        let fields = UNITY_TYPES.iter().find(|(u, _)| *u == n)?.1;
        let values: Vec<f64> = if let Some((_, v)) = UNITY_CONSTS.iter().find(|(c, _)| *c == text) {
            v.to_vec()
        } else {
            let args = text
                .strip_prefix(&format!("new{n}("))
                .or_else(|| text.strip_prefix(&format!("{n}(")))?
                .strip_suffix(')')?;
            let mut v = Vec::new();
            for a in args.split(',') {
                v.push(a.trim_end_matches(['f', 'F']).parse::<f64>().ok()?);
            }
            v
        };
        if values.len() > fields.len() || values.len() < fields.len().min(2) {
            return None;
        }
        let mut parts = Vec::new();
        for (i, (f, ft)) in fields.iter().enumerate() {
            let v = match values.get(i) {
                Some(v) => *v,
                None if *f == "a" => 1.0,
                None => 0.0,
            };
            parts.push(if ft.starts_with('f') {
                format!("{f}: {v:?}")
            } else {
                format!("{f}: {}", v as i64)
            });
        }
        Some(format!("{path} {{ {} }}", parts.join(", ")))
    }

    /// The Rust expression for a field initializer, or None when it is not understood.
    fn init(&self, tokens: &[crate::lex::Token], t: &Ty) -> Option<String> {
        self.init_at(tokens, t, 0)
    }

    fn init_at(&self, tokens: &[crate::lex::Token], t: &Ty, depth: u8) -> Option<String> {
        use crate::source::join;
        let text = join(tokens);
        let text: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        // A name of a const or static field of the same class (also `Class.Name`).
        if depth < 4 {
            let name = text.rsplit('.').next().unwrap_or(&text);
            let is_name = !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_');
            if is_name
                && !matches!(text.as_str(), "true" | "false" | "null")
                && !text.starts_with(|c: char| c.is_ascii_digit())
                && let Some(init) = self.consts.get(name)
            {
                return self.init_at(init, t, depth + 1);
            }
        }
        // `int.MaxValue`, `float.MinValue`, `Mathf.Infinity`.
        if let Ty::Prim(p) = t {
            let (head, member) = text.split_once('.').unwrap_or(("", ""));
            if prim(head).is_some() || head == "Mathf" {
                let float = p.starts_with('f');
                let rust = match member {
                    "MaxValue" => Some("MAX"),
                    "MinValue" => Some("MIN"),
                    "Epsilon" if float => Some("EPSILON"),
                    "PositiveInfinity" | "Infinity" if float => Some("INFINITY"),
                    "NegativeInfinity" if float => Some("NEG_INFINITY"),
                    _ => None,
                };
                if let Some(r) = rust {
                    return Some(format!("{p}::{r}"));
                }
            }
        }
        match t {
            Ty::Prim(p) => {
                if *p == "bool" {
                    return matches!(text.as_str(), "true" | "false").then_some(text);
                }
                let (neg, rest) = match tokens {
                    [m, rest @ ..] if m.is("-") => (true, rest),
                    _ => (false, tokens),
                };
                let [n] = rest else { return None };
                if n.kind != Kind::Number {
                    return None;
                }
                let s = n.text.replace('_', "");
                let sign = if neg { "-" } else { "" };
                if p.starts_with('f') {
                    let s = s.trim_end_matches(['f', 'F', 'd', 'D']);
                    let v: f64 = s.parse().ok()?;
                    Some(format!("{sign}{v:?}"))
                } else {
                    let s = s.trim_end_matches(['u', 'U', 'l', 'L']);
                    let v: i128 = match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
                        Some(h) => i128::from_str_radix(h, 16).ok()?,
                        None => s.parse().ok()?,
                    };
                    Some(format!("{sign}{v}"))
                }
            }
            Ty::Str => {
                if text == "string.Empty" || text == "String.Empty" || text == "null" {
                    return Some("String::new()".into());
                }
                let [s] = tokens else { return None };
                if s.kind != Kind::Str || s.text.starts_with(['@', '$']) {
                    return None;
                }
                let inner = s.text.strip_prefix('"')?.strip_suffix('"')?;
                let mut out = String::new();
                let mut it = inner.chars();
                while let Some(c) = it.next() {
                    if c == '\\' {
                        out.push(match it.next()? {
                            'n' => '\n',
                            't' => '\t',
                            'r' => '\r',
                            '0' => '\0',
                            c @ ('"' | '\\' | '\'') => c,
                            _ => return None,
                        });
                    } else {
                        out.push(c);
                    }
                }
                Some(format!("{out:?}.to_string()"))
            }
            Ty::Vec(elem) => {
                let after_new = tokens.first().filter(|t| t.is("new"))?;
                let _ = after_new;
                let open = tokens.iter().position(|t| t.is("{"));
                if let Some(o) = open {
                    let close = tokens.iter().rposition(|t| t.is("}"))?;
                    let body = &tokens[o + 1..close];
                    let mut items = Vec::new();
                    for part in body.split(|t| t.is(",")) {
                        if part.is_empty() {
                            continue;
                        }
                        items.push(self.init(part, elem)?);
                    }
                    return Some(format!("vec![{}]", items.join(", ")));
                }
                // `new List<T>()`, `new T[0]`, `new T[n]`.
                if text.starts_with("newList<") && text.ends_with("()") {
                    return Some("Vec::new()".into());
                }
                let size = text.rsplit_once('[')?.1.strip_suffix(']')?;
                match size.parse::<usize>().ok()? {
                    0 => Some("Vec::new()".into()),
                    n => Some(format!("vec![Default::default(); {n}]")),
                }
            }
            Ty::Class(n) => {
                let path = text.strip_prefix("new")?.strip_suffix("()")?;
                (path.rsplit('.').next() == Some(n.as_str())).then(|| format!("{n}::default()"))
            }
            Ty::Enum(n) => {
                let (e, _) = self.enums.get(n)?;
                let member = text.strip_prefix(&format!("{n}."))?;
                e.members
                    .iter()
                    .any(|(m, _)| m == member)
                    .then(|| format!("{n}::{member}"))
            }
            Ty::Unity(n) => self.unity_init(n, n, &text),
            Ty::External(path, Some(n)) => self.unity_init(n, path, &text),
            Ty::External(path, None) => {
                let made = text.strip_prefix("new")?.strip_suffix("()")?;
                let _ = made;
                Some(format!("{path}::default()"))
            }
            Ty::ObjectRef | Ty::Raw => None,
        }
    }
}

/// A saved field of a class after the rules above.
struct Field {
    json: String,
    rust: String,
    ty: Ty,
    default: String,
    comment: Option<String>,
}

fn is_saved(f: &RawField) -> Result<(), String> {
    let has = |m: &str| f.modifiers.iter().any(|x| x == m);
    if has("static") || has("const") || has("readonly") {
        return Err("static, const and readonly fields are not saved".into());
    }
    if f.attributes.iter().any(|a| a == "NonSerialized") {
        return Err("[NonSerialized]".into());
    }
    if !(has("public")
        || f.attributes
            .iter()
            .any(|a| a == "SerializeField" || a == "SerializeReference"))
    {
        return Err("not public and not [SerializeField]".into());
    }
    Ok(())
}

/// Generates the Rust module for the `[Serializable]` types of a scan.
pub fn generate(a: &Analysis, opts: &Options) -> Generated {
    let mut g = Gen {
        a,
        classes: BTreeMap::new(),
        all: BTreeMap::new(),
        enums: BTreeMap::new(),
        external: opts.external.clone(),
        consts: BTreeMap::new(),
        notes: Vec::new(),
        noted: BTreeSet::new(),
    };
    for c in &a.classes {
        if c.raw.blueprint || c.raw.engine != crate::map::Engine::Unity {
            continue;
        }
        g.all.entry(c.raw.name.clone()).or_insert(&c.raw);
    }
    // Serializable classes; partial parts merge, other duplicates are reported.
    for c in &a.classes {
        let r = &c.raw;
        if r.blueprint
            || r.engine != crate::map::Engine::Unity
            || !r.attributes.iter().any(|x| x == "Serializable")
        {
            continue;
        }
        match g.classes.get_mut(&r.name) {
            None => {
                g.classes.insert(r.name.clone(), r.clone());
            }
            Some(first) if first.partial && r.partial => first.fields.extend(r.fields.iter().cloned()),
            Some(_) => g.note(
                "duplicate",
                r.file,
                r.line,
                &r.name,
                "a second class with this name was ignored (nested types share one namespace here)".into(),
            ),
        }
    }
    for (fi, f) in a.files.iter().enumerate() {
        for (e, complete) in enum_decls(&f.tokens) {
            if !complete {
                g.note(
                    "enum",
                    fi,
                    e.line,
                    &e.name,
                    "some member values could not be evaluated; those members were left out".into(),
                );
            }
            g.enums.entry(e.name.clone()).or_insert((e, fi));
        }
    }

    // Which types to emit: the roots and everything their saved fields reach.
    let roots: Vec<String> = if opts.roots.is_empty() {
        g.classes
            .keys()
            .filter(|k| !g.external.contains_key(*k))
            .cloned()
            .collect()
    } else {
        opts.roots.clone()
    };
    let mut order: Vec<String> = Vec::new();
    let mut enum_order: Vec<String> = Vec::new();
    let mut unity: BTreeSet<&'static str> = BTreeSet::new();
    let mut object_ref = false;
    let mut field_map: BTreeMap<String, Vec<Field>> = BTreeMap::new();
    let mut queue: Vec<String> = roots;
    let mut seen: BTreeSet<String> = BTreeSet::new();
    while let Some(name) = queue.pop() {
        if !seen.insert(name.clone()) || g.external.contains_key(&name) {
            continue;
        }
        let Some(class) = g.classes.get(&name).cloned() else {
            g.note(
                "unresolved",
                0,
                0,
                &name,
                "no [Serializable] type with this name".into(),
            );
            continue;
        };
        // Base chain, base class first.
        let mut chain: Vec<RawClass> = vec![class.clone()];
        let mut cur = class.base.clone();
        let mut guard = 0;
        while let Some(b) = cur {
            guard += 1;
            let Some(base) = g.all.get(&b).filter(|_| guard < 16) else {
                break;
            };
            chain.push((*base).clone());
            cur = base.base.clone();
        }
        chain.reverse();
        g.consts.clear();
        for c in &chain {
            for f in &c.fields {
                let has = |m: &str| f.modifiers.iter().any(|x| x == m);
                if (has("static") || has("const") || has("readonly")) && !f.init_tokens.is_empty() {
                    g.consts.insert(f.name.clone(), f.init_tokens.clone());
                }
            }
        }
        let mut fields: Vec<Field> = Vec::new();
        let mut used_rust: BTreeSet<String> = BTreeSet::new();
        for c in &chain {
            for f in &c.fields {
                let at = format!("{}.{}", c.name, f.name);
                if let Err(why) = is_saved(f) {
                    // Only worth a note when it looks like it was meant to be saved.
                    if f.modifiers.iter().any(|m| m == "public")
                        || f.attributes.iter().any(|a| a == "NonSerialized")
                    {
                        g.note("skipped", c.file, f.line, &at, why);
                    }
                    continue;
                }
                let (ty, comment) = if f.attributes.iter().any(|a| a == "SerializeReference") {
                    g.note(
                        "unresolved",
                        c.file,
                        f.line,
                        &at,
                        "[SerializeReference] saves a typed object with a class name; kept as raw JSON"
                            .into(),
                    );
                    (Ty::Raw, Some("SerializeReference".to_string()))
                } else {
                    match g.resolve(&f.ty) {
                        Resolved::Ty(t) => (t, None),
                        Resolved::Partial(t, why) => {
                            g.note(
                                "unresolved",
                                c.file,
                                f.line,
                                &at,
                                format!("{why}; elements kept as raw JSON"),
                            );
                            (t, None)
                        }
                        Resolved::Skip(why) => {
                            g.note("skipped", c.file, f.line, &at, why);
                            continue;
                        }
                        Resolved::Unresolved(why) => {
                            g.note(
                                "unresolved",
                                c.file,
                                f.line,
                                &at,
                                format!("{why}; kept as raw JSON"),
                            );
                            (Ty::Raw, Some(f.ty.clone()))
                        }
                    }
                };
                // Walk into what the field needs.
                let mut inner = &ty;
                while let Ty::Vec(i) = inner {
                    inner = i;
                }
                match inner {
                    Ty::Class(n) => queue.push(n.clone()),
                    Ty::Enum(n) => {
                        if !enum_order.contains(n) {
                            enum_order.push(n.clone());
                        }
                    }
                    Ty::Unity(n) => {
                        unity.insert(n);
                    }
                    Ty::ObjectRef => object_ref = true,
                    _ => {}
                }
                let default = if f.init_tokens.is_empty() {
                    match &ty {
                        Ty::Class(n) => format!("{n}::default()"),
                        _ => "Default::default()".into(),
                    }
                } else if let Some(e) = g.init(&f.init_tokens, &ty) {
                    e
                } else {
                    g.note(
                        "initializer",
                        c.file,
                        f.line,
                        &at,
                        format!(
                            "initializer `{}` not translated; the zero value is used",
                            f.init.clone().unwrap_or_default()
                        ),
                    );
                    "Default::default()".into()
                };
                let mut rust = snake(&f.name);
                let mut n = 2;
                while !used_rust.insert(rust.clone()) {
                    rust = format!("{}_{n}", snake(&f.name));
                    n += 1;
                }
                // A later field with the same JSON key (a hidden base field) replaces the base's.
                fields.retain(|x: &Field| x.json != f.name);
                fields.push(Field {
                    json: f.name.clone(),
                    rust,
                    ty,
                    default,
                    comment,
                });
            }
        }
        order.push(name.clone());
        field_map.insert(name, fields);
    }
    order.sort();
    enum_order.sort();

    // Emit.
    let mut derives = vec!["Debug", "Clone", "PartialEq", "Serialize", "Deserialize"];
    let extra: Vec<&str> = opts.derives.iter().map(String::as_str).collect();
    derives.extend(extra);
    let derive = derives.join(", ");
    let mut out = String::new();
    out.push_str(
        "//! Generated by `port_serde` from C# [Serializable] types (decision 0240). Do not edit:\n\
         //! change the C# source or the generator and run it again.\n\
         //!\n\
         //! Reads and writes the JSON Unity's JsonUtility makes. Every field is optional on read\n\
         //! and keeps its C# initializer; unknown keys are ignored, as JsonUtility ignores them.\n\
         #![allow(clippy::all, non_upper_case_globals, dead_code)]\n\n\
         use serde::{Deserialize, Serialize};\n\n\
         /// Reads JsonUtility text (a leading byte order mark is ignored).\n\
         pub fn from_json<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, serde_json::Error> {\n    \
         serde_json::from_str(text.trim_start_matches('\\u{feff}'))\n}\n\n\
         /// Writes compact JSON with the fields in declaration order, like `JsonUtility.ToJson`.\n\
         pub fn to_json<T: Serialize>(value: &T) -> Result<String, serde_json::Error> {\n    \
         serde_json::to_string(value)\n}\n\n",
    );
    let mut types = Vec::new();
    if object_ref {
        let _ = writeln!(
            out,
            "/// A reference to a Unity object. JsonUtility saves only its instance id.\n\
             #[derive({derive}, Default)]\n#[serde(default)]\npub struct ObjectRef {{\n    \
             #[serde(rename = \"instanceID\")]\n    pub instance_id: i32,\n}}\n"
        );
        types.push(TypeInfo {
            name: "ObjectRef".into(),
            kind: "unity".into(),
            file: String::new(),
            line: 0,
            fields: 1,
        });
    }
    let user_names: BTreeSet<&String> = order.iter().collect();
    for (u, fields) in UNITY_TYPES {
        if !unity.contains(u) || user_names.contains(&u.to_string()) {
            continue;
        }
        let _ = writeln!(
            out,
            "#[derive({derive}, Default)]\n#[serde(default)]\npub struct {u} {{"
        );
        for (f, t) in fields {
            let _ = writeln!(out, "    pub {f}: {t},");
        }
        let _ = writeln!(out, "}}\n");
        types.push(TypeInfo {
            name: u.into(),
            kind: "unity".into(),
            file: String::new(),
            line: 0,
            fields: fields.len(),
        });
    }
    for name in &enum_order {
        let (e, fi) = &g.enums[name];
        let ty = prim(&e.underlying).unwrap_or("i32");
        let _ = writeln!(
            out,
            "#[derive({derive}, Copy, Eq, PartialOrd, Ord, Hash, Default)]\n#[serde(transparent)]\npub struct {name}(pub {ty});\n\nimpl {name} {{"
        );
        for (m, v) in &e.members {
            let _ = writeln!(out, "    pub const {m}: {name} = {name}({v});");
        }
        let _ = writeln!(out, "}}\n");
        types.push(TypeInfo {
            name: name.clone(),
            kind: "enum".into(),
            file: g.file_of(*fi),
            line: e.line,
            fields: e.members.len(),
        });
    }
    for name in &order {
        let fields = &field_map[name];
        let class = &g.classes[name];
        let _ = writeln!(
            out,
            "#[derive({derive})]\n#[serde(default)]\npub struct {name} {{"
        );
        for f in fields {
            if let Some(c) = &f.comment {
                let _ = writeln!(out, "    /// C# type `{c}`.");
            }
            if f.rust != f.json {
                let _ = writeln!(out, "    #[serde(rename = \"{}\")]", f.json);
            }
            let _ = writeln!(out, "    pub {}: {},", f.rust, g.rust_ty(&f.ty));
        }
        let _ = writeln!(out, "}}\n");
        let _ = writeln!(
            out,
            "impl Default for {name} {{\n    fn default() -> Self {{\n        Self {{"
        );
        for f in fields {
            let _ = writeln!(out, "            {}: {},", f.rust, f.default);
        }
        let _ = writeln!(out, "        }}\n    }}\n}}\n");
        types.push(TypeInfo {
            name: name.clone(),
            kind: "struct".into(),
            file: g.file_of(class.file),
            line: class.line,
            fields: fields.len(),
        });
    }
    let rust = out.trim_end().to_string() + "\n";
    Generated {
        rust,
        types,
        notes: g.notes,
    }
}
