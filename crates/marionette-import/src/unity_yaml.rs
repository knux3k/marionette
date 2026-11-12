//! A reader for Unity's text serialization (the YAML subset Unity writes for `.prefab`,
//! `.unity` and `.asset` files). Each document becomes a [`UnityDoc`] whose body is a JSON
//! value; every scalar is kept as a string so nothing is lost or rounded while reading.
//!
//! Handled: `--- !u!<class> &<fileID> [stripped]` headers, block mappings, block sequences
//! (also at the same indent as their key, as Unity writes them), flow mappings and sequences
//! (also wrapped over several lines), plain, single- and double-quoted scalars, and plain
//! scalars continued on more-indented lines. Not handled: anchors and aliases inside a body,
//! tags other than the document header, block scalars (`|`, `>`); Unity does not write them.

use serde_json::{Map, Value};

/// One object of a Unity file.
#[derive(Debug, Clone)]
pub struct UnityDoc {
    /// Unity class id (1 GameObject, 4 Transform, 1001 PrefabInstance, ...).
    pub class_id: u32,
    /// The object's local file id, as written (may be negative).
    pub file_id: i64,
    /// A placeholder for an object that lives in a source prefab.
    pub stripped: bool,
    /// The top-level key (`GameObject`, `Transform`, ...).
    pub type_name: String,
    /// The mapping under the top-level key.
    pub body: Value,
}

/// Reads every document of a text-serialized Unity file.
pub fn parse(text: &str) -> Result<Vec<UnityDoc>, String> {
    if !text.starts_with("%YAML") {
        return Err("not a text-serialized Unity file (binary serialization is not read)".into());
    }
    let mut docs = Vec::new();
    let mut header: Option<(u32, i64, bool)> = None;
    let mut body: Vec<&str> = Vec::new();
    let mut flush = |header: Option<(u32, i64, bool)>, body: &mut Vec<&str>| -> Result<(), String> {
        if let Some((class_id, file_id, stripped)) = header {
            let (type_name, value) = parse_body(body)
                .map_err(|e| format!("object &{file_id} (class {class_id}): {e}"))?;
            docs.push(UnityDoc {
                class_id,
                file_id,
                stripped,
                type_name,
                body: value,
            });
        }
        body.clear();
        Ok(())
    };
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("--- ") {
            flush(header.take(), &mut body)?;
            header = Some(parse_header(rest)?);
        } else if line.starts_with('%') {
            continue;
        } else if header.is_some() {
            body.push(line);
        }
    }
    flush(header.take(), &mut body)?;
    Ok(docs)
}

fn parse_header(rest: &str) -> Result<(u32, i64, bool), String> {
    let mut class_id = None;
    let mut file_id = None;
    let mut stripped = false;
    for part in rest.split_whitespace() {
        if let Some(c) = part.strip_prefix("!u!") {
            class_id = c.parse().ok();
        } else if let Some(f) = part.strip_prefix('&') {
            file_id = f.parse().ok();
        } else if part == "stripped" {
            stripped = true;
        }
    }
    match (class_id, file_id) {
        (Some(c), Some(f)) => Ok((c, f, stripped)),
        _ => Err(format!("unreadable document header `--- {rest}`")),
    }
}

#[derive(Debug, Clone)]
struct Line {
    indent: usize,
    text: String,
}

fn parse_body(raw: &[&str]) -> Result<(String, Value), String> {
    let mut lines: Vec<Line> = Vec::new();
    for l in raw {
        let t = l.trim_end();
        if t.trim().is_empty() || t.trim_start().starts_with('#') {
            continue;
        }
        let indent = t.len() - t.trim_start().len();
        lines.push(Line {
            indent,
            text: t.trim_start().to_string(),
        });
    }
    join_wrapped(&mut lines);
    let Some(first) = lines.first().cloned() else {
        return Ok((String::new(), Value::Object(Map::new())));
    };
    let (key, rest) = split_key(&first.text).ok_or("the object has no type line")?;
    let mut i = 1;
    let value = if !rest.is_empty() {
        parse_inline(rest)?
    } else if i < lines.len() && lines[i].indent > first.indent {
        let ind = lines[i].indent;
        block(&mut lines, &mut i, ind)?
    } else {
        Value::Object(Map::new())
    };
    Ok((key.to_string(), value))
}

/// Joins flow collections wrapped over several lines and plain scalars continued on
/// more-indented lines into one logical line.
fn join_wrapped(lines: &mut Vec<Line>) {
    let mut out: Vec<Line> = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        let mut cur = lines[i].clone();
        i += 1;
        // Open brackets outside quotes.
        while depth(&cur.text) > 0 && i < lines.len() {
            cur.text.push(' ');
            cur.text.push_str(&lines[i].text);
            i += 1;
        }
        // A plain scalar value continued on deeper lines (not a key, not a list item).
        while i < lines.len()
            && lines[i].indent > cur.indent
            && value_part(&cur.text).is_some_and(|v| !v.is_empty() && !v.starts_with(['{', '[']))
            && split_key(&lines[i].text).is_none()
            && !lines[i].text.starts_with("- ")
            && lines[i].text != "-"
        {
            cur.text.push(' ');
            cur.text.push_str(&lines[i].text);
            i += 1;
        }
        out.push(cur);
    }
    *lines = out;
}

/// The value part of `key: value` or `- value`, if the line has one.
fn value_part(text: &str) -> Option<&str> {
    let t = text.strip_prefix("- ").unwrap_or(text);
    split_key(t).map(|(_, v)| v)
}

fn depth(s: &str) -> i32 {
    let mut d = 0;
    let mut quote: Option<char> = None;
    for c in s.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None => match c {
                '\'' | '"' => quote = Some(c),
                '{' | '[' => d += 1,
                '}' | ']' => d -= 1,
                _ => {}
            },
        }
    }
    d
}

/// `key: rest` or `key:` (rest empty). Keys never start with `{`, `[`, `-` or a quote.
fn split_key(text: &str) -> Option<(&str, &str)> {
    if text.starts_with(['{', '[', '\'', '"']) || text.starts_with("- ") || text == "-" {
        return None;
    }
    let pos = text.find(": ").or_else(|| text.ends_with(':').then(|| text.len() - 1))?;
    let key = &text[..pos];
    if key.is_empty() || key.contains(['{', '[', '"', '\'']) {
        return None;
    }
    Some((key, text[pos + 1..].trim()))
}

/// Reads a block (mapping or sequence) whose lines start at `indent`.
fn block(lines: &mut [Line], i: &mut usize, indent: usize) -> Result<Value, String> {
    if *i < lines.len() && is_item(&lines[*i].text) {
        sequence(lines, i, indent)
    } else {
        mapping(lines, i, indent)
    }
}

fn is_item(t: &str) -> bool {
    t.starts_with("- ") || t == "-"
}

fn mapping(lines: &mut [Line], i: &mut usize, indent: usize) -> Result<Value, String> {
    let mut map = Map::new();
    while *i < lines.len() && lines[*i].indent == indent && !is_item(&lines[*i].text) {
        let text = lines[*i].text.clone();
        let Some((key, rest)) = split_key(&text) else {
            return Err(format!("expected `key: value`, found `{text}`"));
        };
        *i += 1;
        let value = if !rest.is_empty() {
            parse_inline(rest)?
        } else if *i < lines.len() && lines[*i].indent > indent {
            let ind = lines[*i].indent;
            block(lines, i, ind)?
        } else if *i < lines.len() && lines[*i].indent == indent && is_item(&lines[*i].text) {
            sequence(lines, i, indent)?
        } else {
            Value::String(String::new())
        };
        map.insert(key.to_string(), value);
    }
    if *i < lines.len() && lines[*i].indent > indent {
        return Err(format!("unexpected indentation at `{}`", lines[*i].text));
    }
    Ok(Value::Object(map))
}

fn sequence(lines: &mut [Line], i: &mut usize, indent: usize) -> Result<Value, String> {
    let mut items = Vec::new();
    while *i < lines.len() && lines[*i].indent == indent && is_item(&lines[*i].text) {
        let text = lines[*i].text.clone();
        let rest = text.strip_prefix('-').unwrap_or("").trim_start();
        if rest.is_empty() {
            *i += 1;
            if *i < lines.len() && lines[*i].indent > indent {
                let ind = lines[*i].indent;
                items.push(block(lines, i, ind)?);
            } else {
                items.push(Value::String(String::new()));
            }
        } else if split_key(rest).is_some() {
            // `- key: value` starts a mapping whose keys are at indent + 2.
            let inner = indent + (text.len() - rest.len());
            lines[*i] = Line {
                indent: inner,
                text: rest.to_string(),
            };
            items.push(mapping(lines, i, inner)?);
        } else {
            *i += 1;
            items.push(parse_inline(rest)?);
        }
    }
    Ok(Value::Array(items))
}

/// A value written on one line: a flow collection or a scalar.
fn parse_inline(s: &str) -> Result<Value, String> {
    let mut p = Flow {
        s: s.as_bytes(),
        pos: 0,
        src: s,
    };
    let v = p.value()?;
    p.ws();
    if p.pos != p.s.len() {
        // Not a flow value after all (a plain scalar with brackets in it).
        return Ok(Value::String(s.trim().to_string()));
    }
    Ok(v)
}

struct Flow<'a> {
    s: &'a [u8],
    pos: usize,
    src: &'a str,
}

impl Flow<'_> {
    fn ws(&mut self) {
        while self.pos < self.s.len() && self.s[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.pos).copied()
    }

    fn value(&mut self) -> Result<Value, String> {
        self.ws();
        match self.peek() {
            Some(b'{') => self.map(),
            Some(b'[') => self.seq(),
            Some(b'\'') | Some(b'"') => self.quoted().map(Value::String),
            _ => Ok(Value::String(self.plain(false))),
        }
    }

    fn map(&mut self) -> Result<Value, String> {
        self.pos += 1;
        let mut m = Map::new();
        loop {
            self.ws();
            match self.peek() {
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Value::Object(m));
                }
                None => return Err(format!("unclosed `{{` in `{}`", self.src)),
                _ => {}
            }
            let key = match self.peek() {
                Some(b'\'') | Some(b'"') => self.quoted()?,
                _ => self.plain(true),
            };
            self.ws();
            let value = if self.peek() == Some(b':') {
                self.pos += 1;
                self.value()?
            } else {
                Value::String(String::new())
            };
            m.insert(key, value);
            self.ws();
            if self.peek() == Some(b',') {
                self.pos += 1;
            }
        }
    }

    fn seq(&mut self) -> Result<Value, String> {
        self.pos += 1;
        let mut items = Vec::new();
        loop {
            self.ws();
            match self.peek() {
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Value::Array(items));
                }
                None => return Err(format!("unclosed `[` in `{}`", self.src)),
                _ => {}
            }
            items.push(self.value()?);
            self.ws();
            if self.peek() == Some(b',') {
                self.pos += 1;
            }
        }
    }

    fn quoted(&mut self) -> Result<String, String> {
        let q = self.s[self.pos];
        self.pos += 1;
        let mut out: Vec<u8> = Vec::new();
        while self.pos < self.s.len() {
            let c = self.s[self.pos];
            self.pos += 1;
            if c == q {
                if q == b'\'' && self.peek() == Some(b'\'') {
                    out.push(b'\'');
                    self.pos += 1;
                    continue;
                }
                return Ok(String::from_utf8_lossy(&out).into_owned());
            }
            if q == b'"' && c == b'\\' && self.pos < self.s.len() {
                let e = self.s[self.pos];
                self.pos += 1;
                match e {
                    b'n' => out.push(b'\n'),
                    b't' => out.push(b'\t'),
                    b'u' if self.pos + 4 <= self.s.len() => {
                        let hex = std::str::from_utf8(&self.s[self.pos..self.pos + 4]).unwrap_or("");
                        if let Some(ch) = u32::from_str_radix(hex, 16).ok().and_then(char::from_u32) {
                            let mut buf = [0u8; 4];
                            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                        }
                        self.pos += 4;
                    }
                    other => out.push(other),
                }
                continue;
            }
            out.push(c);
        }
        Err(format!("unclosed quote in `{}`", self.src))
    }

    /// A plain scalar up to `,`, `}` or `]` (and `:` followed by a space, for keys).
    fn plain(&mut self, key: bool) -> String {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c == b',' || c == b'}' || c == b']' {
                break;
            }
            if key && c == b':' {
                break;
            }
            self.pos += 1;
        }
        String::from_utf8_lossy(&self.s[start..self.pos]).trim().to_string()
    }
}

/// Reads a number from a scalar value (`0`, `-1.5`, `1e-05`).
pub fn num(v: &Value) -> Option<f64> {
    match v {
        Value::String(s) => s.trim().parse().ok(),
        Value::Number(n) => n.as_f64(),
        _ => None,
    }
}

/// `{fileID: n, guid: g, type: t}` as (fileID, guid). `fileID: 0` means "none".
pub fn object_ref(v: &Value) -> Option<(i64, Option<String>)> {
    let id = v.get("fileID").and_then(num)? as i64;
    if id == 0 && v.get("guid").is_none() {
        return None;
    }
    let guid = v
        .get("guid")
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|g| !g.is_empty());
    Some((id, guid))
}

/// The file id from a reference, exactly (large ids do not survive a trip through f64).
pub fn ref_file_id(v: &Value) -> Option<i64> {
    match v.get("fileID")? {
        Value::String(s) => s.trim().parse().ok(),
        Value::Number(n) => n.as_i64(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "%YAML 1.1\n%TAG !u! tag:unity3d.com,2011:\n--- !u!1 &100\nGameObject:\n  m_ObjectHideFlags: 0\n  serializedVersion: 6\n  m_Component:\n  - component: {fileID: 400}\n  - component: {fileID: 500}\n  m_Name: Crate 'A'\n  m_IsActive: 1\n--- !u!4 &400\nTransform:\n  m_LocalRotation: {x: 0, y: 0.70710677, z: 0, w: 0.70710677}\n  m_Children:\n  - {fileID: 401}\n  m_Father: {fileID: 0}\n--- !u!114 &500\nMonoBehaviour:\n  m_Script: {fileID: 11500000, guid: 0123456789abcdef0123456789abcdef,\n    type: 3}\n  speed: 2.5\n  label: a long text that Unity\n    wrapped onto another line\n  tags:\n  - one\n  - two\n  nested:\n    - a: 1\n      b: '2'\n  empty: []\n--- !u!4 &401 stripped\nTransform:\n  m_CorrespondingSourceObject: {fileID: 7, guid: ffffffffffffffffffffffffffffffff, type: 3}\n";

    #[test]
    fn reads_unity_documents() {
        let docs = parse(SAMPLE).unwrap();
        assert_eq!(docs.len(), 4);
        assert_eq!((docs[0].class_id, docs[0].file_id), (1, 100));
        assert_eq!(docs[0].type_name, "GameObject");
        assert_eq!(docs[0].body["m_Name"], "Crate 'A'");
        assert_eq!(ref_file_id(&docs[0].body["m_Component"][1]["component"]), Some(500));
        assert_eq!(num(&docs[1].body["m_LocalRotation"]["w"]), Some(0.70710677));
        assert_eq!(ref_file_id(&docs[1].body["m_Children"][0]), Some(401));
        let mb = &docs[2].body;
        assert_eq!(
            object_ref(&mb["m_Script"]),
            Some((11500000, Some("0123456789abcdef0123456789abcdef".into())))
        );
        assert_eq!(mb["label"], "a long text that Unity wrapped onto another line");
        assert_eq!(mb["tags"], serde_json::json!(["one", "two"]));
        assert_eq!(mb["nested"][0]["b"], "2");
        assert_eq!(mb["empty"], serde_json::json!([]));
        assert!(docs[3].stripped);
    }

    #[test]
    fn binary_and_broken_files_are_errors() {
        assert!(parse("\0\0binary").is_err());
        assert!(parse("%YAML 1.1\n--- !u!1\nGameObject:\n").is_err());
        assert!(parse("%YAML 1.1\n--- !u!1 &1\nGameObject:\n  m_Name: {fileID: 1\n").is_err());
    }
}
