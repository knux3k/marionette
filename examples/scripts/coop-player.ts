// One player of a networked co-op scene. The entity named "player-<slot>" moves with that
// player's actions (`p<slot>.move_x`, `p<slot>.move_y`) and hops on `p<slot>.jump`, but only
// while the slot is in the session (`NetPlayers`). Used by examples/coop.scene.json and the
// `net_simulate` proofs (decision 0211).
export function update(self: Entity): void {
  const slot = Number((self.name ?? "player-0").slice(7));
  const players = engine.resource("NetPlayers");
  const here = players !== undefined && players.slots.indexOf(slot) >= 0;
  const p = "p" + slot + ".";
  const v = self.get("Velocity");
  const pos = self.get("Position");
  if (v === undefined || pos === undefined) return;
  if (!here) {
    self.set("Velocity", { x: 0, y: 0, z: 0 });
    return;
  }
  const up = engine.justPressed(p + "jump") && pos.y <= 0 ? 4 : v.y - 9.8 * engine.dt;
  self.set("Velocity", { x: engine.action(p + "move_x") * 3, y: up, z: engine.action(p + "move_y") * 3 });
  if (pos.y < 0) self.set("Position", { x: pos.x, y: 0, z: pos.z });
}
