// Ember Run: the runner (proof scene, decision 0050). A spark that runs right across
// rooftops, collects embers, dodges falling drops and lights the beacon at the far end.
// Movement and jumping come from the CharacterController and the input actions; this script
// handles pickups and hits (trigger hooks), falls, checkpoints and lives.

const LIVES = 3;
const FALL_Y = -4;

export function start(self: Entity): void {
  self.state.lives = LIVES;
  self.state.embers = 0;
  self.state.checkpoint = { x: 0, y: 1.5 };
  self.state.reached = [] as number[];
  self.state.hurtUntil = 0;
  self.state.result = "running";
}

export function onTriggerEnter(self: Entity, other: Entity): void {
  if (self.state.result !== "running") return;
  switch (other.name) {
    case "ember":
      engine.destroy(other);
      self.state.embers += 1;
      engine.emit("ember", { count: self.state.embers });
      break;
    case "checkpoint": {
      const { x, y } = other.get("Transform")!.translation;
      if (!self.state.reached.includes(x)) {
        self.state.reached.push(x);
        self.state.checkpoint = { x, y };
        engine.emit("checkpoint", { x });
      }
      break;
    }
    case "drop":
      if (engine.tick >= self.state.hurtUntil) {
        engine.destroy(other);
        hurt(self, "drop");
      }
      break;
    case "beacon":
      self.state.result = "won";
      engine.emit("won", { embers: self.state.embers, lives: self.state.lives, tick: engine.tick });
      break;
  }
}


function hurt(self: Entity, cause: string): void {
  if (self.state.result !== "running") return;
  self.state.lives -= 1;
  self.state.hurtUntil = engine.tick + 90; // 1.5 s of grace
  engine.emit("hurt", { cause, lives: self.state.lives });
  if (self.state.lives <= 0) {
    self.state.result = "lost";
    engine.emit("lost");
  }
}

export function update(self: Entity): void {
  const t = self.get("Transform")!;
  if (t.translation.y < FALL_Y) {
    hurt(self, "fall");
    // Teleport: physics moves a character whose Transform a script changed.
    t.translation.x = self.state.checkpoint.x;
    t.translation.y = self.state.checkpoint.y;
    self.set("Transform", t);
    self.set("CharacterState", { grounded: false, vertical_velocity: 0 });
  }
  // Blink while hurt; stop taking input once the run is over.
  const s = self.get("Sprite")!;
  const blinking = engine.tick < self.state.hurtUntil && Math.floor(engine.tick / 6) % 2 === 0;
  s.color.a = blinking ? 0.3 : 1;
  self.set("Sprite", s);
  if (self.state.result !== "running" && !self.has("CharacterInput")) {
    self.set("CharacterInput", { move_x: 0, move_forward: 0, jump: false });
  }
}
