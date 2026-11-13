// Raises the music's intensity by one every 4 seconds, then drops back to the pad alone:
// layers fade in at their `at` values. Used by examples/blend-and-sound.scene.json.
export function update(self: Entity): void {
  if (engine.tick % 240 !== 0) return;
  const m = self.get("Music");
  if (!m) return;
  m.intensity = m.intensity >= 3 ? 0 : m.intensity + 1;
  self.set("Music", m);
}
