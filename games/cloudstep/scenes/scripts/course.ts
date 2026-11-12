// Cloudstep: the course (proof scene, decision 0050). Slides the ferry, spins and bobs the
// seeds, flies the camera behind the hero, and writes the HUD (seeds and the run time).

export function start(self: Entity): void {
  self.state.seeds = engine.query("Model").filter((e) => e.name === "seed").map((e) => ({ id: e.id, y: e.get("Transform")!.translation.y }));
}

export function update(self: Entity): void {
  const time = engine.tick * engine.dt;

  const ferry = engine.find("ferry")!;
  const f = ferry.get("Transform")!;
  f.translation.x = 3 * Math.sin(time * 0.6);
  ferry.set("Transform", f);

  for (const s of self.state.seeds) {
    const e = engine.entity(s.id);
    if (!e) continue; // collected
    const t = e.get("Transform")!;
    const a = time * 2 + s.id;
    t.translation.y = s.y + 0.15 * Math.sin(a * 1.5);
    t.rotation = { x: 0, y: Math.sin(a / 2), z: 0, w: Math.cos(a / 2) };
    e.set("Transform", t);
  }

  const hero = engine.find("hero")!;
  const p = hero.get("Transform")!.translation;
  const cam = engine.find("camera")!;
  const c = cam.get("Transform")!;
  // Ease toward a point behind and above the hero (the camera runs one tick behind physics).
  const k = 0.12;
  c.translation.x += (p.x - c.translation.x) * k;
  c.translation.y += (p.y + 4.5 - c.translation.y) * k;
  c.translation.z += (p.z + 7.5 - c.translation.z) * k;
  cam.set("Transform", c);
  cam.set("LookAt", { target: { x: p.x, y: p.y + 0.5, z: p.z - 2 }, up: { x: 0, y: 1, z: 0 } });

  const data = JSON.parse(hero.get("ScriptState")?.data ?? "{}");
  const shown = data.result === "won" ? data.finishTick * engine.dt : time;
  const hud = engine.find("hud")!;
  const ui = hud.get("UiText")!;
  ui.text = `SEEDS ${data.seeds ?? 0}/8  TIME ${shown.toFixed(1)}`;
  hud.set("UiText", ui);
  if (data.result === "won") {
    const b = engine.find("banner")!;
    const bt = b.get("UiText")!;
    bt.text = `BEACON REACHED IN ${shown.toFixed(1)} S`;
    b.set("UiText", bt);
  }
}
