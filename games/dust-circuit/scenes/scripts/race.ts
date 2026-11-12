// Dust Circuit: the race (proof scene, decision 0050). A three-lap race on a dusty oval
// against one rival. This script counts down and sends both cars "go", takes their "finish"
// messages to decide the result, works out who leads, flies the chase camera behind the
// player and keeps the HUD.

const COUNTDOWN = 3; // seconds
const LAPS = 3;
// Checkpoint centres in driving order, for telling which car leads between checkpoints.
const CPS = [
  { x: 0, z: 12 },
  { x: 32, z: 0 },
  { x: 0, z: -12 },
  { x: -32, z: 0 },
];

export function start(self: Entity): void {
  self.state.phase = "countdown";
  self.state.goTick = 0;
  self.state.order = [];
  self.state.result = "racing";
  engine.after(COUNTDOWN, go);
}

export function go(self: Entity): void {
  self.state.phase = "racing";
  self.state.goTick = engine.tick;
  engine.send(engine.find("player")!, "go");
  engine.send(engine.find("rival")!, "go");
  engine.emit("go");
}

export function onMessage(self: Entity, m: Message): void {
  if (m.kind !== "finish") return;
  const car = (m.detail as { car: string }).car;
  self.state.order.push(car);
  if (self.state.order.length === 1) {
    self.state.result = car === "player" ? "won" : "lost";
    const seconds = (engine.tick - self.state.goTick) * engine.dt;
    engine.emit(self.state.result, { winner: car, seconds });
  }
}

type CarState = { laps: number; passed: number; next: number; speed: number; yaw: number };

function progress(car: Entity, s: CarState): number {
  const p = car.get("Transform")!.translation;
  const c = CPS[s.next];
  return s.passed * 1000 - Math.hypot(p.x - c.x, p.z - c.z);
}

export function update(self: Entity): void {
  const st = self.state;
  const player = engine.find("player")!;
  const rival = engine.find("rival")!;
  const ps = JSON.parse(player.get("ScriptState")?.data ?? "{}") as CarState;
  const rs = JSON.parse(rival.get("ScriptState")?.data ?? "{}") as CarState;

  // Chase camera: eases toward a point behind and above the player's car.
  const t = player.get("Transform")!;
  const p = t.translation;
  const yaw = ps.yaw ?? -Math.PI / 2;
  const fx = -Math.sin(yaw);
  const fz = -Math.cos(yaw);
  const cam = engine.find("camera")!;
  const c = cam.get("Transform")!;
  const k = 0.1;
  c.translation.x += (p.x - fx * 9 - c.translation.x) * k;
  c.translation.y += (p.y + 3.6 - c.translation.y) * k;
  c.translation.z += (p.z - fz * 9 - c.translation.z) * k;
  cam.set("Transform", c);
  cam.set("LookAt", { target: { x: p.x + fx * 6, y: p.y + 0.5, z: p.z + fz * 6 }, up: { x: 0, y: 1, z: 0 } });

  const ahead = ps.passed === undefined || rs.passed === undefined ? true : progress(player, ps) >= progress(rival, rs);
  const time = st.phase === "racing" ? (engine.tick - st.goTick) * engine.dt : 0;
  const hud = engine.find("hud")!;
  const ui = hud.get("UiText")!;
  const lap = Math.min(LAPS, (ps.laps ?? 0) + 1);
  ui.text = `LAP ${lap}/${LAPS}  POS ${ahead ? 1 : 2}/2  TIME ${time.toFixed(1)}  ${Math.round(Math.abs(ps.speed ?? 0) * 3.6)} KM/H`;
  hud.set("UiText", ui);

  const b = engine.find("banner")!;
  const bt = b.get("UiText")!;
  if (st.phase === "countdown") bt.text = String(COUNTDOWN - Math.floor(engine.tick * engine.dt));
  else if (st.result === "won") bt.text = "YOU WIN";
  else if (st.result === "lost") bt.text = "THE RIVAL WINS";
  else bt.text = engine.tick - st.goTick < 60 ? "GO" : "";
  b.set("UiText", bt);
}
