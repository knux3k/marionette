// Ember Run: the level director (proof scene, decision 0050). Follows the runner with the
// camera, drifts the rooftop backdrop and scrolls the far hills slower (parallax), slides the lift back and forth, drops rain
// (falling sensors that pass through the ground) over the two wet stretches with a repeating
// timer, and keeps the HUD current.

const RAIN_ZONES = [
  [37, 50],
  [62, 76],
];

export function start(self: Entity): void {
  const hills = engine.query("Sprite").filter((e) => e.name === "hill");
  self.state.hills = hills.map((h) => ({ id: h.id, x: h.get("Transform")!.translation.x }));
  self.state.drops = 0;
  engine.every(0.4, rain);
}

export function rain(self: Entity): void {
  const runner = engine.find("runner");
  if (!runner) return;
  const x = runner.get("Transform")!.translation.x;
  const zone = RAIN_ZONES.find(([a, b]) => x > a - 4 && x < b);
  if (!zone) return;
  // A drop lands ahead of the runner, somewhere in the zone.
  const dx = Math.min(zone[1], Math.max(zone[0], x + 3 + engine.rand() * 10));
  engine.spawn(
    {
      Transform: { translation: { x: dx, y: 9, z: 0 }, rotation: { x: 0, y: 0, z: 0, w: 1 }, scale: { x: 1, y: 1, z: 1 } },
      RigidBody: { kind: "Dynamic", linear_damping: 0, angular_damping: 0, gravity_scale: 0.6, lock_rotations: false },
      Collider: { shape: { Ball: { radius: 0.22 } }, density: 1, friction: 0, restitution: 0, sensor: true },
      Sprite: { image: "builtin:circle", color: { r: 0.45, g: 0.7, b: 1.0, a: 0.9 }, size: { x: 0.44, y: 0.44 }, layer: 2 },
      DestroyAt: { tick: engine.tick + 150 },
    },
    "drop",
  );
  self.state.drops += 1;
}

export function update(self: Entity): void {
  const runner = engine.find("runner");
  if (!runner) return;
  const rx = runner.get("Transform")!.translation.x;

  const cam = engine.find("camera")!;
  const ct = cam.get("Transform")!;
  ct.translation.x = Math.min(76, Math.max(4, rx + 3));
  cam.set("Transform", ct);

  // The rooftop backdrop drifts slowly: it stays within its slack at both ends of the level.
  const sky = engine.find("sky");
  if (sky) {
    const st = sky.get("Transform")!;
    st.translation.x = ct.translation.x * 0.92 + 3.2;
    sky.set("Transform", st);
  }

  for (const h of self.state.hills) {
    const e = engine.entity(h.id)!;
    const t = e.get("Transform")!;
    t.translation.x = h.x + ct.translation.x * 0.5;
    e.set("Transform", t);
  }

  const lift = engine.find("lift")!;
  const lt = lift.get("Transform")!;
  lt.translation.x = 54 + 2.5 * Math.sin(engine.tick * engine.dt * 1.2);
  lift.set("Transform", lt);

  // Another entity's `state` is not visible from here (gap G2): read its component.
  const data = JSON.parse(runner.get("ScriptState")?.data ?? "{}");
  const hud = engine.find("hud")!;
  const ui = hud.get("UiText")!;
  ui.text = `EMBERS ${data.embers ?? 0}  LIVES ${data.lives ?? 3}`;
  hud.set("UiText", ui);
  const banner = engine.find("banner")!;
  const b = banner.get("UiText")!;
  b.text = data.result === "won" ? "THE BEACON IS LIT" : data.result === "lost" ? "THE SPARK WENT OUT" : "";
  banner.set("UiText", b);
}
