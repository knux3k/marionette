// Bounces under gravity and sways toward the entity named "anchor".
// Hooks and engine API: docs/ai/reference/marionette.d.ts.

const GRAVITY = 9.8;
const BOUNCE = 0.8;

export function start(self: Entity): void {
  self.state.bounces = 0;
  self.state.phase = engine.rand() * Math.PI * 2;
}

export function update(self: Entity): void {
  const pos = self.get("Position");
  const vel = self.get("Velocity");
  if (!pos || !vel) return;
  vel.y -= GRAVITY * engine.dt;
  if (pos.y < 0 && vel.y < 0) {
    vel.y = -vel.y * BOUNCE;
    self.state.bounces += 1;
    engine.emit("bounce", { count: self.state.bounces });
  }
  const anchor = engine.find("anchor")?.get("Position");
  if (anchor) {
    vel.x = (anchor.x - pos.x) * 0.5 + Math.sin(engine.tick * 0.05 + self.state.phase);
  }
  self.set("Velocity", vel);
}
