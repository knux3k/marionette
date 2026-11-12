// Dust Circuit: the player's car (proof scene, decision 0050). The car is a kinematic
// character: this script keeps its speed and heading (throttle, brake, drag, steering that
// grows with speed), turns it by setting Transform.rotation and moves it through
// CharacterInput. Laps are counted from the checkpoint sensors cp0..cp3, in order.

const ACCEL = 14; // m/s² at full throttle
const BRAKE = 24; // m/s² when braking
const DRAG = 0.35; // share of speed lost per second
const MAX = 21; // m/s
const REVERSE = -5; // m/s
const TURN = 2.0; // rad/s at full lock
const LAPS = 3;

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
    engine.emit("lap", { car: "player", lap: self.state.laps });
    if (self.state.laps >= LAPS) {
      self.state.finished = true;
      engine.send(engine.find("race")!, "finish", { car: "player" });
    }
  }
  self.state.started = true;
}

export function update(self: Entity): void {
  const s = self.state;
  const dt = engine.dt;
  const throttle = s.go && !s.finished ? engine.action("throttle") : 0;
  const steer = s.go ? engine.action("steer") : 0;
  if (throttle > 0) s.speed += ACCEL * throttle * dt;
  else if (throttle < 0) s.speed += (s.speed > 0 ? BRAKE : ACCEL * 0.5) * throttle * dt;
  s.speed -= s.speed * DRAG * dt;
  s.speed = Math.min(MAX, Math.max(REVERSE, s.speed));
  // Steering bites harder as the car speeds up and reverses when going backwards.
  const grip = Math.min(1, Math.abs(s.speed) / 6) * Math.sign(s.speed);
  s.yaw -= steer * TURN * grip * dt;
  const t = self.get("Transform")!;
  t.rotation = { x: 0, y: Math.sin(s.yaw / 2), z: 0, w: Math.cos(s.yaw / 2) };
  self.set("Transform", t);
  const cc = self.get("CharacterController")!;
  cc.speed = Math.abs(s.speed);
  self.set("CharacterController", cc);
  self.set("CharacterInput", { move_x: 0, move_forward: Math.sign(s.speed), jump: false });
}
