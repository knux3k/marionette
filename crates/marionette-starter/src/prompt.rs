//! A brief from a one-line prompt without a language model: keywords pick the template, the
//! view, the difficulty and a theme; simple phrases ("play as a moth", "collect lanterns",
//! "avoid bats", "reach the moon gate") fill the asset slots. An agent writes better briefs;
//! this keeps `marionette make "<idea>"` working with nothing else installed.

use crate::{Brief, Colors, Difficulty, Size, SlotBrief, Template, View, Words};
use std::collections::BTreeMap;

struct Theme {
    keys: &'static [&'static str],
    style: &'static str,
    background: &'static str,
    accent: &'static str,
    /// player, enemy, pickup (singular), pickups (plural), goal, wall, floor/ground, backdrop
    slots: [&'static str; 8],
}

const THEMES: &[Theme] = &[
    Theme {
        keys: &[
            "space",
            "star",
            "planet",
            "alien",
            "rocket",
            "galaxy",
            "astronaut",
        ],
        style: "bright sci-fi cartoon, clean shapes, glowing accents",
        background: "#0b0d1f",
        accent: "#7ee8ff",
        slots: [
            "a small round astronaut",
            "a spiky purple alien blob",
            "a glowing star crystal",
            "star crystals",
            "a swirling warp portal",
            "a riveted metal hull panel",
            "a dark metal deck plate",
            "a starry nebula over a cratered moon",
        ],
    },
    Theme {
        keys: &["ocean", "sea", "underwater", "fish", "reef", "mermaid", "diver"],
        style: "soft underwater cartoon, teal light, bubbles",
        background: "#062a3a",
        accent: "#ffe07a",
        slots: [
            "a little orange fish",
            "a grumpy spiky pufferfish",
            "a shining pearl",
            "pearls",
            "an ancient coral archway",
            "a block of pink coral",
            "pale sea sand with ripples",
            "a sunlit coral reef seen from the side",
        ],
    },
    Theme {
        keys: &[
            "forest", "garden", "woods", "tree", "moth", "fairy", "mushroom", "leaf",
        ],
        style: "cozy hand-painted storybook, warm soft light",
        background: "#10201a",
        accent: "#ffd166",
        slots: [
            "a small moth with glowing wings",
            "a hungry night bat",
            "a paper lantern",
            "lanterns",
            "a mossy stone archway",
            "a dense green hedge",
            "a mossy forest floor",
            "a misty forest at dusk",
        ],
    },
    Theme {
        keys: &[
            "dungeon", "castle", "knight", "dragon", "skeleton", "crypt", "wizard",
        ],
        style: "chunky fantasy cartoon, torchlight",
        background: "#120e14",
        accent: "#ffb347",
        slots: [
            "a little knight in a too-big helmet",
            "a rattling skeleton",
            "a gold coin",
            "coins",
            "an iron-bound dungeon door",
            "a grey stone brick wall",
            "worn dungeon flagstones",
            "a torch-lit castle hall",
        ],
    },
    Theme {
        keys: &["candy", "sweet", "cake", "sugar", "chocolate", "dessert"],
        style: "glossy candy cartoon, pastel colours",
        background: "#2a1030",
        accent: "#ff8fd8",
        slots: [
            "a cheerful gumdrop",
            "a sour lemon drop with angry eyes",
            "a wrapped candy",
            "candies",
            "a gingerbread door",
            "a pink frosted wafer block",
            "a chocolate tile floor",
            "candy hills under a pink sky",
        ],
    },
    Theme {
        keys: &["desert", "sand", "pyramid", "cactus", "tomb", "egypt"],
        style: "warm sunlit cartoon, bold shadows",
        background: "#2a1a0c",
        accent: "#ffd27a",
        slots: [
            "a small desert explorer with a scarf",
            "a scuttling scorpion",
            "a golden scarab",
            "scarabs",
            "a carved sandstone gate",
            "a sandstone block wall",
            "rippled desert sand",
            "dunes and a distant pyramid at sunset",
        ],
    },
    Theme {
        keys: &["snow", "ice", "winter", "penguin", "frozen", "arctic"],
        style: "crisp winter cartoon, cool blue light",
        background: "#0e1a2a",
        accent: "#bff4ff",
        slots: [
            "a little penguin in a scarf",
            "a grumpy snowman",
            "a snowflake crystal",
            "snowflakes",
            "an ice cave entrance",
            "a block of blue ice",
            "packed snow",
            "snowy mountains under northern lights",
        ],
    },
    Theme {
        keys: &["city", "neon", "cyber", "robot", "street", "future"],
        style: "neon cyberpunk cartoon, glowing magenta and cyan",
        background: "#0a0716",
        accent: "#ff4fd8",
        slots: [
            "a small courier robot",
            "a patrolling security drone",
            "a glowing data chip",
            "data chips",
            "a neon exit door",
            "a dark metal wall with neon trim",
            "wet dark asphalt",
            "a neon city skyline at night",
        ],
    },
];

const DEFAULT: Theme = Theme {
    keys: &[],
    style: "bright friendly cartoon, clean shapes",
    background: "#141a2e",
    accent: "#ffd166",
    slots: [
        "a small brave explorer",
        "a grumpy wandering monster",
        "a shiny gem",
        "gems",
        "a glowing exit door",
        "a mossy stone wall",
        "a dusty stone floor",
        "rolling hills under a bright sky",
    ],
};

fn words(prompt: &str) -> Vec<String> {
    prompt
        .split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '\''))
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect()
}

fn has(ws: &[String], keys: &[&str]) -> bool {
    ws.iter().any(|w| {
        keys.iter()
            .any(|k| w == k || (w.len() > k.len() && w.starts_with(k) && w.len() <= k.len() + 3))
    })
}

const STOP: &[&str] = &[
    "and", "while", "but", "then", "to", "in", "on", "through", "across", "before", "with", "who", "that",
    "where", "from", "into", "at", "of", "for", "or", "so", "as",
];

/// The noun phrase after one of `cues`, up to a stop word (at most four words), e.g.
/// "collect glowing lanterns and avoid bats" with cue "collect" gives "glowing lanterns".
fn phrase_after(ws: &[String], cues: &[&str]) -> Option<String> {
    for (i, w) in ws.iter().enumerate() {
        if cues.contains(&w.as_str()) {
            let mut out = Vec::new();
            for x in &ws[i + 1..] {
                if (STOP.contains(&x.as_str()) && !out.is_empty()) || out.len() == 4 {
                    break;
                }
                if !["a", "an", "the", "all", "every", "some", "of", "to"].contains(&x.as_str()) {
                    out.push(x.clone());
                }
            }
            if !out.is_empty() {
                return Some(out.join(" "));
            }
        }
    }
    None
}

fn singular(plural: &str) -> String {
    let mut words: Vec<&str> = plural.split(' ').collect();
    let last = words.pop().unwrap_or("");
    let one = if let Some(s) = last.strip_suffix("ies") {
        format!("{s}y")
    } else if last.ends_with("ses")
        || last.ends_with("xes")
        || last.ends_with("ches")
        || last.ends_with("shes")
    {
        last[..last.len() - 2].to_string()
    } else if let Some(s) = last.strip_suffix('s').filter(|s| !s.ends_with('s')) {
        s.to_string()
    } else {
        last.to_string()
    };
    words.push(&one);
    words.join(" ")
}

fn title_case(s: &str) -> String {
    s.split(' ')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next().map_or(String::new(), |f| {
                f.to_uppercase().collect::<String>() + c.as_str()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// FNV-1a over the prompt: the same prompt gives the same seed.
fn seed_of(prompt: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in prompt.trim().to_lowercase().bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h % 10_000 + 1
}

/// A brief for `prompt`. Template, view, difficulty and size may be forced.
pub fn brief_from_prompt(prompt: &str, template: Option<Template>, view: Option<View>) -> Brief {
    let ws = words(prompt);
    let template = template.unwrap_or(
        if has(
            &ws,
            &[
                "platform",
                "platformer",
                "jump",
                "side-scroller",
                "sidescroller",
                "scroller",
                "runner",
                "side",
            ],
        ) {
            Template::Platformer
        } else {
            Template::Maze
        },
    );
    let view = match template {
        Template::Platformer => View::TwoD,
        Template::Maze => view.unwrap_or(if has(&ws, &["3d", "3-d", "three-d", "isometric"]) {
            View::ThreeD
        } else {
            View::TwoD
        }),
    };
    let difficulty = if has(&ws, &["easy", "relaxing", "cozy", "chill", "kids", "gentle"]) {
        Difficulty::Easy
    } else if has(&ws, &["hard", "difficult", "brutal", "challenging", "tough"]) {
        Difficulty::Hard
    } else {
        Difficulty::Normal
    };
    let size = if has(&ws, &["tiny", "short", "small", "quick"]) {
        Size::Small
    } else if has(&ws, &["big", "huge", "long", "large", "epic"]) {
        Size::Large
    } else {
        Size::Medium
    };
    let theme = THEMES.iter().find(|t| has(&ws, t.keys)).unwrap_or(&DEFAULT);

    let player = phrase_after(&ws, &["as", "are", "playing"]).map(|p| format!("a {p}"));
    let pickups = phrase_after(
        &ws,
        &[
            "collect",
            "collects",
            "collecting",
            "gather",
            "gathers",
            "find",
            "finds",
            "grab",
            "grabs",
            "save",
            "saves",
            "rescue",
            "rescues",
            "light",
            "lights",
        ],
    );
    let enemy = phrase_after(
        &ws,
        &[
            "avoid", "avoids", "avoiding", "dodge", "dodges", "escape", "escapes", "evade", "evades", "past",
            "fleeing", "chased", "by",
        ],
    )
    .map(|e| format!("a {}", singular(&e)));
    let goal = phrase_after(
        &ws,
        &[
            "reach",
            "reaches",
            "exit",
            "escape-to",
            "home",
            "toward",
            "towards",
        ],
    )
    .map(|g| format!("a {g}"));

    let pickups_plural = pickups.clone().unwrap_or_else(|| theme.slots[3].to_string());
    let pickup_one = pickups
        .as_deref()
        .map(|p| format!("a {}", singular(p)))
        .unwrap_or_else(|| theme.slots[2].to_string());
    let style = [
        ("pixel", "crisp pixel art"),
        ("watercolor", "soft watercolor"),
        ("painted", "hand-painted"),
        ("low-poly", "stylised low-poly"),
        ("cute", "cute rounded cartoon"),
        ("spooky", "spooky moonlit cartoon"),
        ("neon", "glowing neon"),
    ]
    .iter()
    .filter(|(k, _)| has(&ws, &[k]))
    .map(|(_, v)| *v)
    .next()
    .map_or(theme.style.to_string(), |s| format!("{s}, {}", theme.style));

    let mut assets = BTreeMap::new();
    let mut put = |slot: &str, describe: String| {
        assets.insert(
            slot.to_string(),
            SlotBrief {
                describe,
                use_asset: None,
                search: None,
            },
        );
    };
    put("player", player.clone().unwrap_or_else(|| theme.slots[0].into()));
    put("enemy", enemy.unwrap_or_else(|| theme.slots[1].into()));
    put("pickup", pickup_one);
    put("goal", goal.unwrap_or_else(|| theme.slots[4].into()));
    match (template, view) {
        (Template::Maze, View::TwoD) => {
            put("wall", theme.slots[5].into());
            put("floor", theme.slots[6].into());
        }
        (Template::Maze, View::ThreeD) => put(
            "wall",
            format!("a square block of {}", theme.slots[5].trim_start_matches("a ")),
        ),
        (Template::Platformer, _) => {
            put("ground", theme.slots[6].into());
            put("spikes", "a row of sharp spikes".into());
            put("background", theme.slots[7].into());
        }
    }

    let hero = player
        .as_deref()
        .map(|p| {
            p.trim_start_matches("a ")
                .rsplit(' ')
                .next()
                .unwrap_or("hero")
                .to_string()
        })
        .unwrap_or_else(|| theme.slots[0].rsplit(' ').next().unwrap_or("hero").to_string());
    let last = pickups_plural.rsplit(' ').next().unwrap_or("gems").to_string();
    let title = title_case(&format!("{hero} and the {last}"));
    let verb = if template == Template::Maze {
        "Collect all the"
    } else {
        "Run, jump and grab the"
    };
    Brief {
        format: crate::BRIEF_FORMAT.into(),
        title: title.chars().take(60).collect(),
        pitch: format!(
            "{verb} {last}, then reach the goal. From the prompt: \"{}\"",
            prompt.trim()
        ),
        template,
        view,
        style,
        difficulty,
        size,
        seed: seed_of(prompt),
        words: Words {
            pickups: last.chars().take(20).collect(),
            ..Words::default()
        },
        colors: Colors {
            background: theme.background.into(),
            accent: theme.accent.into(),
        },
        assets,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_main_phrases() {
        let b = brief_from_prompt(
            "a cozy 3d game where you play as a moth who collects paper lanterns and avoids bats",
            None,
            None,
        );
        assert_eq!(b.template, Template::Maze);
        assert_eq!(b.view, View::ThreeD);
        assert_eq!(b.difficulty, Difficulty::Easy);
        assert_eq!(b.assets["player"].describe, "a moth");
        assert_eq!(b.assets["pickup"].describe, "a paper lantern");
        assert_eq!(b.assets["enemy"].describe, "a bat");
        assert_eq!(b.words.pickups, "lanterns");
        assert_eq!(b.title, "Moth And The Lanterns");
        b.check().unwrap();
    }

    #[test]
    fn platformer_keywords_and_themes() {
        let b = brief_from_prompt("a hard space platformer, jump over alien slime", None, None);
        assert_eq!(b.template, Template::Platformer);
        assert_eq!(b.view, View::TwoD);
        assert_eq!(b.difficulty, Difficulty::Hard);
        assert_eq!(b.colors.background, "#0b0d1f");
        assert!(b.assets.contains_key("background"));
        b.check().unwrap();
        assert_eq!(
            b.seed,
            brief_from_prompt("a hard space platformer, jump over alien slime", None, None).seed
        );
    }

    #[test]
    fn plain_prompt_still_makes_a_valid_brief() {
        let b = brief_from_prompt("something fun", None, None);
        b.check().unwrap();
        assert_eq!(singular("berries"), "berry");
        assert_eq!(singular("glowing boxes"), "glowing box");
    }
}
