//! The interview (decision 0151): questions asked before a game is made from a prompt, so the
//! person gets the game they meant instead of the keyword reader's guess.
//!
//! The same questions serve both callers. An agent calls `game_interview`, asks the person the
//! questions it gets back in its own words, and passes the replies as `answers`; the CLI asks
//! them on the terminal (or takes `--answer id=value`). Every question carries the keyword
//! reader's guess, so an unanswered question still has a value and a reply of "keep the guess"
//! is cheap. Answers are plain strings keyed by question id; [`brief_from_answers`] folds them
//! into a [`Brief`].

use crate::{Brief, Difficulty, Size, Template, View, codes, level, prompt};
use marionette_core::{Diagnostic, Errors};
use schemars::JsonSchema;
use serde::Serialize;
use std::collections::BTreeMap;

/// Replies by question id ("genre" -> "platformer", "player" -> "a moth with glowing wings").
pub type Answers = BTreeMap<String, String>;

/// One fixed choice of a question.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Choice {
    pub value: &'static str,
    /// What picking it means for the game.
    pub means: &'static str,
}

/// One interview question. Static text is borrowed from the question table; only the guess
/// and the answer are owned, because they depend on the prompt.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Question {
    /// Key for `answers`.
    pub id: &'static str,
    /// The question, ready to ask a person.
    pub ask: &'static str,
    /// What the answer changes in the game.
    pub why: &'static str,
    /// Fixed choices; empty for a free-text question.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<Choice>,
    /// Example replies for a free-text question.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<&'static str>,
    /// What the game gets if the question is not answered (the keyword reader's reading of the
    /// prompt and the answers so far). Empty means "nothing".
    pub guess: String,
    /// The reply already given, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,
}

struct Spec {
    id: &'static str,
    ask: &'static str,
    why: &'static str,
    choices: &'static [Choice],
    examples: &'static [&'static str],
}

const GENRES: &[Choice] = &[
    Choice {
        value: "maze",
        means: "move in four directions (WASD, arrows or left stick), collect every pickup while enemies wander, then reach the goal; 2D top-down or 3D",
    },
    Choice {
        value: "platformer",
        means: "2D side view: run (A/D, arrows, stick) and jump (Space, W, Up, pad South) over gaps and spikes, stomp enemies, collect pickups and reach the goal before time runs out",
    },
];
const VIEWS: &[Choice] = &[
    Choice {
        value: "2d",
        means: "flat sprites seen from above",
    },
    Choice {
        value: "3d",
        means: "3D models with shadows, sky light and glow",
    },
];
const DIFFICULTIES: &[Choice] = &[
    Choice {
        value: "easy",
        means: "fewer, slower enemies; relaxed",
    },
    Choice {
        value: "normal",
        means: "a fair challenge",
    },
    Choice {
        value: "hard",
        means: "more, faster enemies",
    },
];
const LENGTHS: &[Choice] = &[
    Choice {
        value: "small",
        means: "a level of a minute or two",
    },
    Choice {
        value: "medium",
        means: "a few minutes",
    },
    Choice {
        value: "large",
        means: "a long level",
    },
];

/// The question table, in asking order. `setting` comes before the slot questions because it
/// changes their guesses (theme colors, walls, floors, backdrop).
const SPECS: &[Spec] = &[
    Spec {
        id: "genre",
        ask: "What kind of game is it: a maze or a platformer?",
        why: "Picks the template: the core loop, the controls and the autopilot that proves the level can be won.",
        choices: GENRES,
        examples: &[],
    },
    Spec {
        id: "view",
        ask: "Should the maze be 2D (sprites from above) or 3D (models)?",
        why: "2D draws sprites; 3D draws models with shadows and lighting (and costs more to generate).",
        choices: VIEWS,
        examples: &[],
    },
    Spec {
        id: "setting",
        ask: "Where does it take place?",
        why: "Sets the colors and the look of walls, floors and the backdrop.",
        choices: &[],
        examples: &[
            "space", "ocean", "forest", "dungeon", "candy", "desert", "snow", "city",
        ],
    },
    Spec {
        id: "player",
        ask: "Who or what does the player control? Describe how it looks.",
        why: "The player's art (the generation prompt when nothing is found) and the title.",
        choices: &[],
        examples: &["a small moth with glowing wings", "a courier robot"],
    },
    Spec {
        id: "pickups",
        ask: "What does the player collect? (plural)",
        why: "The pickup art and the HUD word.",
        choices: &[],
        examples: &["paper lanterns", "star crystals"],
    },
    Spec {
        id: "pickup_count",
        ask: "How many must the player collect? (a number from 1 to 12)",
        why: "Exactly this many pickups are placed in the maze; the HUD and win check use the real count.",
        choices: &[],
        examples: &["3", "7"],
    },
    Spec {
        id: "enemy",
        ask: "What must the player avoid?",
        why: "The enemy art.",
        choices: &[],
        examples: &["a hungry night bat", "a security drone"],
    },
    Spec {
        id: "enemy_count",
        ask: "How many enemies? (a number from 0 to 6)",
        why: "Exactly this many enemies are placed in the maze.",
        choices: &[],
        examples: &["1", "3"],
    },
    Spec {
        id: "goal",
        ask: "What does the player reach to win?",
        why: "The goal art.",
        choices: &[],
        examples: &["a mossy stone archway", "a warp portal"],
    },
    Spec {
        id: "style",
        ask: "What art style?",
        why: "Added to every art prompt, so all assets match.",
        choices: &[],
        examples: &[
            "crisp pixel art",
            "hand-painted storybook",
            "stylised low-poly",
            "glowing neon",
        ],
    },
    Spec {
        id: "difficulty",
        ask: "How hard should it be?",
        why: "Enemy count and speed (and time in the platformer).",
        choices: DIFFICULTIES,
        examples: &[],
    },
    Spec {
        id: "length",
        ask: "How long should a level be?",
        why: "Level size.",
        choices: LENGTHS,
        examples: &[],
    },
    Spec {
        id: "title",
        ask: "What is the game called?",
        why: "Title on screen, in the README and the folder name (up to 60 characters).",
        choices: &[],
        examples: &[],
    },
    Spec {
        id: "extra",
        ask: "Anything else it must have that the questions above did not cover?",
        why: "The templates do not build this by themselves: it is recorded in interview.json and the handoff notes as work for the agent after the game is made.",
        choices: &[],
        examples: &["a double jump", "music", "a boss at the end"],
    },
];

/// Every question id, in asking order.
pub fn ids() -> Vec<&'static str> {
    SPECS.iter().map(|s| s.id).collect()
}

/// The interview for `prompt`: every question that applies (the view question only for a
/// maze), each with its guess given the answers so far, plus notes on what the starter cannot
/// do for this prompt. Fails only on bad answers.
pub fn questions(
    prompt_text: &str,
    answers: &Answers,
    template: Option<Template>,
    view: Option<View>,
) -> Result<(Vec<Question>, Vec<String>), Errors> {
    let brief = brief_from_answers(prompt_text, answers, template, view)?;
    let mut out = Vec::new();
    for spec in SPECS {
        // A forced template or view is already decided: no need to ask.
        if (spec.id == "genre" && template.is_some())
            || (spec.id == "view" && (view.is_some() || brief.template != Template::Maze))
            || (matches!(spec.id, "pickup_count" | "enemy_count") && brief.template != Template::Maze)
        {
            continue;
        }
        let guess = guess_of(spec.id, prompt_text, answers, &brief);
        out.push(Question {
            id: spec.id,
            ask: spec.ask,
            why: spec.why,
            choices: spec.choices.to_vec(),
            examples: if spec.id == "setting" {
                prompt::settings()
            } else {
                spec.examples.to_vec()
            },
            guess,
            answer: answers.get(spec.id).cloned(),
        });
    }
    let mut notes = Vec::new();
    let other = prompt::other_genres(prompt_text);
    if !other.is_empty() {
        notes.push(format!(
            "The idea mentions {}; the starter has maze and platformer templates only. Ask which \
             is closer, then build the rest as game code after making it (skills \
             marionette-write-script, marionette-add-rust-code).",
            other.join(", ")
        ));
    }
    Ok((out, notes))
}

/// What a question's value is in `brief` (the brief already folds in the answers).
fn guess_of(id: &str, prompt_text: &str, answers: &Answers, brief: &Brief) -> String {
    let slot = |name: &str| {
        brief
            .assets
            .get(name)
            .map(|s| s.describe.clone())
            .unwrap_or_default()
    };
    match id {
        "genre" => brief.template.name().to_string(),
        "view" => match brief.view {
            View::TwoD => "2d".into(),
            View::ThreeD => "3d".into(),
        },
        "setting" => answers
            .get("setting")
            .map(String::as_str)
            .or_else(|| prompt::setting_of(prompt_text))
            .unwrap_or("")
            .to_string(),
        "player" | "enemy" | "goal" => slot(id),
        "pickups" => brief.words.pickups.clone(),
        "pickup_count" => brief
            .pickup_count
            .map_or_else(|| level::default_pickups(brief.size), |n| n as usize)
            .to_string(),
        "enemy_count" => brief
            .enemy_count
            .map_or_else(
                || level::default_enemies(brief.size, brief.difficulty),
                |n| n as usize,
            )
            .to_string(),
        "style" => brief.style.clone(),
        "difficulty" => match brief.difficulty {
            Difficulty::Easy => "easy",
            Difficulty::Normal => "normal",
            Difficulty::Hard => "hard",
        }
        .into(),
        "length" => match brief.size {
            Size::Small => "small",
            Size::Medium => "medium",
            Size::Large => "large",
        }
        .into(),
        "title" => brief.title.clone(),
        _ => String::new(),
    }
}

fn bad(msg: String, fix: String) -> Diagnostic {
    Diagnostic::error(codes::BAD_ANSWER, msg).fix(fix)
}

/// Checks every answer: known id, and one of the choices where the question has them. Returns
/// every problem at once.
pub fn check(answers: &Answers) -> Result<(), Errors> {
    let mut problems = Vec::new();
    for (id, value) in answers {
        let Some(spec) = SPECS.iter().find(|s| s.id == id) else {
            problems.push(bad(
                format!("`{id}` is not an interview question"),
                format!("questions: {}", ids().join(", ")),
            ));
            continue;
        };
        let v = value.trim();
        if !spec.choices.is_empty() && !spec.choices.iter().any(|c| c.value.eq_ignore_ascii_case(v)) {
            let values: Vec<&str> = spec.choices.iter().map(|c| c.value).collect();
            problems.push(bad(
                format!("`{id}` is `{v}`, not one of {}", values.join(", ")),
                if id == "genre" {
                    "pick the closer template; build the rest as game code after making it".into()
                } else {
                    format!("answer one of: {}", values.join(", "))
                },
            ));
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(Errors(problems))
    }
}

/// The answer to `id`, trimmed, or `None` when missing or blank.
fn answer<'a>(answers: &'a Answers, id: &str) -> Option<&'a str> {
    answers.get(id).map(|s| s.trim()).filter(|s| !s.is_empty())
}

/// A brief from the prompt plus the answers. The setting joins the prompt so the keyword
/// reader picks its theme; every other answer replaces the matching field. A template or view
/// passed by the caller wins over the answers. The brief is not checked here (`Brief::check`).
pub fn brief_from_answers(
    prompt_text: &str,
    answers: &Answers,
    template: Option<Template>,
    view: Option<View>,
) -> Result<Brief, Errors> {
    check(answers)?;
    let lower = |id: &str| answer(answers, id).map(str::to_ascii_lowercase);
    let template = template.or(match lower("genre").as_deref() {
        Some("platformer") => Some(Template::Platformer),
        Some("maze") => Some(Template::Maze),
        _ => None,
    });
    let view = view.or(match lower("view").as_deref() {
        Some("3d") => Some(View::ThreeD),
        Some("2d") => Some(View::TwoD),
        _ => None,
    });
    let full_prompt = match answer(answers, "setting") {
        Some(s) => format!("{prompt_text} {s}"),
        None => prompt_text.to_string(),
    };
    let mut b = prompt::brief_from_prompt(&full_prompt, template, view);
    // The seed follows the person's own words, so answering questions does not move it.
    b.seed = prompt::brief_from_prompt(prompt_text, template, view).seed;

    let mut set_slot = |name: &str, text: String| {
        if let Some(s) = b.assets.get_mut(name) {
            s.describe = text;
        }
    };
    for id in ["player", "enemy", "goal"] {
        if let Some(v) = answer(answers, id) {
            set_slot(id, v.to_string());
        }
    }
    if let Some(p) = answer(answers, "pickups") {
        set_slot("pickup", format!("a {}", prompt::singular(p)));
        // The HUD word: the whole phrase when it fits, else its last word.
        let word = if p.chars().count() <= 20 {
            p.to_string()
        } else {
            p.rsplit(' ').next().unwrap_or(p).chars().take(20).collect()
        };
        b.words.pickups = word;
    }
    if let Some(s) = answer(answers, "style") {
        b.style = s.to_string();
    }
    match lower("difficulty").as_deref() {
        Some("easy") => b.difficulty = Difficulty::Easy,
        Some("hard") => b.difficulty = Difficulty::Hard,
        Some("normal") => b.difficulty = Difficulty::Normal,
        _ => {}
    }
    match lower("length").as_deref() {
        Some("small") => b.size = Size::Small,
        Some("large") => b.size = Size::Large,
        Some("medium") => b.size = Size::Medium,
        _ => {}
    }
    if b.template == Template::Maze {
        // A count answer wins; else a number the prompt gave ("3 gems") is the guess.
        let count = |id: &str| -> Result<Option<u32>, Errors> {
            match answer(answers, id) {
                Some(v) => v.trim().parse::<u32>().map(Some).map_err(|_| {
                    Errors(vec![bad(
                        format!("answer `{id}` must be a whole number, got `{v}`"),
                        "write it like 3".into(),
                    )])
                }),
                None => Ok(None),
            }
        };
        b.pickup_count = count("pickup_count")?.or_else(|| prompt::count_of(prompt_text, &b.words.pickups));
        b.enemy_count = count("enemy_count")?;
    }
    let named = |id: &str| answer(answers, id).is_some();
    if named("player") || named("pickups") || named("enemy") || named("goal") {
        b.pitch = pitch(&b, prompt_text);
    }
    match answer(answers, "title") {
        Some(t) => b.title = t.to_string(),
        None if named("player") || named("pickups") => {
            let hero = b
                .assets
                .get("player")
                .map_or("hero", |s| head_noun(&s.describe))
                .to_string();
            let last = b.words.pickups.rsplit(' ').next().unwrap_or("gems").to_string();
            b.title = prompt::title_case(&format!("{hero} and the {last}"))
                .chars()
                .take(60)
                .collect();
        }
        None => {}
    }
    Ok(b)
}

/// The noun a description is about: the last word before a phrase that adds detail, e.g.
/// "a penguin in a red scarf" gives "penguin".
fn head_noun(describe: &str) -> &str {
    let cut = [" in ", " with ", " who ", " that ", " wearing ", " made of "]
        .iter()
        .filter_map(|p| describe.find(p))
        .min()
        .unwrap_or(describe.len());
    describe[..cut]
        .rsplit(' ')
        .find(|w| !w.is_empty())
        .unwrap_or("hero")
}

/// One sentence from the answered slots.
fn pitch(b: &Brief, prompt_text: &str) -> String {
    let d = |name: &str| {
        b.assets
            .get(name)
            .map(|s| s.describe.as_str())
            .unwrap_or("")
            .to_string()
    };
    let verb = match b.template {
        Template::Maze => "collect all the",
        Template::Platformer => "run, jump and grab the",
    };
    format!(
        "Play as {}: {verb} {}, avoid {} and reach {}. From the prompt: \"{}\"",
        d("player"),
        b.words.pickups,
        d("enemy"),
        d("goal"),
        prompt_text.trim()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answers(pairs: &[(&str, &str)]) -> Answers {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn unanswered_questions_carry_the_keyword_guess() {
        let (qs, notes) = questions(
            "a cozy 3d game where you play as a moth who collects paper lanterns",
            &Answers::new(),
            None,
            None,
        )
        .unwrap();
        let q = |id: &str| qs.iter().find(|q| q.id == id).unwrap();
        assert_eq!(q("genre").guess, "maze");
        assert_eq!(q("view").guess, "3d");
        assert_eq!(q("player").guess, "a moth");
        assert_eq!(q("pickups").guess, "lanterns");
        assert_eq!(q("setting").guess, "forest");
        assert!(q("genre").answer.is_none());
        assert!(notes.is_empty());
    }

    #[test]
    fn answers_shape_the_brief() {
        let a = answers(&[
            ("genre", "Platformer"),
            ("setting", "snow"),
            ("player", "a penguin in a red scarf"),
            ("pickups", "fish"),
            ("difficulty", "hard"),
            ("extra", "a double jump"),
        ]);
        let b = brief_from_answers("a game about a bird", &a, None, None).unwrap();
        assert_eq!(b.template, Template::Platformer);
        assert_eq!(b.difficulty, Difficulty::Hard);
        assert_eq!(b.assets["player"].describe, "a penguin in a red scarf");
        assert_eq!(b.assets["pickup"].describe, "a fish");
        assert_eq!(b.words.pickups, "fish");
        assert_eq!(b.colors.background, "#0e1a2a", "the snow theme");
        assert_eq!(b.title, "Penguin And The Fish");
        assert_eq!(
            b.seed,
            prompt::brief_from_prompt("a game about a bird", None, None).seed
        );
        b.check().unwrap();
        // A platformer has no view question.
        let (qs, _) = questions("a game about a bird", &a, None, None).unwrap();
        assert!(qs.iter().all(|q| q.id != "view"));
        assert_eq!(
            qs.iter().find(|q| q.id == "genre").unwrap().answer.as_deref(),
            Some("Platformer")
        );
    }

    #[test]
    fn bad_answers_are_listed_together() {
        let e = check(&answers(&[
            ("genre", "racing"),
            ("colour", "red"),
            ("length", "huge"),
        ]))
        .unwrap_err();
        assert_eq!(e.0.len(), 3);
        assert!(e.0.iter().all(|d| d.code == codes::BAD_ANSWER));
    }

    #[test]
    fn other_genres_get_a_note_and_forced_choices_are_not_asked() {
        let (qs, notes) = questions(
            "a racing game with cars",
            &Answers::new(),
            Some(Template::Maze),
            Some(View::TwoD),
        )
        .unwrap();
        assert_eq!(notes.len(), 1);
        assert!(qs.iter().all(|q| q.id != "genre" && q.id != "view"));
    }
}
