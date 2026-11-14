//! Genre templates: asset slots, generation recipes, and the project files of a game.

use crate::{Art, ArtMap, Brief, Difficulty, Template, View, level, parse_hex};
use serde_json::{Value, json};

pub const MAZE_SCRIPT: &str = include_str!("../templates/maze.ts");
pub const PLATFORMER_SCRIPT: &str = include_str!("../templates/platformer.ts");

pub const CONFIG_BEGIN: &str = "// BEGIN GAME CONFIG";
pub const CONFIG_END: &str = "// END GAME CONFIG";

pub const SCENE: &str = "scenes/main.scene.json";
pub const SCRIPT: &str = "scenes/scripts/game.ts";
/// Scene id of the entity that runs the game script (tests read its `ScriptState`).
pub const GAME_ENTITY: u64 = 3;

/// What a slot's asset is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SlotKind {
    /// A sprite with a transparent background (2D), seen as `facing` says.
    Sprite { facing: &'static str },
    /// A seamless texture tile (2D walls, floors, ground).
    Tile { seen: &'static str },
    /// A wide background picture.
    Backdrop,
    /// A 3D model this tall in meters, with this triangle budget.
    Model { height_m: f64, max_triangles: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Slot {
    pub name: &'static str,
    pub kind: SlotKind,
    /// Used when the brief does not describe the slot.
    pub default_describe: &'static str,
    /// Placeholder color (linear RGB) until an asset exists.
    pub placeholder: [f32; 3],
}

const TOP: SlotKind = SlotKind::Sprite {
    facing: "seen from directly above",
};
const SIDE: SlotKind = SlotKind::Sprite {
    facing: "side view facing right",
};

/// The asset slots of a template in a view, in a stable order.
pub fn slots(template: Template, view: View) -> Vec<Slot> {
    let s = |name, kind, default_describe, placeholder| Slot {
        name,
        kind,
        default_describe,
        placeholder,
    };
    let model = |h, t| SlotKind::Model {
        height_m: h,
        max_triangles: t,
    };
    match (template, view) {
        (Template::Maze, View::TwoD) => vec![
            s("player", TOP, "a small brave explorer", [0.3, 0.8, 1.0]),
            s("enemy", TOP, "a grumpy wandering ghost", [1.0, 0.3, 0.35]),
            s("pickup", TOP, "a shiny gem", [1.0, 0.8, 0.3]),
            s("goal", TOP, "a glowing exit door", [0.9, 0.75, 0.3]),
            s(
                "wall",
                SlotKind::Tile {
                    seen: "from directly above",
                },
                "a mossy stone wall",
                [0.35, 0.4, 0.45],
            ),
            s(
                "floor",
                SlotKind::Tile {
                    seen: "from directly above",
                },
                "a dusty stone floor",
                [0.12, 0.13, 0.16],
            ),
        ],
        (Template::Maze, View::ThreeD) => vec![
            s(
                "player",
                model(0.9, 8000),
                "a small brave explorer",
                [0.3, 0.8, 1.0],
            ),
            s(
                "enemy",
                model(0.9, 8000),
                "a grumpy wandering ghost",
                [1.0, 0.3, 0.35],
            ),
            s("pickup", model(0.55, 4000), "a shiny gem", [1.0, 0.8, 0.3]),
            s("goal", model(1.4, 8000), "a glowing exit door", [0.9, 0.75, 0.3]),
            s(
                "wall",
                model(1.0, 2000),
                "a square mossy stone wall block",
                [0.35, 0.4, 0.45],
            ),
        ],
        (Template::Platformer, _) => vec![
            s("player", SIDE, "a small brave explorer", [0.3, 0.8, 1.0]),
            s("enemy", SIDE, "a grumpy walking beetle", [1.0, 0.3, 0.35]),
            s("pickup", SIDE, "a shiny coin", [1.0, 0.8, 0.3]),
            s("goal", SIDE, "a tall checkered goal flag", [0.9, 0.75, 0.3]),
            s("spikes", SIDE, "a row of sharp metal spikes", [0.85, 0.85, 0.9]),
            s(
                "ground",
                SlotKind::Tile {
                    seen: "from the front",
                },
                "grassy earth",
                [0.45, 0.33, 0.22],
            ),
            s(
                "background",
                SlotKind::Backdrop,
                "rolling hills under a bright sky",
                [0.0, 0.0, 0.0],
            ),
        ],
    }
}

/// The description used for a slot: the brief's, or the template default.
pub fn describe<'a>(brief: &'a Brief, slot: &Slot) -> &'a str {
    match brief.assets.get(slot.name) {
        Some(b) if !b.describe.trim().is_empty() => b.describe.trim(),
        _ => slot.default_describe,
    }
}

/// Generation recipes for one slot (`marionette.recipe/1` documents), in build order: a
/// model slot needs its reference image first.
pub fn recipes(brief: &Brief, slot: &Slot) -> Vec<(String, Value)> {
    let what = describe(brief, slot);
    let style = if brief.style.trim().is_empty() {
        String::new()
    } else {
        format!(", {}", brief.style.trim())
    };
    let note = format!(
        "Starter slot `{}` of \"{}\" (game_make, decision 0150).",
        slot.name, brief.title
    );
    match slot.kind {
        SlotKind::Sprite { facing } => vec![(
            slot.name.to_string(),
            json!({
                "format": "marionette.recipe/1", "name": slot.name, "output": "texture",
                "prompt": format!("{what}{style}, single game sprite {facing}, centred, flat plain magenta background (#ff00ff), crisp edges, bold readable silhouette, no text"),
                "texture": {"resize": [256, 256], "remove_background": true},
                "notes": note,
            }),
        )],
        SlotKind::Tile { seen } => vec![(
            slot.name.to_string(),
            json!({
                "format": "marionette.recipe/1", "name": slot.name, "output": "texture",
                "prompt": format!("{what}{style}, seamless tileable game texture seen {seen}, even flat light, no text"),
                "texture": {"resize": [256, 256], "tileable": true},
                "notes": note,
            }),
        )],
        SlotKind::Backdrop => vec![(
            slot.name.to_string(),
            json!({
                "format": "marionette.recipe/1", "name": slot.name, "output": "texture",
                "prompt": format!("{what}{style}, wide side-scrolling game background landscape, no characters, no text"),
                "texture": {"resize": [1024, 512]},
                "notes": note,
            }),
        )],
        SlotKind::Model {
            height_m,
            max_triangles,
        } => {
            let reference = format!("{}_ref", slot.name);
            vec![
                (
                    reference.clone(),
                    json!({
                        "format": "marionette.recipe/1", "name": reference, "output": "image",
                        "prompt": format!("{what}{style}, single object, three-quarter front view, centred, whole object in frame, plain light grey background, soft even studio light, stylised low-poly game asset, clean shapes, no text"),
                        "notes": note,
                    }),
                ),
                (
                    slot.name.to_string(),
                    json!({
                        "format": "marionette.recipe/1", "name": slot.name, "output": "model", "from": reference,
                        "model": {"height_m": height_m, "max_triangles": max_triangles, "max_texture_px": 512},
                        "notes": note,
                    }),
                ),
            ]
        }
    }
}

/// The generated level for a brief and seed.
pub fn level(brief: &Brief, seed: u64) -> Vec<String> {
    match brief.template {
        Template::Maze => level::maze(seed, brief.size, brief.difficulty),
        Template::Platformer => level::platformer(seed, brief.size, brief.difficulty),
    }
}

fn rgb(c: [f32; 3]) -> Value {
    json!([round3(c[0]), round3(c[1]), round3(c[2])])
}

fn round3(v: f32) -> f64 {
    (v as f64 * 1000.0).round() / 1000.0
}

fn rgba(c: [f32; 3]) -> Value {
    json!({"r": round3(c[0]), "g": round3(c[1]), "b": round3(c[2]), "a": 1.0})
}

fn colors(brief: &Brief) -> ([f32; 3], [f32; 3]) {
    let bg = parse_hex(&brief.colors.background).unwrap_or([0.007, 0.01, 0.027]);
    let accent = parse_hex(&brief.colors.accent).unwrap_or([1.0, 0.63, 0.13]);
    (bg, accent)
}

/// The CONFIG object the template script reads.
pub fn config(brief: &Brief, art: &ArtMap, map: &[String]) -> Value {
    let (bg, accent) = colors(brief);
    let mut arts = serde_json::Map::new();
    for slot in slots(brief.template, brief.view) {
        let a = art.get(slot.name).cloned().unwrap_or(Art::Shape {
            color: slot.placeholder.map(f64::from),
        });
        // A backdrop placeholder is no backdrop at all: the clear color shows.
        if slot.kind == SlotKind::Backdrop && matches!(a, Art::Shape { .. }) {
            continue;
        }
        let k = brief.assets.get(slot.name).and_then(|b| b.scale).unwrap_or(1.0);
        let a = match a {
            Art::Shape { color } => Art::Shape {
                color: color.map(|c| (c * 1000.0).round() / 1000.0),
            },
            Art::Model { asset, scale } => Art::Model {
                asset,
                scale: ((scale * k as f64) * 1000.0).round() / 1000.0,
            },
            other => other,
        };
        let mut v = serde_json::to_value(&a).expect("art serializes");
        if k != 1.0 && !matches!(a, Art::Model { .. }) {
            v["scale"] = json!(round3(k));
        }
        arts.insert(slot.name.into(), v);
    }
    let words = json!({
        "pickups": brief.words.pickups.to_uppercase(),
        "win": brief.words.win,
        "lose": brief.words.lose,
    });
    let common = json!({
        "title": brief.title,
        "map": map,
        "words": words,
        "colors": {"background": rgb(bg), "accent": rgb(accent)},
        "art": Value::Object(arts),
    });
    let mut out = common.as_object().cloned().expect("object");
    let extra = match brief.template {
        Template::Maze => {
            let (lives, enemy_step) = match brief.difficulty {
                Difficulty::Easy => (5, 14),
                Difficulty::Normal => (3, 12),
                Difficulty::Hard => (3, 10),
            };
            json!({
                "view": if brief.view == View::ThreeD { "3d" } else { "2d" },
                "lives": lives, "playerStep": 8, "enemyStep": enemy_step, "enemyDelay": 1.5,
            })
        }
        Template::Platformer => {
            let (lives, enemy_speed) = match brief.difficulty {
                Difficulty::Easy => (5, 1.5),
                Difficulty::Normal => (3, 2.0),
                Difficulty::Hard => (3, 2.5),
            };
            let cols = map.first().map_or(0, |r| r.len());
            json!({
                "lives": lives, "timeLimit": (cols as f64 * 0.6 + 20.0).round(),
                "runSpeed": 6.5, "jumpSpeed": 13.0, "gravity": 32.0, "enemySpeed": enemy_speed,
            })
        }
    };
    out.extend(extra.as_object().cloned().expect("object"));
    Value::Object(out)
}

/// The script's config block (between the markers, inclusive).
pub fn config_block(config: &Value) -> String {
    // One map row per line keeps the level readable and diffs small.
    let mut text = serde_json::to_string_pretty(config).expect("config serializes");
    text = compact_arrays(&text);
    format!("{CONFIG_BEGIN}\nconst CONFIG: any = {text};\n{CONFIG_END}")
}

/// Puts short arrays of numbers (colors, regions) on one line.
fn compact_arrays(pretty: &str) -> String {
    let mut out = String::new();
    let mut lines = pretty.lines().peekable();
    while let Some(line) = lines.next() {
        if line.trim_end().ends_with('[') {
            let mut items = Vec::new();
            let mut tail = String::new();
            let mut ok = true;
            let mut look = lines.clone();
            for l in look.by_ref() {
                let t = l.trim();
                if t.starts_with(']') {
                    tail = t.to_string();
                    break;
                }
                let v = t.trim_end_matches(',');
                if v.parse::<f64>().is_err() {
                    ok = false;
                    break;
                }
                items.push(v.to_string());
            }
            if ok && !tail.is_empty() && items.len() <= 4 {
                for _ in 0..=items.len() {
                    lines.next();
                }
                out.push_str(line.trim_end());
                out.push_str(&items.join(", "));
                out.push_str(&tail);
                out.push('\n');
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    out.trim_end().to_string()
}

/// The template script with its config block filled in.
pub fn script(brief: &Brief, config: &Value) -> String {
    let source = match brief.template {
        Template::Maze => MAZE_SCRIPT,
        Template::Platformer => PLATFORMER_SCRIPT,
    };
    replace_config(source, &config_block(config)).expect("templates carry the config markers")
}

/// Replaces the config block of a script, keeping everything else. `None` when the markers
/// are missing.
pub fn replace_config(source: &str, block: &str) -> Option<String> {
    let a = source.find(CONFIG_BEGIN)?;
    let b = source[a..].find(CONFIG_END)? + a + CONFIG_END.len();
    Some(format!("{}{}{}", &source[..a], block, &source[b..]))
}

fn binding(input: &str, scale: f64) -> Value {
    json!({"input": input, "scale": scale})
}

/// The input map (`scenes/main.actions.json`).
pub fn actions(template: Template) -> Value {
    let mut actions = serde_json::Map::new();
    actions.insert(
        "move_x".into(),
        json!({"bindings": [binding("key:KeyD", 1.0), binding("key:KeyA", -1.0), binding("key:ArrowRight", 1.0), binding("key:ArrowLeft", -1.0), binding("axis:LeftStickX", 1.0)]}),
    );
    match template {
        Template::Maze => {
            actions.insert(
                "move_y".into(),
                json!({"bindings": [binding("key:KeyW", 1.0), binding("key:KeyS", -1.0), binding("key:ArrowUp", 1.0), binding("key:ArrowDown", -1.0), binding("axis:LeftStickY", 1.0)]}),
            );
        }
        Template::Platformer => {
            actions.insert(
                "jump".into(),
                json!({"bindings": [binding("key:Space", 1.0), binding("key:KeyW", 1.0), binding("key:ArrowUp", 1.0), binding("pad:South", 1.0)]}),
            );
        }
    }
    actions.insert(
        "restart".into(),
        json!({"bindings": [binding("key:KeyR", 1.0), binding("pad:Start", 1.0)]}),
    );
    actions.insert("autoplay".into(), json!({"bindings": [binding("key:F1", 1.0)]}));
    json!({"format": "marionette.input-map", "version": 1, "actions": Value::Object(actions)})
}

fn transform(x: f64, y: f64, z: f64) -> Value {
    json!({"translation": {"x": x, "y": y, "z": z}, "rotation": {"x": 0.0, "y": 0.0, "z": 0.0, "w": 1.0}, "scale": {"x": 1.0, "y": 1.0, "z": 1.0}})
}

fn entity(id: u64, name: &str, components: Value) -> Value {
    json!({"op": "entity_create", "input": {"scene": SCENE, "id": id, "name": name, "components": components}})
}

fn resource(name: &str, value: Value) -> Value {
    json!({"op": "resource_set", "input": {"scene": SCENE, "resource": name, "value": value}})
}

/// The scene recipe (`build.ops.json`, format `marionette.ops`). The level itself is spawned
/// by the script, so the scene holds only the camera, lights, HUD and the game entity.
pub fn build_ops(brief: &Brief, map: &[String]) -> Value {
    let (bg, accent) = colors(brief);
    let (w, h) = (map[0].len() as f64, map.len() as f64);
    let mut ops = vec![
        json!({"op": "scene_create", "input": {"scene": SCENE}}),
        resource("FixedStep", json!({"dt": 0.016666668})),
        resource("InputConfig", json!({"action_map": "main.actions.json"})),
    ];
    match (brief.template, brief.view) {
        (Template::Maze, View::ThreeD) => {
            // Frame the whole maze from above and to the south at 16:9.
            let tan_half_v = (22.5f64).to_radians().tan();
            let tan_half_h = tan_half_v * 16.0 / 9.0;
            let d = ((w / 2.0 + 1.0) / tan_half_h).max((h / 2.0 + 1.5) / tan_half_v) * 1.08;
            let pitch = 66f64.to_radians();
            let (cy, cz) = (
                (d * pitch.sin() * 100.0).round() / 100.0,
                (d * pitch.cos() * 100.0).round() / 100.0,
            );
            let sky = [bg[0] * 0.5 + 0.35, bg[1] * 0.5 + 0.38, bg[2] * 0.5 + 0.45];
            ops.push(resource(
                "AmbientLight",
                json!({"color": rgba(sky), "intensity": 0.2}),
            ));
            ops.push(resource("SkyLight", json!({"sky": rgba(sky), "ground": rgba([bg[0] + 0.05, bg[1] + 0.04, bg[2] + 0.03]), "intensity": 0.6})));
            ops.push(resource(
                "Shadows",
                json!({"distance": (w.max(h) * 0.75).ceil(), "resolution": 2048, "softness": 1.5}),
            ));
            ops.push(resource("ToneMapping", json!({"exposure": 1.0})));
            ops.push(resource("Bloom", json!({"intensity": 0.15, "threshold": 1.0})));
            let mut cam = json!({
                "Camera": {"clear_color": rgba(bg), "projection": {"Perspective": {"far": 200.0, "fov_y_degrees": 45.0, "near": 0.1}}},
                "LookAt": {"target": {"x": 0.0, "y": 0.0, "z": 0.3}, "up": {"x": 0.0, "y": 1.0, "z": 0.0}},
                "Transform": transform(0.0, cy, cz),
            });
            cam["Transform"]["translation"]["z"] = json!(cz);
            ops.push(entity(1, "camera", cam));
            ops.push(entity(
                2,
                "sun",
                json!({"DirectionalLight": {"color": rgba([1.0, 0.95, 0.85]), "direction": {"x": 0.35, "y": -1.0, "z": -0.45}, "illuminance": 1.6}}),
            ));
            let floor = [0, 1, 2].map(|i| bg[i] * 0.5 + accent[i] * 0.12 + 0.07);
            let mut plane = transform(0.0, 0.0, 0.0);
            plane["scale"] = json!({"x": w + 2.0, "y": 1.0, "z": h + 2.0});
            ops.push(entity(
                6,
                "floor",
                json!({"Transform": plane, "Model": {"asset": "primitive:plane"}, "Material": {"base_color": rgba(floor), "metallic": 0.0, "roughness": 0.9}}),
            ));
        }
        (Template::Maze, View::TwoD) => {
            let height = (h + 1.5).max((w + 1.0) * 9.0 / 16.0);
            ops.push(entity(
                1,
                "camera",
                json!({"Camera": {"clear_color": rgba(bg), "projection": {"Orthographic": {"far": 100.0, "height": height, "near": 0.1}}}, "Transform": transform(0.0, 0.0, 10.0)}),
            ));
        }
        (Template::Platformer, _) => {
            ops.push(entity(
                1,
                "camera",
                json!({"Camera": {"clear_color": rgba(bg), "projection": {"Orthographic": {"far": 100.0, "height": 12.5, "near": 0.1}}}, "Transform": transform(11.2, h / 2.0, 10.0)}),
            ));
        }
    }
    ops.push(entity(
        GAME_ENTITY,
        "game",
        json!({"Script": {"path": "scripts/game.ts"}}),
    ));
    ops.push(entity(
        4,
        "hud",
        json!({"UiNode": {"anchor": "TopLeft", "layer": 1, "offset": {"x": 12, "y": 10}, "size": {"x": 700, "y": 18}}, "UiText": {"color": rgba(accent), "size": 18, "text": ""}}),
    ));
    ops.push(entity(
        5,
        "banner",
        json!({"UiNode": {"anchor": "Center", "layer": 1, "offset": {"x": 0, "y": -60}, "size": {"x": 700, "y": 32}}, "UiText": {"color": rgba(accent), "size": 32, "text": ""}}),
    ));
    ops.push(json!({"op": "scene_validate", "input": {"scene": SCENE}}));
    json!({"format": "marionette.ops", "version": 1, "ops": ops})
}

/// Presses F1 (autopilot) on the first tick.
pub fn autoplay_inputs() -> Value {
    json!({"format": "marionette.input-events", "version": 1, "events": [
        {"tick": 0, "input": "key:F1", "value": 1},
        {"tick": 2, "input": "key:F1", "value": 0},
    ]})
}

/// The autopilot test: it wins by `ticks`, with this state hash.
pub fn autoplay_test(ticks: u64, hash: &str) -> Value {
    json!({
        "scene": SCENE, "seed": 1, "ticks": ticks, "input": "tests/autoplay.inputs.json",
        "assertions": [
            {"event": {"kind": "won"}},
            {"field": {"entity": GAME_ENTITY, "path": "ScriptState.data.result", "op": "eq", "value": "won"}},
            {"state_hash": hash},
        ],
    })
}

/// The idle test: nobody touches the controls and the game is not won.
pub fn idle_test(ticks: u64, hash: &str) -> Value {
    json!({
        "scene": SCENE, "seed": 1, "ticks": ticks,
        "assertions": [
            {"event": {"kind": "won", "at_most": 0}},
            {"state_hash": hash},
        ],
    })
}

pub const IDLE_TICKS: u64 = 1200;

/// The project's README.
pub fn readme(brief: &Brief) -> String {
    let controls = match brief.template {
        Template::Maze => "Arrows or WASD move. R restarts after a win or loss. F1 toggles the autopilot.",
        Template::Platformer => {
            "Arrows or A/D run, Space/W/Up jump. R restarts after a win or loss. F1 toggles the autopilot."
        }
    };
    let pitch = if brief.pitch.trim().is_empty() {
        String::new()
    } else {
        format!("{}\n\n", brief.pitch.trim())
    };
    format!(
        "# {title}\n\n{pitch}Made with the Marionette game starter (`game_make`, template `{template}`, decision 0150).\n\n\
         {controls}\n\n\
         ## Files\n\n\
         - `brief.json`: the whole design (template, words, colors, one line per asset slot, seed).\n\
         - `scenes/scripts/game.ts`: the game. `game_make` rewrites only its CONFIG block.\n\
         - `build.ops.json`: the scene recipe, rebuilt by `game_make`. Put your own scene operations in\n  `custom.ops.json` (same format); they run after it.\n\
         - `assets/generated/*.recipe.json`: art still to generate (`marionette generate run`).\n\
         - `tests/`: the autopilot wins (`autoplay.test.json`); doing nothing does not (`idle.test.json`).\n\n\
         ## Change it\n\n\
         Edit `brief.json`, then `marionette op game_make '{{\"path\": \"<this folder>\", \"refresh\": true}}'` from the\n\
         parent project. After `marionette generate run`, the same refresh swaps the placeholders for the\n\
         generated art and re-pins the tests.\n",
        title = brief.title,
        template = brief.template.name(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_block_round_trips_and_keeps_the_rest() {
        let src = "a\n// BEGIN GAME CONFIG\nconst CONFIG: any = {};\n// END GAME CONFIG\nb\n";
        let out = replace_config(src, "// BEGIN GAME CONFIG\nX\n// END GAME CONFIG").unwrap();
        assert_eq!(out, "a\n// BEGIN GAME CONFIG\nX\n// END GAME CONFIG\nb\n");
        assert!(replace_config("no markers", "x").is_none());
    }

    #[test]
    fn short_number_arrays_go_on_one_line() {
        let v = json!({"c": [0.1, 0.2, 0.3], "map": ["##", "#."]});
        let text = compact_arrays(&serde_json::to_string_pretty(&v).unwrap());
        assert!(text.contains("\"c\": [0.1, 0.2, 0.3]"), "{text}");
        assert!(text.contains("\"##\","), "{text}");
        let back: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(back, v);
    }
}
