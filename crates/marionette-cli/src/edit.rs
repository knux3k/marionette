//! `marionette edit`: the scene editor window (decision 0130). Edits go through the agent's
//! operations, so they are validated and written exactly like `marionette op` edits.

use marionette_agent::{Agent, engine};
use marionette_core::{Errors, Sim};
use marionette_editor::Editor;
use marionette_editor::window::{EditorOptions, run};
use serde_json::{Value, json};
use std::path::Path;

struct Host<'a> {
    agent: &'a mut Agent,
    scene: String,
    seed: u64,
    player: Option<engine::SoundPlayer>,
    problems: Vec<String>,
}

impl Host<'_> {
    fn note(&mut self, m: String) {
        if self.problems.len() < 20 && !self.problems.contains(&m) {
            self.problems.push(m);
        }
    }
}

impl marionette_editor::Host for Host<'_> {
    fn load(&mut self) -> Result<Sim, Errors> {
        self.agent.load_sim(&self.scene, self.seed)
    }
    fn call(&mut self, op: &str, input: Value) -> Result<Value, Errors> {
        self.agent.call(op, input)
    }
    fn device(&mut self, sim: &mut Sim, input: &str, value: f32) {
        let tick = sim.tick();
        if let Err(e) = engine::queue_live(sim, tick, input, false, value) {
            self.note(e.to_string());
        }
    }
    fn after_ticks(&mut self, sim: &mut Sim) {
        if let Some(p) = self.player.as_mut() {
            for m in p.play_new(sim.events()) {
                self.note(m);
            }
        }
    }
}

/// Options of `marionette edit`.
pub struct EditRun<'a> {
    pub scene: &'a str,
    pub seed: u64,
    pub width: u32,
    pub height: u32,
    pub mute: bool,
    pub frames: Option<u64>,
}

pub fn edit(agent: &mut Agent, o: EditRun) -> Result<Value, Errors> {
    let base = agent
        .resolve(o.scene)?
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let sim = agent.load_sim(o.scene, o.seed)?;
    let mut host = Host {
        agent,
        scene: o.scene.to_string(),
        seed: o.seed,
        player: None,
        problems: Vec::new(),
    };
    if !o.mute {
        match engine::SoundPlayer::new(base.clone()) {
            Ok(p) => host.player = Some(p),
            Err(m) => host.problems.push(m),
        }
    }
    let opts = EditorOptions {
        title: format!("Marionette editor - {}", o.scene),
        width: o.width,
        height: o.height,
        sample_count: 4,
        base_dir: base,
        max_frames: o.frames,
    };
    let editor = run(Editor::new(o.scene, sim), opts, &mut host).map_err(|e| e.with_file(o.scene))?;
    Ok(json!({
        "ok": true,
        "command": "edit",
        "file": o.scene,
        "tick": editor.sim.tick(),
        "selected": editor.selected,
        "edits_undoable": editor.undo_depth(),
        "problems": host.problems,
    }))
}
