// Signal Yard: a drone's orb (proof scene, decision 0050). Flies straight along the velocity
// it was sent; hurts the player on contact and bursts on anything solid.

export function onMessage(self: Entity, m: Message): void {
  if (m.kind === "heading") self.state.v = m.detail;
}

export function onTriggerEnter(self: Entity, other: Entity): void {
  const n = other.name;
  if (n === "drone" || n === "orb" || n === "tracer" || n === "gun") return;
  if (n === "player") engine.send(other, "hurt");
  engine.destroy(self);
}

export function update(self: Entity): void {
  const v = self.state.v;
  if (!v) return;
  const t = self.get("Transform")!;
  const dt = engine.dt;
  t.translation = { x: t.translation.x + v.x * dt, y: t.translation.y + v.y * dt, z: t.translation.z + v.z * dt };
  self.set("Transform", t);
}
