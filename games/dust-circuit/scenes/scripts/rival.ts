// Dust Circuit: the rival car (proof scene, decision 0050). The same car as the player's, but
// driven by this script: it aims at a point a few metres ahead on the inner racing line and
// holds a steady pace. It counts laps the same way and reports its finish to the race.

const ACCEL = 14;
const DRAG = 0.35;
const TURN = 2.0;
const PACE = 15.5; // m/s the rival aims for
const LANE = 10.5; // radius of its line around the curves (and z of the straights)
const HALF = 20; // the straights run from x = -20 to 20
const AHEAD = 6; // metres it looks ahead
const LAPS = 3;

// The point `ahead` metres further along the line from the one nearest to (x, z). The track
// runs +x along z > 0, around the right curve, -x along z < 0 and around the left curve; the
// left half is the right half turned by 180 degrees.
function aim(x: number, z: number): { x: number; z: number } {
  if (x < -HALF || (x <= HALF && z < 0)) {
    const p = aim(-x, -z);
    return { x: -p.x, z: -p.z };
  }
  let a: number; // angle on the right curve, pi/2 at its start and -pi/2 at its end
  if (x <= HALF) {
    if (x + AHEAD <= HALF) return { x: x + AHEAD, z: LANE };
    a = Math.PI / 2 - (x + AHEAD - HALF) / LANE;
  } else {
    a = Math.atan2(z, x - HALF) - AHEAD / LANE;
  }
  if (a < -Math.PI / 2) return { x: HALF - (-Math.PI / 2 - a) * LANE, z: -LANE };
  return { x: HALF + LANE * Math.cos(a), z: LANE * Math.sin(a) };
}

export function start(self: Entity): void {
  const q = self.get("Transform")!.rotation;
  self.state.yaw = 2 * Math.atan2(q.y, q.w);
  self.state.speed = 0;
  self.state.go = false;
  self.state.next = 0;
  self.state.passed = 0;
  self.state.laps = 0;
  self.state.started = false;
  self.state.finished = false;
}

export function onMessage(self: Entity, m: Message): void {
  if (m.kind === "go") self.state.go = true;
}

export function onTriggerEnter(self: Entity, other: Entity): void {
  const n = other.name;
  if (!n || !n.startsWith("cp") || self.state.finished) return;
  const i = Number(n.slice(2));
  if (i !== self.state.next) return;
  self.state.next = (i + 1) % 4;
  self.state.passed += 1;
  if (i !== 0) return;
  if (self.state.started) {
    self.state.laps += 1;
    engine.emit("lap", { car: "rival", lap: self.state.laps });
    if (self.state.laps >= LAPS) {
      self.state.finished = true;
      engine.send(engine.find("race")!, "finish", { car: "rival" });
    }
  }
  self.state.started = true;
}

export function update(self: Entity): void {
  const s = self.state;
  const dt = engine.dt;
  const t = self.get("Transform")!;
  const p = t.translation;
  let steer = 0;
  let throttle = 0;
  if (s.go) {
    const g = aim(p.x, p.z);
    let err = Math.atan2(-(g.x - p.x), -(g.z - p.z)) - s.yaw;
    err = Math.atan2(Math.sin(err), Math.cos(err));
    steer = Math.max(-1, Math.min(1, -2.5 * err));
    throttle = !s.finished && s.speed < PACE ? 1 : 0;
  }
  s.speed += ACCEL * throttle * dt;
  s.speed -= s.speed * DRAG * dt;
  const grip = Math.min(1, Math.abs(s.speed) / 6) * Math.sign(s.speed);
  s.yaw -= steer * TURN * grip * dt;
  t.rotation = { x: 0, y: Math.sin(s.yaw / 2), z: 0, w: Math.cos(s.yaw / 2) };
  self.set("Transform", t);
  const cc = self.get("CharacterController")!;
  cc.speed = Math.abs(s.speed);
  self.set("CharacterController", cc);
  self.set("CharacterInput", { move_x: 0, move_forward: Math.sign(s.speed), jump: false });
}
