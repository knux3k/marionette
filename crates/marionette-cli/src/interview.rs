//! `marionette make` on a terminal: asks the game interview's questions (decision 0151).
//!
//! The questions come from the `game_interview` operation as JSON, so the terminal and an
//! agent see the same wording and guesses. Reading and writing go through `BufRead`/`Write`
//! parameters instead of stdin/stderr directly so the tests can drive it with byte buffers.

use serde_json::{Map, Value};
use std::io::{BufRead, Write};

/// Asks each question in `questions` (the `questions` array of `game_interview`) and returns
/// the replies as a JSON object by question id. An empty line keeps the guess and is left out,
/// so the caller can tell answered from guessed. A choice may be typed as its number or its
/// value. End of input stops asking; the rest keep their guesses.
pub(crate) fn ask(
    questions: &[Value],
    input: &mut impl BufRead,
    out: &mut impl Write,
) -> std::io::Result<Map<String, Value>> {
    let mut answers = Map::new();
    writeln!(
        out,
        "A few questions so the right game gets made. Press Enter to keep the guess in [brackets]."
    )?;
    for q in questions {
        let Some(id) = q["id"].as_str() else { continue };
        let choices: Vec<(&str, &str)> = q["choices"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|c| Some((c["value"].as_str()?, c["means"].as_str().unwrap_or(""))))
            .collect();
        let guess = q["guess"].as_str().unwrap_or("");
        writeln!(out, "\n{}", q["ask"].as_str().unwrap_or(id))?;
        for (i, (value, means)) in choices.iter().enumerate() {
            writeln!(out, "  {}. {value}: {means}", i + 1)?;
        }
        if let Some(ex) = q["examples"].as_array().filter(|a| !a.is_empty()) {
            let ex: Vec<&str> = ex.iter().filter_map(Value::as_str).collect();
            writeln!(out, "  e.g. {}", ex.join(", "))?;
        }
        loop {
            write!(out, "[{guess}] > ")?;
            out.flush()?;
            let mut line = String::new();
            if input.read_line(&mut line)? == 0 {
                return Ok(answers);
            }
            let reply = line.trim();
            if reply.is_empty() {
                break;
            }
            if choices.is_empty() {
                answers.insert(id.to_string(), Value::String(reply.to_string()));
                break;
            }
            let picked = reply
                .parse::<usize>()
                .ok()
                .and_then(|n| n.checked_sub(1))
                .and_then(|i| choices.get(i))
                .or_else(|| choices.iter().find(|(v, _)| v.eq_ignore_ascii_case(reply)));
            match picked {
                Some((value, _)) => {
                    answers.insert(id.to_string(), Value::String((*value).to_string()));
                    break;
                }
                None => writeln!(out, "  pick a number from 1 to {}", choices.len())?,
            }
        }
    }
    Ok(answers)
}

/// `id=value` pairs from `--answer` into a JSON object; a pair without `=` is an error.
pub(crate) fn parse_pairs(pairs: &[String]) -> Result<Map<String, Value>, String> {
    let mut out = Map::new();
    for p in pairs {
        let (k, v) = p
            .split_once('=')
            .ok_or_else(|| format!("--answer `{p}` is not id=value (e.g. --answer genre=maze)"))?;
        out.insert(k.trim().to_string(), Value::String(v.trim().to_string()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn numbers_values_and_enter_keep_guess() {
        let qs = vec![
            json!({"id": "genre", "ask": "Kind?", "guess": "maze",
                   "choices": [{"value": "maze", "means": "m"}, {"value": "platformer", "means": "p"}]}),
            json!({"id": "view", "ask": "View?", "guess": "2d",
                   "choices": [{"value": "2d", "means": ""}, {"value": "3d", "means": ""}]}),
            json!({"id": "player", "ask": "Who?", "guess": "a moth", "examples": ["a cat"]}),
            json!({"id": "title", "ask": "Title?", "guess": "Moth"}),
        ];
        // "9" is out of range and asked again; "3D" matches a value; Enter keeps the guess.
        let mut input = "2\n9\n3D\na penguin\n\n".as_bytes();
        let mut out = Vec::new();
        let a = ask(&qs, &mut input, &mut out).unwrap();
        assert_eq!(a["genre"], "platformer");
        assert_eq!(a["view"], "3d");
        assert_eq!(a["player"], "a penguin");
        assert!(!a.contains_key("title"), "Enter keeps the guess");
        assert!(
            String::from_utf8(out)
                .unwrap()
                .contains("pick a number from 1 to 2")
        );
    }

    #[test]
    fn answer_pairs() {
        let a = parse_pairs(&["genre=maze".into(), "player = a red fox ".into()]).unwrap();
        assert_eq!(a["player"], "a red fox");
        assert!(parse_pairs(&["genre".into()]).is_err());
    }
}
