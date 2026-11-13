// Speeds up from idle to a walk and back every 8 seconds through the "move" blend space
// (idle at 0, walk at 1.5, phase-synced), and waves with the right arm (an overlay masked to
// `upper_arm.R`) for two seconds of every eight. Used by examples/blend-and-sound.scene.json.
export function update(self: Entity): void {
  const t = engine.tick % 480;
  if (t === 0) self.setBlend("move", 1.5, 0, { rate: 0.75 });
  if (t === 240) self.setBlend("move", 0, 0, { rate: 0.75 });
  if (t === 120) self.playOverlay("arm", "wave", { mask: ["upper_arm.R"], fade: 0.3 });
  if (t === 240) self.stopOverlay("arm", { fade: 0.3 });
}
