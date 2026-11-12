// Waves for one second every three seconds, idling in between (cross-fades of 0.25 s).
// Used by examples/animation.scene.json; see the `playClip` declaration in marionette.d.ts.
export function update(self: Entity): void {
  const t = engine.tick % 180;
  if (t === 30) self.playClip("wave", { fade: 0.25 });
  if (t === 90) self.playClip("idle", { fade: 0.25 });
}
