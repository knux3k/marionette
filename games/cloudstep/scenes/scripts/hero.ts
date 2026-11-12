// Cloudstep: the hero (proof scene, decision 0050). A 3D platformer over floating islands.
// Walking and jumping come from the CharacterController and input actions. This script
// collects seeds, bounces on the spring (it sets the controller's vertical speed), keeps
// checkpoints, respawns after a fall and finishes the run at the beacon.

const FALL_Y = -8;
const SPRING_SPEED = 12;

export function start(self: Entity): void {
  self.state.seeds = 0;
  self.state.falls = 0;
  self.state.checkpoint = { x: 0, y: 1.2, z: 0 };
  self.state.result = "running";
  self.state.finishTick = 0;
}

export function onTriggerEnter(self: Entity, other: Entity): void {
  if (self.state.result !== "running") return;
  const at = other.get("Transform")!.translation;
  switch (other.name) {
    case "seed":
      engine.destroy(other);
      self.state.seeds += 1;
      engine.emit("seed", { count: self.state.seeds });
      break;
    case "spring":
      self.set("CharacterState", { grounded: false, vertical_velocity: SPRING_SPEED });
      engine.emit("spring");
      break;
    case "checkpoint":
      if (at.z !== self.state.checkpoint.z) {
        self.state.checkpoint = { x: at.x, y: at.y, z: at.z };
        engine.emit("checkpoint", { z: at.z });
      }
      break;
    case "beacon":
      self.state.result = "won";
      self.state.finishTick = engine.tick;
      engine.emit("won", { seeds: self.state.seeds, falls: self.state.falls, seconds: engine.tick * engine.dt });
      self.set("CharacterInput", { move_x: 0, move_forward: 0, jump: false });
      break;
  }
}

export function update(self: Entity): void {
  const t = self.get("Transform")!;
  if (t.translation.y < FALL_Y) {
    self.state.falls += 1;
    engine.emit("fell", { falls: self.state.falls });
    t.translation = { ...self.state.checkpoint };
    self.set("Transform", t);
    self.set("CharacterState", { grounded: false, vertical_velocity: 0 });
  }
}
