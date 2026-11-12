// Signal Yard: the player (proof scene, decision 0050). A first-person shooter on foot.
// Walking, strafing and jumping come from the CharacterController and input actions; this
// script turns the body (mouse, keys or right stick), tilts the view, places the camera at the
// eyes and the gun in front of them, shoots with a ray cast, and takes hits from drone orbs.

const EYE = 0.6; // metres above the body's centre
const LOOK = 0.25; // radians per unit of mouse axis (100 pixels)
const TURN = 2.2; // radians per second at full key or stick
const PITCH_MAX = 1.3;
const COOLDOWN = 9; // ticks between shots
const RANGE = 60;
const HP = 5;

type Q = { x: number; y: number; z: number; w: number };
type V = { x: number; y: number; z: number };

// Yaw around Y, then pitch around the turned X axis.
function view(yaw: number, pitch: number): Q {
  const cy = Math.cos(yaw / 2), sy = Math.sin(yaw / 2), cp = Math.cos(pitch / 2), sp = Math.sin(pitch / 2);
  return { x: cy * sp, y: sy * cp, z: -sy * sp, w: cy * cp };
}

function rotate(q: Q, v: V): V {
  // v + 2w (q × v) + 2 q × (q × v)
  const tx = 2 * (q.y * v.z - q.z * v.y), ty = 2 * (q.z * v.x - q.x * v.z), tz = 2 * (q.x * v.y - q.y * v.x);
  return {
    x: v.x + q.w * tx + (q.y * tz - q.z * ty),
    y: v.y + q.w * ty + (q.z * tx - q.x * tz),
    z: v.z + q.w * tz + (q.x * ty - q.y * tx),
  };
}

export function start(self: Entity): void {
  self.state.yaw = 0;
  self.state.pitch = 0;
  self.state.hp = HP;
  self.state.ready = 0;
  self.state.shots = 0;
  self.state.hits = 0;
  self.state.alive = true;
}

export function onMessage(self: Entity, m: Message): void {
  if (m.kind !== "hurt" || !self.state.alive) return;
  self.state.hp -= 1;
  engine.emit("hurt", { hp: self.state.hp });
  if (self.state.hp <= 0) {
    self.state.alive = false;
    // Stand still from now on: CharacterInput replaces the input actions.
    self.set("CharacterInput", { move_x: 0, move_forward: 0, jump: false });
    engine.send(engine.find("yard")!, "down");
  }
}

export function update(self: Entity): void {
  const s = self.state;
  const dt = engine.dt;
  if (s.alive) {
    s.yaw -= engine.action("look_x") * LOOK + engine.action("turn") * TURN * dt;
    s.pitch -= engine.action("look_y") * LOOK - engine.action("tilt") * TURN * 0.6 * dt;
    s.pitch = Math.max(-PITCH_MAX, Math.min(PITCH_MAX, s.pitch));
  }
  const t = self.get("Transform")!;
  t.rotation = { x: 0, y: Math.sin(s.yaw / 2), z: 0, w: Math.cos(s.yaw / 2) };
  self.set("Transform", t);

  const q = view(s.yaw, s.pitch);
  const eye = { x: t.translation.x, y: t.translation.y + EYE, z: t.translation.z };
  const cam = engine.find("camera")!;
  cam.set("Transform", { translation: eye, rotation: q, scale: { x: 1, y: 1, z: 1 } });
  const kick = engine.tick < s.ready ? (s.ready - engine.tick) / COOLDOWN : 0;
  const g = rotate(q, { x: 0.16, y: -0.14, z: -0.5 + 0.05 * kick });
  engine.find("gun")!.set("Transform", {
    translation: { x: eye.x + g.x, y: eye.y + g.y, z: eye.z + g.z },
    rotation: q,
    scale: { x: 0.045, y: 0.06, z: 0.3 },
  });

  if (s.alive && engine.pressed("fire") && engine.tick >= s.ready) {
    s.ready = engine.tick + COOLDOWN;
    s.shots += 1;
    const dir = rotate(q, { x: 0, y: 0, z: -1 });
    const hit = engine.raycast(eye, dir, RANGE, self);
    const len = hit ? hit.distance : RANGE;
    const muzzle = { x: eye.x + g.x, y: eye.y + g.y, z: eye.z + g.z };
    const end = { x: eye.x + dir.x * len, y: eye.y + dir.y * len, z: eye.z + dir.z * len };
    const dx = end.x - muzzle.x, dy = end.y - muzzle.y, dz = end.z - muzzle.z;
    const tl = Math.hypot(dx, dy, dz);
    // The tracer: a thin bright box from the muzzle to where the shot landed, gone in 4 ticks.
    const ty = Math.atan2(-dx, -dz), tp = Math.asin(dy / tl);
    engine.spawn(
      {
        Transform: { translation: { x: muzzle.x + dx / 2, y: muzzle.y + dy / 2, z: muzzle.z + dz / 2 }, rotation: view(ty, tp), scale: { x: 0.03, y: 0.03, z: tl } },
        Model: { asset: "primitive:cube" },
        Material: { base_color: { r: 1, g: 0.85, b: 0.4, a: 1 }, metallic: 0, roughness: 1 },
        AudioSource: { sound: "builtin:beep", trigger: "OnStart", looping: false, volume: 0.3 },
        DestroyAt: { tick: engine.tick + 4 },
      },
      "tracer",
    );
    engine.emit("shot", { hit: hit ? hit.entity.name : null });
    if (hit && hit.entity.name === "drone") {
      s.hits += 1;
      engine.send(hit.entity, "hit");
    }
  }
}
