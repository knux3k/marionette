// Crystal Ward: the warden (proof scene, decision 0050). Walks where the stick or keys point
// (the script turns the character to face that way and drives it with CharacterInput) and,
// while fire is held, throws a bolt at the nearest crawler within range every fifth of a second.

const RANGE = 15;
const COOLDOWN = 12; // ticks

export function start(self: Entity): void {
  self.state.ready = 0;
  self.state.shots = 0;
}

export function update(self: Entity): void {
  const t = self.get("Transform")!;
  const dx = engine.action("move_x");
  const dz = -engine.action("move_forward");
  const len = Math.hypot(dx, dz);
  if (len > 0.01) {
    const yaw = Math.atan2(-dx, -dz);
    t.rotation = { x: 0, y: Math.sin(yaw / 2), z: 0, w: Math.cos(yaw / 2) };
    self.set("Transform", t);
  }
  self.set("CharacterInput", { move_x: 0, move_forward: Math.min(1, len), jump: false });

  if (engine.pressed("fire") && engine.tick >= self.state.ready) {
    const p = t.translation;
    let best: Entity | null = null;
    let bestD = RANGE;
    for (const c of engine.query("CharacterInput")) {
      if (c.name !== "crawler") continue;
      const q = c.get("Transform")!.translation;
      const d = Math.hypot(q.x - p.x, q.z - p.z);
      if (d < bestD) {
        bestD = d;
        best = c;
      }
    }
    if (best) {
      const q = best.get("Transform")!.translation;
      const vx = (q.x - p.x) / bestD;
      const vz = (q.z - p.z) / bestD;
      const bolt = engine.spawn(
        {
          Transform: { translation: { x: p.x + vx * 0.7, y: 0.9, z: p.z + vz * 0.7 }, rotation: { x: 0, y: 0, z: 0, w: 1 }, scale: { x: 0.3, y: 0.3, z: 0.3 } },
          RigidBody: { kind: "Kinematic", linear_damping: 0, angular_damping: 0, gravity_scale: 0, lock_rotations: true },
          Collider: { shape: { Ball: { radius: 0.5 } }, density: 1, friction: 0, restitution: 0, sensor: true },
          Model: { asset: "primitive:sphere" },
          Material: { base_color: { r: 1, g: 0.9, b: 0.5, a: 1 }, metallic: 0, roughness: 0.2 },
          AudioSource: { sound: "builtin:beep", trigger: "OnStart", looping: false, volume: 0.4 },
          Script: { path: "scripts/bolt.ts" },
          DestroyAt: { tick: engine.tick + 60 },
        },
        "bolt",
      );
      // A spawned entity's script starts fresh; its heading arrives as a message.
      engine.send(bolt, "heading", { vx, vz });
      engine.emit("shot", { vx, vz });
      self.state.shots += 1;
      self.state.ready = engine.tick + COOLDOWN;
    }
  }
}
