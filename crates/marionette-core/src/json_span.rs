//! Exact source positions for JSON values, keyed by JSON Pointer (RFC 6901).
//!
//! `serde_json::Value` does not keep positions, so after a successful parse we scan the
//! text once more and record where every value starts. For an object member the position
//! is that of its key; for an array element, the element itself. Lines and columns are
//! 1-based; columns count characters.
//!
//! Positions are only read when a problem is reported, so the scan runs on the first
//! [`Spans::get`], not when the text is loaded (decision 0270: a valid scene pays nothing).

use std::cell::OnceCell;
use std::collections::BTreeMap;

type Map = BTreeMap<String, (usize, usize)>;

#[derive(Debug, Clone)]
pub struct Spans<'a> {
    text: &'a str,
    map: OnceCell<Map>,
}

impl Spans<'_> {
    /// Position of the value at `pointer` (e.g. `/entities/0/components/Position`).
    pub fn get(&self, pointer: &str) -> Option<(usize, usize)> {
        self.map.get_or_init(|| scan(self.text)).get(pointer).copied()
    }
}

/// Builds a JSON Pointer from path segments, escaping `~` and `/`.
pub fn pointer<I, S>(segments: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    segments.into_iter().fold(String::new(), |mut acc, s| {
        acc.push('/');
        acc.push_str(&s.as_ref().replace('~', "~0").replace('/', "~1"));
        acc
    })
}

/// Value positions of `text` (which must already be valid JSON), found on first use.
pub fn spans(text: &str) -> Spans<'_> {
    Spans {
        text,
        map: OnceCell::new(),
    }
}

fn scan(text: &str) -> Map {
    let mut s = Scanner {
        chars: text.chars().collect(),
        i: 0,
        line: 1,
        col: 1,
        spans: Map::new(),
    };
    s.ws();
    s.value(String::new(), None);
    s.spans
}

struct Scanner {
    chars: Vec<char>,
    i: usize,
    line: usize,
    col: usize,
    spans: Map,
}

impl Scanner {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.i).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.i += 1;
        if c == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(c)
    }

    fn ws(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t' | '\n' | '\r')) {
            self.bump();
        }
    }

    /// Reads one value. `at` is the position to record (the key's position for object
    /// members); `None` means "where the value starts".
    fn value(&mut self, path: String, at: Option<(usize, usize)>) {
        let here = at.unwrap_or((self.line, self.col));
        self.spans.insert(path.clone(), here);
        match self.peek() {
            Some('{') => {
                self.bump();
                self.ws();
                while let Some(c) = self.peek() {
                    if c == '}' {
                        break;
                    }
                    let key_pos = (self.line, self.col);
                    let key = self.string();
                    self.ws();
                    self.bump(); // ':'
                    self.ws();
                    self.value(format!("{path}{}", pointer([key])), Some(key_pos));
                    self.ws();
                    if self.peek() == Some(',') {
                        self.bump();
                        self.ws();
                    }
                }
                self.bump();
            }
            Some('[') => {
                self.bump();
                self.ws();
                let mut index = 0usize;
                while let Some(c) = self.peek() {
                    if c == ']' {
                        break;
                    }
                    self.value(format!("{path}/{index}"), None);
                    index += 1;
                    self.ws();
                    if self.peek() == Some(',') {
                        self.bump();
                        self.ws();
                    }
                }
                self.bump();
            }
            Some('"') => {
                self.string();
            }
            _ => {
                while matches!(self.peek(), Some(c) if !matches!(c, ',' | '}' | ']' | ' ' | '\t' | '\n' | '\r'))
                {
                    self.bump();
                }
            }
        }
    }

    /// Reads a string literal and returns its decoded content (escapes resolved enough for
    /// key matching: `\"`, `\\`, `\/`, `\n`, `\t`, `\r`, `\b`, `\f`, `\uXXXX`).
    fn string(&mut self) -> String {
        let mut out = String::new();
        self.bump(); // opening quote
        while let Some(c) = self.bump() {
            match c {
                '"' => break,
                '\\' => match self.bump() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some('r') => out.push('\r'),
                    Some('b') => out.push('\u{8}'),
                    Some('f') => out.push('\u{c}'),
                    Some('u') => {
                        let hex: String = (0..4).filter_map(|_| self.bump()).collect();
                        if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                            out.push(ch);
                        }
                    }
                    Some(other) => out.push(other),
                    None => break,
                },
                other => out.push(other),
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_exact_positions() {
        let text = "{\n  \"a\": [1, {\"b/c\": true}],\n  \"d\": \"x\"\n}";
        let s = spans(text);
        assert_eq!(s.get(""), Some((1, 1)));
        assert_eq!(s.get("/a"), Some((2, 3)));
        assert_eq!(s.get("/a/0"), Some((2, 9)));
        assert_eq!(s.get("/a/1"), Some((2, 12)));
        assert_eq!(s.get(&pointer(["a", "1", "b/c"])), Some((2, 13)));
        assert_eq!(s.get("/d"), Some((3, 3)));
    }

    #[test]
    fn works_on_minified_text() {
        let s = spans(r#"{"entities":[{"id":5,"components":{"P":{}}}]}"#);
        assert_eq!(s.get("/entities/0/components/P"), Some((1, 36)));
    }
}
