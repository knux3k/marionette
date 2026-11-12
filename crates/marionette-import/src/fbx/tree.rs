//! The FBX node tree, read from binary or ASCII FBX (version 7000 and newer, FBX 2011+).
//!
//! Both encodings produce the same [`Node`] tree, so the scene reader does not know which one it
//! got. Binary object names (`Name\x00\x01Class`) are rewritten to the ASCII form
//! (`Class::Name`).

use std::io::Read;

/// Limits that keep a hostile or broken file from exhausting memory.
#[derive(Debug, Clone, Copy)]
pub struct TreeLimits {
    /// Largest decoded array, in bytes.
    pub max_array_bytes: usize,
    /// Most nodes in the whole tree.
    pub max_nodes: usize,
    /// Deepest nesting.
    pub max_depth: usize,
}

impl Default for TreeLimits {
    fn default() -> Self {
        Self {
            max_array_bytes: 512 << 20,
            max_nodes: 4_000_000,
            max_depth: 64,
        }
    }
}

/// One property value of a node.
#[derive(Debug, Clone, PartialEq)]
pub enum Prop {
    Bool(bool),
    I16(i16),
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
    Str(String),
    Raw(Vec<u8>),
    BoolArr(Vec<bool>),
    I32Arr(Vec<i32>),
    I64Arr(Vec<i64>),
    F32Arr(Vec<f32>),
    F64Arr(Vec<f64>),
}

impl Prop {
    pub fn as_i64(&self) -> Option<i64> {
        Some(match self {
            Prop::Bool(b) => *b as i64,
            Prop::I16(v) => *v as i64,
            Prop::I32(v) => *v as i64,
            Prop::I64(v) => *v,
            Prop::F32(v) => *v as i64,
            Prop::F64(v) => *v as i64,
            _ => return None,
        })
    }

    pub fn as_f64(&self) -> Option<f64> {
        Some(match self {
            Prop::F32(v) => *v as f64,
            Prop::F64(v) => *v,
            p => p.as_i64()? as f64,
        })
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Prop::Str(s) => Some(s),
            _ => None,
        }
    }

    /// Any numeric array (or a single number) as `f64` values.
    pub fn f64s(&self) -> Option<Vec<f64>> {
        Some(match self {
            Prop::F64Arr(v) => v.clone(),
            Prop::F32Arr(v) => v.iter().map(|&x| x as f64).collect(),
            Prop::I32Arr(v) => v.iter().map(|&x| x as f64).collect(),
            Prop::I64Arr(v) => v.iter().map(|&x| x as f64).collect(),
            Prop::BoolArr(v) => v.iter().map(|&x| x as u8 as f64).collect(),
            p => vec![p.as_f64()?],
        })
    }

    /// Any integer array (or a single integer) as `i64` values.
    pub fn i64s(&self) -> Option<Vec<i64>> {
        Some(match self {
            Prop::I64Arr(v) => v.clone(),
            Prop::I32Arr(v) => v.iter().map(|&x| x as i64).collect(),
            Prop::F64Arr(v) => v.iter().map(|&x| x as i64).collect(),
            Prop::F32Arr(v) => v.iter().map(|&x| x as i64).collect(),
            Prop::BoolArr(v) => v.iter().map(|&x| x as i64).collect(),
            p => vec![p.as_i64()?],
        })
    }
}

/// One node: a name, its properties and its children.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Node {
    pub name: String,
    pub props: Vec<Prop>,
    pub children: Vec<Node>,
}

impl Node {
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }

    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }

    pub fn prop(&self, i: usize) -> Option<&Prop> {
        self.props.get(i)
    }

    /// The first property of child `name` (the usual shape of a value node).
    pub fn value(&self, name: &str) -> Option<&Prop> {
        self.child(name)?.props.first()
    }

    /// Array data of child `name`: binary files store it as the first property; ASCII files as
    /// the values of an `a:` child (or, before 7.x layout habits, as plain values).
    pub fn array(&self, name: &str) -> Option<&Prop> {
        let c = self.child(name)?;
        c.props.first()
    }
}

/// The parsed file: FBX version and the top-level nodes.
#[derive(Debug, Clone)]
pub struct Document {
    pub version: u32,
    pub binary: bool,
    pub roots: Vec<Node>,
}

impl Document {
    pub fn root(&self, name: &str) -> Option<&Node> {
        self.roots.iter().find(|n| n.name == name)
    }
}

const BINARY_MAGIC: &[u8] = b"Kaydara FBX Binary  \0";

/// Reads a binary or ASCII FBX file.
pub fn parse(bytes: &[u8], limits: TreeLimits) -> Result<Document, String> {
    if bytes.starts_with(BINARY_MAGIC) {
        parse_binary(bytes, limits)
    } else {
        parse_ascii(bytes, limits)
    }
}

struct Bin<'a> {
    b: &'a [u8],
    pos: usize,
    limits: TreeLimits,
    nodes: usize,
}

impl Bin<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|&e| e <= self.b.len())
            .ok_or_else(|| format!("binary FBX ends early at byte {}", self.pos))?;
        let s = &self.b[self.pos..end];
        self.pos = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn array<T, const N: usize>(&mut self, conv: fn([u8; N]) -> T) -> Result<Vec<T>, String> {
        let len = self.u32()? as usize;
        let encoding = self.u32()?;
        let stored = self.u32()? as usize;
        let want = len
            .checked_mul(N)
            .filter(|&n| n <= self.limits.max_array_bytes)
            .ok_or_else(|| format!("array of {len} elements is larger than the import limit"))?;
        let raw = self.take(stored)?;
        let data: Vec<u8> = match encoding {
            0 => raw.to_vec(),
            1 => {
                let mut out = Vec::with_capacity(want);
                flate2::read::ZlibDecoder::new(raw)
                    .take(want as u64 + 1)
                    .read_to_end(&mut out)
                    .map_err(|e| format!("compressed array cannot be inflated: {e}"))?;
                out
            }
            e => return Err(format!("unknown array encoding {e}")),
        };
        if data.len() != want {
            return Err(format!(
                "array says {len} elements but holds {} bytes",
                data.len()
            ));
        }
        Ok(data.as_chunks::<N>().0.iter().map(|c| conv(*c)).collect())
    }

    fn prop(&mut self) -> Result<Prop, String> {
        let t = self.u8()?;
        Ok(match t {
            b'Y' => Prop::I16(i16::from_le_bytes(self.take(2)?.try_into().unwrap())),
            b'C' => Prop::Bool(self.u8()? != 0),
            b'I' => Prop::I32(i32::from_le_bytes(self.take(4)?.try_into().unwrap())),
            b'F' => Prop::F32(f32::from_le_bytes(self.take(4)?.try_into().unwrap())),
            b'D' => Prop::F64(f64::from_le_bytes(self.take(8)?.try_into().unwrap())),
            b'L' => Prop::I64(i64::from_le_bytes(self.take(8)?.try_into().unwrap())),
            b'f' => Prop::F32Arr(self.array(f32::from_le_bytes)?),
            b'd' => Prop::F64Arr(self.array(f64::from_le_bytes)?),
            b'l' => Prop::I64Arr(self.array(i64::from_le_bytes)?),
            b'i' => Prop::I32Arr(self.array(i32::from_le_bytes)?),
            b'b' => Prop::BoolArr(self.array(|[b]: [u8; 1]| b != 0)?),
            b'S' => {
                let n = self.u32()? as usize;
                Prop::Str(binary_name(self.take(n)?))
            }
            b'R' => {
                let n = self.u32()? as usize;
                Prop::Raw(self.take(n)?.to_vec())
            }
            t => {
                return Err(format!(
                    "unknown property type `{}` at byte {}",
                    t as char,
                    self.pos - 1
                ));
            }
        })
    }

    /// Reads one node record; `None` for the null record that ends a list.
    fn node(&mut self, wide: bool, depth: usize) -> Result<Option<Node>, String> {
        if depth > self.limits.max_depth {
            return Err("FBX nodes are nested too deeply".into());
        }
        let start = self.pos;
        let (end, nprops, _plen) = if wide {
            (self.u64()?, self.u64()?, self.u64()?)
        } else {
            (self.u32()? as u64, self.u32()? as u64, self.u32()? as u64)
        };
        let name_len = self.u8()? as usize;
        if end == 0 {
            return Ok(None);
        }
        let end = usize::try_from(end).map_err(|_| "node end offset too large".to_string())?;
        if end > self.b.len() || end <= start {
            return Err(format!("node at byte {start} has a bad end offset"));
        }
        self.nodes += 1;
        if self.nodes > self.limits.max_nodes {
            return Err("FBX has more nodes than the import limit".into());
        }
        let name = String::from_utf8_lossy(self.take(name_len)?).into_owned();
        let mut props = Vec::new();
        for _ in 0..nprops {
            if self.pos >= end {
                return Err(format!("node `{name}` properties run past its end"));
            }
            props.push(self.prop()?);
        }
        let mut children = Vec::new();
        while self.pos < end {
            match self.node(wide, depth + 1)? {
                Some(c) => children.push(c),
                None => break,
            }
        }
        self.pos = end;
        Ok(Some(Node {
            name,
            props,
            children,
        }))
    }
}

/// `Name\x00\x01Class` (binary) becomes `Class::Name` (ASCII form).
fn binary_name(raw: &[u8]) -> String {
    let s = String::from_utf8_lossy(raw);
    match s.split_once("\u{0}\u{1}") {
        Some((name, class)) => format!("{class}::{name}"),
        None => s.into_owned(),
    }
}

fn parse_binary(bytes: &[u8], limits: TreeLimits) -> Result<Document, String> {
    if bytes.len() < 27 {
        return Err("binary FBX header is cut short".into());
    }
    let version = u32::from_le_bytes(bytes[23..27].try_into().unwrap());
    let wide = version >= 7500;
    let mut r = Bin {
        b: bytes,
        pos: 27,
        limits,
        nodes: 0,
    };
    let mut roots = Vec::new();
    let null_len = if wide { 25 } else { 13 };
    while r.pos + null_len <= bytes.len() {
        match r.node(wide, 0)? {
            Some(n) => roots.push(n),
            None => break,
        }
    }
    Ok(Document {
        version,
        binary: true,
        roots,
    })
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Word(String),
    Str(String),
    Num(String),
    Colon,
    Comma,
    Open,
    Close,
    Star(usize),
}

fn tokenize(text: &str) -> Result<Vec<(Tok, usize)>, String> {
    let b = text.as_bytes();
    let mut i = 0;
    let mut line = 1;
    let mut out = Vec::new();
    while i < b.len() {
        let c = b[i];
        match c {
            b'\n' => {
                line += 1;
                i += 1;
            }
            b' ' | b'\t' | b'\r' => i += 1,
            b';' => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b':' => {
                out.push((Tok::Colon, line));
                i += 1;
            }
            b',' => {
                out.push((Tok::Comma, line));
                i += 1;
            }
            b'{' => {
                out.push((Tok::Open, line));
                i += 1;
            }
            b'}' => {
                out.push((Tok::Close, line));
                i += 1;
            }
            b'"' => {
                let s = i + 1;
                i = s;
                while i < b.len() && b[i] != b'"' {
                    if b[i] == b'\n' {
                        line += 1;
                    }
                    i += 1;
                }
                if i >= b.len() {
                    return Err(format!("line {line}: string is not closed"));
                }
                out.push((Tok::Str(text[s..i].replace("&quot;", "\"")), line));
                i += 1;
            }
            b'*' => {
                let s = i + 1;
                i = s;
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
                let n = text[s..i]
                    .parse()
                    .map_err(|_| format!("line {line}: bad array length"))?;
                out.push((Tok::Star(n), line));
            }
            c if c.is_ascii_digit() || c == b'-' || c == b'+' || c == b'.' => {
                let s = i;
                i += 1;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || matches!(b[i], b'.' | b'-' | b'+')) {
                    i += 1;
                }
                out.push((Tok::Num(text[s..i].to_string()), line));
            }
            c if c.is_ascii_alphabetic() || c == b'_' || c == b'|' => {
                let s = i;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || matches!(b[i], b'_' | b'|')) {
                    i += 1;
                }
                out.push((Tok::Word(text[s..i].to_string()), line));
            }
            _ => return Err(format!("line {line}: unexpected character `{}`", c as char)),
        }
    }
    Ok(out)
}

struct Ascii {
    toks: Vec<(Tok, usize)>,
    pos: usize,
    limits: TreeLimits,
    nodes: usize,
}

impl Ascii {
    fn peek(&self, k: usize) -> Option<&Tok> {
        self.toks.get(self.pos + k).map(|t| &t.0)
    }
    fn line(&self) -> usize {
        self.toks.get(self.pos).or(self.toks.last()).map_or(0, |t| t.1)
    }
    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).map(|t| t.0.clone());
        self.pos += 1;
        t
    }

    fn number(s: &str) -> Prop {
        if let Ok(v) = s.parse::<i64>() {
            Prop::I64(v)
        } else {
            Prop::F64(s.parse::<f64>().unwrap_or(0.0))
        }
    }

    /// Nodes until `}` or end of input.
    fn nodes(&mut self, depth: usize) -> Result<Vec<Node>, String> {
        if depth > self.limits.max_depth {
            return Err("FBX nodes are nested too deeply".into());
        }
        let mut out = Vec::new();
        loop {
            match self.peek(0) {
                None | Some(Tok::Close) => return Ok(out),
                Some(Tok::Word(_)) if self.peek(1) == Some(&Tok::Colon) => {
                    out.push(self.node(depth)?);
                }
                _ => return Err(format!("line {}: expected a node name", self.line())),
            }
        }
    }

    fn node(&mut self, depth: usize) -> Result<Node, String> {
        let Some(Tok::Word(name)) = self.next() else {
            unreachable!("checked by the caller")
        };
        self.next(); // colon
        self.nodes += 1;
        if self.nodes > self.limits.max_nodes {
            return Err("FBX has more nodes than the import limit".into());
        }
        let mut props = Vec::new();
        let mut children = Vec::new();
        loop {
            match self.peek(0) {
                Some(Tok::Star(_)) => {
                    let Some(Tok::Star(n)) = self.next() else {
                        unreachable!()
                    };
                    if n.saturating_mul(8) > self.limits.max_array_bytes {
                        return Err(format!(
                            "line {}: array larger than the import limit",
                            self.line()
                        ));
                    }
                    if self.next() != Some(Tok::Open) {
                        return Err(format!("line {}: expected `{{` after array length", self.line()));
                    }
                    let mut vals: Vec<String> = Vec::with_capacity(n);
                    if matches!(self.peek(0), Some(Tok::Word(w)) if w == "a") {
                        self.next();
                        self.next();
                        loop {
                            match self.next() {
                                Some(Tok::Num(s)) => vals.push(s),
                                Some(Tok::Comma) => {}
                                Some(Tok::Close) => break,
                                _ => return Err(format!("line {}: bad array value", self.line())),
                            }
                        }
                    } else if self.next() != Some(Tok::Close) {
                        return Err(format!("line {}: bad array", self.line()));
                    }
                    if vals.iter().all(|v| v.parse::<i64>().is_ok()) {
                        props.push(Prop::I64Arr(vals.iter().map(|v| v.parse().unwrap()).collect()));
                    } else {
                        props.push(Prop::F64Arr(
                            vals.iter().map(|v| v.parse().unwrap_or(0.0)).collect(),
                        ));
                    }
                }
                Some(Tok::Num(_)) => {
                    let Some(Tok::Num(s)) = self.next() else {
                        unreachable!()
                    };
                    props.push(Self::number(&s));
                }
                Some(Tok::Str(_)) => {
                    let Some(Tok::Str(s)) = self.next() else {
                        unreachable!()
                    };
                    props.push(Prop::Str(s));
                }
                Some(Tok::Word(_)) if self.peek(1) != Some(&Tok::Colon) => {
                    let Some(Tok::Word(s)) = self.next() else {
                        unreachable!()
                    };
                    props.push(Prop::Str(s));
                }
                _ => {}
            }
            match self.peek(0) {
                Some(Tok::Comma) => {
                    self.next();
                }
                Some(Tok::Open) => {
                    self.next();
                    children = self.nodes(depth + 1)?;
                    if self.next() != Some(Tok::Close) {
                        return Err(format!("line {}: `{name}` is not closed", self.line()));
                    }
                    break;
                }
                _ => break,
            }
        }
        // Old-style ASCII arrays written as plain comma lists: collapse many numbers into one
        // array property for the array-valued nodes the scene reader asks for.
        if props.len() > 1 && is_array_node(&name) {
            let all_int = props.iter().all(|p| matches!(p, Prop::I64(_)));
            props = if all_int {
                vec![Prop::I64Arr(props.iter().filter_map(Prop::as_i64).collect())]
            } else {
                vec![Prop::F64Arr(props.iter().filter_map(Prop::as_f64).collect())]
            };
        }
        // ASCII 7.x array nodes keep their values in an `a` child.
        if props.is_empty() && children.len() == 1 && children[0].name == "a" {
            props = std::mem::take(&mut children[0].props);
            children.clear();
        }
        Ok(Node {
            name,
            props,
            children,
        })
    }
}

fn is_array_node(name: &str) -> bool {
    matches!(
        name,
        "Vertices"
            | "PolygonVertexIndex"
            | "Edges"
            | "Normals"
            | "NormalsIndex"
            | "NormalsW"
            | "UV"
            | "UVIndex"
            | "Materials"
            | "Colors"
            | "ColorIndex"
            | "Indexes"
            | "Weights"
            | "Transform"
            | "TransformLink"
            | "Matrix"
            | "KeyTime"
            | "KeyValueFloat"
            | "KeyAttrFlags"
            | "KeyAttrDataFloat"
            | "KeyAttrRefCount"
    )
}

fn parse_ascii(bytes: &[u8], limits: TreeLimits) -> Result<Document, String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "not an FBX file: neither binary FBX nor UTF-8 text".to_string())?;
    if !text.contains("FBXHeaderExtension") {
        return Err("not an FBX file: no binary header and no FBXHeaderExtension".into());
    }
    let mut p = Ascii {
        toks: tokenize(text)?,
        pos: 0,
        limits,
        nodes: 0,
    };
    let roots = p.nodes(0)?;
    if p.pos < p.toks.len() {
        return Err(format!("line {}: unexpected `}}`", p.line()));
    }
    let version = roots
        .iter()
        .find(|n| n.name == "FBXHeaderExtension")
        .and_then(|h| h.value("FBXVersion"))
        .and_then(Prop::as_i64)
        .unwrap_or(0) as u32;
    Ok(Document {
        version,
        binary: false,
        roots,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_nodes_arrays_and_names() {
        let text = r#"; FBX 7.4.0 project file
FBXHeaderExtension:  {
	FBXVersion: 7400
}
Objects:  {
	Geometry: 100, "Geometry::Box", "Mesh" {
		Vertices: *6 {
			a: 0,0,0,1.5,-2,3e2
		}
		PolygonVertexIndex: *3 {
			a: 0,1,-3
		}
		Shading: T
	}
}
"#;
        let doc = parse(text.as_bytes(), TreeLimits::default()).unwrap();
        assert_eq!(doc.version, 7400);
        assert!(!doc.binary);
        let g = doc.root("Objects").unwrap().child("Geometry").unwrap();
        assert_eq!(g.props[0], Prop::I64(100));
        assert_eq!(g.props[1], Prop::Str("Geometry::Box".into()));
        assert_eq!(
            g.array("Vertices").unwrap().f64s().unwrap(),
            vec![0.0, 0.0, 0.0, 1.5, -2.0, 300.0]
        );
        assert_eq!(
            g.array("PolygonVertexIndex").unwrap().i64s().unwrap(),
            vec![0, 1, -3]
        );
        assert_eq!(g.value("Shading").unwrap().as_str(), Some("T"));
    }

    #[test]
    fn binary_rejects_truncation_and_bad_offsets() {
        let mut b = BINARY_MAGIC.to_vec();
        b.extend_from_slice(&[0x1a, 0]);
        b.extend_from_slice(&7400u32.to_le_bytes());
        // A node whose end offset points past the file.
        b.extend_from_slice(&9999u32.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        b.push(1);
        b.push(b'A');
        b.extend_from_slice(&[0; 13]);
        assert!(parse(&b, TreeLimits::default()).is_err());
        assert!(parse(&b[..20], TreeLimits::default()).is_err());
    }

    #[test]
    fn not_fbx_is_an_error() {
        assert!(parse(b"hello", TreeLimits::default()).is_err());
        assert!(parse(&[0xff, 0xfe, 0x00], TreeLimits::default()).is_err());
    }
}
