// Crystal Ward: a bolt (proof scene, decision 0050). A kinematic sensor that flies straight
// along the heading the warden sends it. It stops a crawler (and reports the kill to the
// director), breaks on walls and pillars, and expires after a second (DestroyAt).

const SPEED = 18;

export function onMessage(self: Entity, m: Message): void {
  if (m.kind === "heading") self.state.v = m.detail;
}

export function onTriggerEnter(self: Entity, other: Entity): void {
  if (self.state.spent) return;
  if (other.name === "crawler") {
    self.state.spent = true;
    engine.send(engine.find("ward")!, "kill");
    engine.emit("kill", { crawler: other.id });
    engine.destroy(other);
    self.destroy();
  } else if (other.name === "wall" || other.name === "pillar" || other.name === "crystal") {
    self.state.spent = true;
    self.destroy();
  }
}

export function update(self: Entity): void {
  const v = self.state.v;
  if (!v) return;
  const t = self.get("Transform")!;
  t.translation.x += v.vx * SPEED * engine.dt;
  t.translation.z += v.vz * SPEED * engine.dt;
  self.set("Transform", t);
}
