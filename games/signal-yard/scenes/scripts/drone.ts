// Signal Yard: a drone (proof scene, decision 0050). Circles its post in the air, bobbing,
// and every couple of seconds fires a slow orb at the player if it can see them (a ray cast
// toward the player's eyes). Two hits bring it down. Its post arrives as a message from the
// yard that spawned it.

const HP = 2;
const ORBIT = 2.5; // metres
const SPIN = 0.7; // radians per second
const ORB_SPEED = 8; // m/s

export function start(self: Entity): void {
  self.state.hp = HP;
  self.state.home = null;
  self.state.phase = engine.rand() * Math.PI * 2;
  self.state.flash = 0;
  engine.every(2.4, fire, null, 1 + engine.rand() * 1.5);
}

export function onMessage(self: Entity, m: Message): void {
  if (m.kind === "post") self.state.home = m.detail;
  if (m.kind === "hit" && self.state.hp > 0) {
    self.state.hp -= 1;
    self.state.flash = 6;
    if (self.state.hp <= 0) {
      engine.emit("drone_down", { id: self.id });
      engine.send(engine.find("yard")!, "kill");
      engine.destroy(self);
    }
  }
}

export function fire(self: Entity): void {
  const player = engine.find("player");
  if (!player || self.state.hp <= 0) return;
  const p = self.get("Transform")!.translation;
  const q = player.get("Transform")!.translation;
  const to = { x: q.x - p.x, y: q.y + 0.6 - p.y, z: q.z - p.z };
  const d = Math.hypot(to.x, to.y, to.z);
  const hit = engine.raycast(p, to, d + 1, self);
  if (!hit || hit.entity.name !== "player") return;
  const v = { x: to.x / d, y: to.y / d, z: to.z / d };
  const orb = engine.spawn(
    {
      Transform: { translation: { x: p.x + v.x * 0.7, y: p.y + v.y * 0.7, z: p.z + v.z * 0.7 }, rotation: { x: 0, y: 0, z: 0, w: 1 }, scale: { x: 0.35, y: 0.35, z: 0.35 } },
      RigidBody: { kind: "Kinematic", linear_damping: 0, angular_damping: 0, gravity_scale: 0, lock_rotations: true },
      Collider: { shape: { Ball: { radius: 0.5 } }, density: 1, friction: 0, restitution: 0, sensor: true },
      Model: { asset: "primitive:sphere" },
      Material: { base_color: { r: 1, g: 0.2, b: 0.6, a: 1 }, metallic: 0, roughness: 0.2 },
      Script: { path: "scripts/orb.ts" },
      DestroyAt: { tick: engine.tick + 360 },
    },
    "orb",
  );
  engine.send(orb, "heading", { x: v.x * ORB_SPEED, y: v.y * ORB_SPEED, z: v.z * ORB_SPEED });
  engine.emit("orb");
}

export function update(self: Entity): void {
  const s = self.state;
  if (!s.home) return;
  const a = s.phase + engine.tick * engine.dt * SPIN;
  const t = self.get("Transform")!;
  t.translation = {
    x: s.home.x + Math.cos(a) * ORBIT,
    y: s.home.y + 0.35 * Math.sin(a * 2.3),
    z: s.home.z + Math.sin(a) * ORBIT,
  };
  t.rotation = { x: 0, y: Math.sin(-a / 2), z: 0, w: Math.cos(-a / 2) };
  self.set("Transform", t);
  // Hit feedback as glow, so the model keeps its textures: white flash, then orange once hurt.
  self.set(
    "Emissive",
    s.flash > 0
      ? { color: { r: 1, g: 1, b: 1, a: 1 }, intensity: 1.5 }
      : s.hp < HP
        ? { color: { r: 1, g: 0.35, b: 0.1, a: 1 }, intensity: 0.6 }
        : { color: { r: 0, g: 0, b: 0, a: 1 }, intensity: 0 },
  );
  if (s.flash > 0) s.flash -= 1;
}
