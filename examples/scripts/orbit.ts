// Circles the listener (radius 3 m, one turn every 6 s) so the hum pans around the head and
// is muffled while behind it. Used by examples/blend-and-sound.scene.json.
export function update(self: Entity): void {
  const a = (engine.tick / 360) * Math.PI * 2;
  const t = self.get("Transform");
  if (!t) return;
  t.translation.x = Math.sin(a) * 3;
  t.translation.z = 4.2 - Math.cos(a) * 3;
  self.set("Transform", t);
}
