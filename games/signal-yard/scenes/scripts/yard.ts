// Signal Yard: the director (proof scene, decision 0050). A walled yard at night around a
// relay mast. Drones come in three waves (3, 4 and 5) to posts picked with the seeded random
// generator; clear all three to win, lose all five hit points and the relay stays theirs.
// This script spawns the waves, counts kills and the player's fall from messages, and keeps
// the HUD.

const WAVES = [3, 4, 5];
const POSTS = [
  { x: -12, y: 3.5, z: -12 },
  { x: 0, y: 4.5, z: -11 },
  { x: 12, y: 3.5, z: -12 },
  { x: -14, y: 3, z: 0 },
  { x: 14, y: 3, z: 0 },
  { x: -8, y: 5, z: -4 },
  { x: 8, y: 5, z: -4 },
  { x: 0, y: 3, z: 4 },
];

export function start(self: Entity): void {
  self.state.wave = 0;
  self.state.score = 0;
  self.state.result = "playing";
  engine.after(1.5, nextWave);
}

export function nextWave(self: Entity): void {
  if (self.state.result !== "playing") return;
  self.state.wave += 1;
  const count = WAVES[self.state.wave - 1];
  engine.emit("wave", { wave: self.state.wave, count });
  // Distinct posts: shuffle with the seeded generator and take the first `count`.
  const order = POSTS.map((p) => ({ p, k: engine.rand() }));
  order.sort((a, b) => a.k - b.k);
  for (let i = 0; i < count; i++) {
    const post = order[i].p;
    const drone = engine.spawn(
      {
        Transform: { translation: { x: post.x + 2.5, y: post.y, z: post.z }, rotation: { x: 0, y: 0, z: 0, w: 1 }, scale: { x: 1.1, y: 0.35, z: 1.1 } },
        RigidBody: { kind: "Kinematic", linear_damping: 0, angular_damping: 0, gravity_scale: 0, lock_rotations: true },
        Collider: { shape: { Cuboid: { half_extents: { x: 0.5, y: 0.5, z: 0.5 } } }, density: 1, friction: 0, restitution: 0, sensor: false },
        Model: { asset: "../assets/generated/drone.glb" },
        // Sized to the flat collider box and centred on it; faces -Z.
        ModelPlacement: { offset: { x: 0, y: -0.115, z: 0 }, rotation: { x: 0, y: 0.707107, z: 0, w: 0.707107 }, scale: { x: 0.38, y: 0.38, z: 0.38 } },
        Emissive: { color: { r: 0, g: 0, b: 0, a: 1 }, intensity: 0 },
        Script: { path: "scripts/drone.ts" },
      },
      "drone",
    );
    engine.send(drone, "post", post);
  }
}

export function onMessage(self: Entity, m: Message): void {
  if (self.state.result !== "playing") return;
  if (m.kind === "kill") self.state.score += 1;
  if (m.kind === "down") {
    self.state.result = "lost";
    engine.emit("lost", { wave: self.state.wave, score: self.state.score });
  }
}

export function update(self: Entity): void {
  const s = self.state;
  const drones = engine.query("Script").filter((e) => e.name === "drone").length;
  if (s.result === "playing" && s.wave > 0 && drones === 0 && engine.pending(nextWave) === 0) {
    if (s.wave >= WAVES.length) {
      s.result = "won";
      engine.emit("won", { score: s.score, seconds: engine.tick * engine.dt });
    } else {
      engine.after(2, nextWave);
    }
  }
  const player = engine.find("player")!;
  const ps = JSON.parse(player.get("ScriptState")?.data ?? "{}");
  const hud = engine.find("hud")!;
  const ui = hud.get("UiText")!;
  ui.text = `HP ${ps.hp ?? 5}  WAVE ${s.wave}/${WAVES.length}  DRONES ${drones}  SCORE ${s.score}`;
  hud.set("UiText", ui);
  const b = engine.find("banner")!;
  const bt = b.get("UiText")!;
  bt.text = s.result === "won" ? "THE YARD IS QUIET" : s.result === "lost" ? "SIGNAL LOST" : "";
  b.set("UiText", bt);
  // The relay's beacon pulses.
  const beacon = engine.find("beacon")!;
  const m = beacon.get("Material")!;
  const k = 0.5 + 0.5 * Math.sin(engine.tick * 0.1);
  m.base_color = { r: 0.4 + 0.6 * k, g: 0.05 + 0.1 * k, b: 0.05, a: 1 };
  beacon.set("Material", m);
}
