//! A small lexer for C-family source (C# and C++). It is not a parser: it removes comments,
//! keeps string and character literals as single tokens, records preprocessor lines, and
//! gives every token its 1-based line. That is enough to find classes, members and engine
//! API calls reliably in ordinary gameplay code.

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Ident,
    Number,
    Str,
    Punct,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: Kind,
    pub text: String,
    pub line: u32,
}

impl Token {
    pub fn is(&self, s: &str) -> bool {
        self.text == s && self.kind != Kind::Str
    }
    pub fn ident(&self) -> Option<&str> {
        (self.kind == Kind::Ident).then_some(self.text.as_str())
    }
}

/// Multi-character punctuation, longest first.
const PUNCT: [&str; 22] = [
    "<<=", ">>=", "::", "->", "=>", "==", "!=", "<=", ">=", "&&", "||", "++", "--", "+=", "-=", "*=", "/=",
    "%=", "&=", "|=", "^=", "??",
];

pub struct Lexed {
    pub tokens: Vec<Token>,
    /// Preprocessor lines (`#include ...`, `#if ...`) with their line numbers.
    pub directives: Vec<(u32, String)>,
    /// Source lines, for quoting original code in skeletons.
    pub lines: Vec<String>,
}

pub fn lex(src: &str) -> Lexed {
    let chars: Vec<char> = src.chars().collect();
    let mut tokens = Vec::new();
    let mut directives = Vec::new();
    let mut i = 0;
    let mut line = 1u32;
    let mut at_line_start = true;
    let n = chars.len();
    while i < n {
        let c = chars[i];
        if c == '\n' {
            line += 1;
            i += 1;
            at_line_start = true;
            continue;
        }
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        // Preprocessor directive (C# and C++): '#' first on its line, runs to end of line
        // (with backslash continuation).
        if c == '#' && at_line_start {
            let start_line = line;
            let mut text = String::new();
            while i < n && chars[i] != '\n' {
                if chars[i] == '\\' && i + 1 < n && chars[i + 1] == '\n' {
                    i += 2;
                    line += 1;
                    continue;
                }
                text.push(chars[i]);
                i += 1;
            }
            directives.push((start_line, text.trim().to_string()));
            continue;
        }
        at_line_start = false;
        // Comments.
        if c == '/' && i + 1 < n && chars[i + 1] == '/' {
            while i < n && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && i + 1 < n && chars[i + 1] == '*' {
            i += 2;
            while i < n && !(chars[i] == '*' && i + 1 < n && chars[i + 1] == '/') {
                if chars[i] == '\n' {
                    line += 1;
                }
                i += 1;
            }
            i = (i + 2).min(n);
            continue;
        }
        // C++ raw string R"delim( ... )delim".
        if c == 'R' && i + 1 < n && chars[i + 1] == '"' {
            let start_line = line;
            let mut j = i + 2;
            let mut delim = String::new();
            while j < n && chars[j] != '(' && delim.len() < 16 {
                delim.push(chars[j]);
                j += 1;
            }
            let close: Vec<char> = format!("){delim}\"").chars().collect();
            j += 1;
            while j < n && !chars[j..].starts_with(&close) {
                if chars[j] == '\n' {
                    line += 1;
                }
                j += 1;
            }
            let end = (j + close.len()).min(n);
            tokens.push(Token {
                kind: Kind::Str,
                text: chars[i..end].iter().collect(),
                line: start_line,
            });
            i = end;
            continue;
        }
        // C# verbatim / interpolated strings: @"..", $"..", $@"..", @$"..".
        let mut prefix = 0;
        while i + prefix < n && prefix < 2 && (chars[i + prefix] == '@' || chars[i + prefix] == '$') {
            prefix += 1;
        }
        if prefix > 0 && i + prefix < n && chars[i + prefix] == '"' {
            let verbatim = chars[i..i + prefix].contains(&'@');
            let start_line = line;
            let mut j = i + prefix + 1;
            while j < n {
                if chars[j] == '\n' {
                    line += 1;
                }
                if verbatim && chars[j] == '"' && j + 1 < n && chars[j + 1] == '"' {
                    j += 2;
                    continue;
                }
                if !verbatim && chars[j] == '\\' {
                    j += 2;
                    continue;
                }
                if chars[j] == '"' {
                    break;
                }
                j += 1;
            }
            let end = (j + 1).min(n);
            tokens.push(Token {
                kind: Kind::Str,
                text: chars[i..end].iter().collect(),
                line: start_line,
            });
            i = end;
            continue;
        }
        if c == '"' || c == '\'' {
            let mut j = i + 1;
            while j < n && chars[j] != c && chars[j] != '\n' {
                if chars[j] == '\\' {
                    j += 1;
                }
                j += 1;
            }
            let end = (j + 1).min(n);
            tokens.push(Token {
                kind: Kind::Str,
                text: chars[i..end].iter().collect(),
                line,
            });
            i = end;
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            let mut j = i;
            while j < n && (chars[j].is_alphanumeric() || chars[j] == '_') {
                j += 1;
            }
            tokens.push(Token {
                kind: Kind::Ident,
                text: chars[i..j].iter().collect(),
                line,
            });
            i = j;
            continue;
        }
        if c.is_ascii_digit() || (c == '.' && i + 1 < n && chars[i + 1].is_ascii_digit()) {
            let mut j = i;
            while j < n && (chars[j].is_alphanumeric() || chars[j] == '.' || chars[j] == '_') {
                // Stop at a member access after a number like `1.ToString` (rare) only if the
                // char after '.' is a letter and we already have a '.'.
                j += 1;
            }
            tokens.push(Token {
                kind: Kind::Number,
                text: chars[i..j].iter().collect(),
                line,
            });
            i = j;
            continue;
        }
        let rest: String = chars[i..(i + 3).min(n)].iter().collect();
        let p = PUNCT.iter().find(|p| rest.starts_with(**p));
        let text = match p {
            Some(p) => (*p).to_string(),
            None => c.to_string(),
        };
        i += text.chars().count();
        tokens.push(Token {
            kind: Kind::Punct,
            text,
            line,
        });
    }
    Lexed {
        tokens,
        directives,
        lines: src.lines().map(str::to_string).collect(),
    }
}

/// Index of the token that closes the bracket opened at `open` (`(`, `[`, `{` or `<`), or
/// `None` if it is not closed. For `<` the search gives up at `;`, `{` or `}` so comparisons
/// are not mistaken for generics.
pub fn matching(tokens: &[Token], open: usize) -> Option<usize> {
    let (o, c) = match tokens.get(open)?.text.as_str() {
        "(" => ("(", ")"),
        "[" => ("[", "]"),
        "{" => ("{", "}"),
        "<" => ("<", ">"),
        _ => return None,
    };
    let mut depth = 0i32;
    for (k, t) in tokens.iter().enumerate().skip(open) {
        if t.kind == Kind::Str {
            continue;
        }
        if o == "<" {
            if t.is(";") || t.is("{") || t.is("}") {
                return None;
            }
            if t.is(">>") {
                depth -= 2;
                if depth <= 0 {
                    return Some(k);
                }
                continue;
            }
        }
        if t.is(o) {
            depth += 1;
        } else if t.is(c) {
            depth -= 1;
            if depth == 0 {
                return Some(k);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(src: &str) -> Vec<String> {
        lex(src).tokens.into_iter().map(|t| t.text).collect()
    }

    #[test]
    fn comments_and_strings_are_handled() {
        let l = lex("a // x.y()\n/* Input.GetAxis(\"x\") \n */ b \"Time.deltaTime\" @\"c\"\"d\" 'e'");
        let t: Vec<_> = l.tokens.iter().map(|t| (t.text.as_str(), t.line)).collect();
        assert_eq!(
            t,
            vec![
                ("a", 1),
                ("b", 3),
                ("\"Time.deltaTime\"", 3),
                ("@\"c\"\"d\"", 3),
                ("'e'", 3)
            ]
        );
    }

    #[test]
    fn punctuation_and_directives() {
        let l = lex("#include \"A.h\"\nGetWorld()->GetTimerManager().SetTimer(H, this, &A::F, 1.5f, true);");
        assert_eq!(l.directives, vec![(1, "#include \"A.h\"".to_string())]);
        assert!(texts("a->b::c").contains(&"->".to_string()));
        assert!(texts("a->b::c").contains(&"::".to_string()));
        assert!(texts("x = 1.5f;").contains(&"1.5f".to_string()));
    }

    #[test]
    fn matching_brackets() {
        let l = lex("List<Dictionary<int, string>> x; f(a, (b)) { }");
        // `>>` is two tokens; the outer `<` closes at the second one.
        assert_eq!(matching(&l.tokens, 1), Some(8));
        let open = l.tokens.iter().position(|t| t.is("(")).unwrap();
        assert_eq!(l.tokens[matching(&l.tokens, open).unwrap()].line, 1);
    }
}
