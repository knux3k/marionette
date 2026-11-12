// Crystal Ward: a crawler (proof scene, decision 0050). Turns toward the crystal and walks
// with the character controller under script control (CharacterInput), sliding around
// pillars. Touching the ward ring breaches it: it tells the director and is gone.

export function start(self: Entity): void {
  self.state.wobble = engine.rand() * 6.28;
}

export function onTriggerEnter(self: Entity, other: Entity): void {
  if (other.name === "ward-ring" && !self.state.done) {
    self.state.done = true;
    engine.send(engine.find("ward")!, "breach");
    engine.emit("breach", { crawler: self.id });
    self.destroy();
  }
}

export function update(self: Entity): void {
  const t = self.get("Transform")!;
  const p = t.translation;
  // Face the crystal at the origin, weaving a little so crawlers do not walk in single file.
  // Forward (-Z) turned by `yaw` about +Y points along (-sin yaw, 0, -cos yaw).
  const yaw = Math.atan2(p.x, p.z) + 0.35 * Math.sin(engine.tick * 0.05 + self.state.wobble);
  t.rotation = { x: 0, y: Math.sin(yaw / 2), z: 0, w: Math.cos(yaw / 2) };
  self.set("Transform", t);
}
