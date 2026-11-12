// Crystal Ward: the director (proof scene, decision 0050). A top-down 3D arena: crawlers come
// in waves from the edges and walk to the crystal; the warden's bolts stop them. Five waves.
// This script schedules the waves (timers), spawns crawlers at seeded random gates, counts
// crystal damage and kills (messages from crawlers and bolts) and keeps the HUD.

const WAVES = 5;
const CRYSTAL_HP = 10;
const GATES = [
  { x: 0, z: -14.5 },
  { x: 14.5, z: 0 },
  { x: 0, z: 14.5 },
  { x: -14.5, z: 0 },
  { x: -12, z: -12 },
  { x: 12, z: 12 },
];

export function start(self: Entity): void {
  self.state.wave = 0;
  self.state.hp = CRYSTAL_HP;
  self.state.score = 0;
  self.state.result = "playing";
  engine.every(7, nextWave, null, 1);
}

export function nextWave(self: Entity): void {
  if (self.state.result !== "playing") return;
  if (self.state.wave >= WAVES) {
    engine.cancel(nextWave);
    return;
  }
  self.state.wave += 1;
  const count = 2 + self.state.wave;
  engine.emit("wave", { wave: self.state.wave, count });
  for (let i = 0; i < count; i++) engine.after(0.4 * i, spawnCrawler, self.state.wave);
}

export function spawnCrawler(self: Entity, wave: number): void {
  if (self.state.result !== "playing") return;
  const g = GATES[Math.floor(engine.rand() * GATES.length)];
  engine.spawn(
    {
      Transform: { translation: { x: g.x, y: 0.8, z: g.z }, rotation: { x: 0, y: 0, z: 0, w: 1 }, scale: { x: 1, y: 0.8, z: 1 } },
      RigidBody: { kind: "Kinematic", linear_damping: 0, angular_damping: 0, gravity_scale: 1, lock_rotations: true },
      Collider: { shape: { Ball: { radius: 0.45 } }, density: 1, friction: 0, restitution: 0, sensor: false },
      CharacterController: { speed: 2 + 0.25 * wave, jump_speed: 0, gravity: 20, mass: 20, move_x: "none", move_forward: "none", jump: "none" },
      CharacterState: { grounded: false, vertical_velocity: 0 },
      CharacterInput: { move_x: 0, move_forward: 1, jump: false },
      Model: { asset: "primitive:sphere" },
      Material: { base_color: { r: 0.7, g: 0.1 + 0.05 * wave, b: 0.25, a: 1 }, metallic: 0, roughness: 0.6 },
      Script: { path: "scripts/crawler.ts" },
    },
    "crawler",
  );
}

// Crawlers report a breach and bolts report a kill with engine.send.
export function onMessage(self: Entity, m: Message): void {
  if (self.state.result !== "playing") return;
  if (m.kind === "breach") {
    self.state.hp -= 1;
    engine.emit("crystal_hit", { hp: self.state.hp });
  } else if (m.kind === "kill") {
    self.state.score += 1;
  }
}

export function update(self: Entity): void {
  const s = self.state;
  const crawlers = engine.query("CharacterInput").filter((e) => e.name === "crawler");
  if (s.result === "playing") {
    if (s.hp <= 0) {
      s.result = "lost";
      engine.emit("lost", { wave: s.wave });
    } else if (s.wave >= WAVES && engine.pending(nextWave) === 0 && engine.pending(spawnCrawler) === 0 && crawlers.length === 0) {
      s.result = "won";
      engine.emit("won", { hp: s.hp, score: s.score });
    }
  }
  const hud = engine.find("hud")!;
  const ui = hud.get("UiText")!;
  ui.text = `WAVE ${s.wave}/${WAVES}  CRYSTAL ${s.hp}  CRAWLERS ${crawlers.length}  SCORE ${s.score}`;
  hud.set("UiText", ui);
  if (s.result !== "playing") {
    const b = engine.find("banner")!;
    const bt = b.get("UiText")!;
    bt.text = s.result === "won" ? "THE CRYSTAL HOLDS" : "THE CRYSTAL SHATTERED";
    b.set("UiText", bt);
  }
  // The crystal slowly turns and dims with damage.
  const crystal = engine.find("crystal")!;
  const m = crystal.get("Material")!;
  const k = Math.max(0, s.hp) / CRYSTAL_HP;
  m.base_color = { r: 0.3 * k + 0.2, g: 0.9 * k + 0.05, b: 1.0 * k + 0.1, a: 1 };
  crystal.set("Material", m);
}
