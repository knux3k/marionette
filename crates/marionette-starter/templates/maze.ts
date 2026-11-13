// Maze game (starter template `maze`, decision 0150).
// Collect every pickup while enemies wander the corridors, then reach the goal.
// Touching an enemy costs a life and sends you back to the start.
//
// Keys: arrows or WASD move, R restarts after a win or loss, F1 toggles the autopilot
// (the tests use it to prove the level can be won).
//
// The level, the art and the numbers live in the CONFIG block below; `game_make` rewrites
// only that block (from brief.json), so changes elsewhere in this file are kept.
// Everything that changes during play lives in self.state (snapshots and the state hash).

// BEGIN GAME CONFIG
const CONFIG: any = {};
// END GAME CONFIG

type Dir = [number, number];
type Mover = { x: number; y: number; dx: number; dy: number; t: number; id: number; yaw: number };
type Rgb = [number, number, number];

const MAP: string[] = CONFIG.map;
const W = MAP[0].length;
const H = MAP.length;
const THREE_D = CONFIG.view === "3d";
const DIRS: Dir[] = [
  [1, 0],
  [-1, 0],
  [0, 1],
  [0, -1],
];
const IDENTITY = { x: 0, y: 0, z: 0, w: 1 };

const rgba = (c: Rgb, a = 1) => ({ r: c[0], g: c[1], b: c[2], a });
const tileAt = (x: number, y: number) => (y < 0 || y >= H || x < 0 || x >= W ? "#" : MAP[y][x]);
const key = (x: number, y: number) => y * W + x;

// Tile (x, y) to world space: 2D is x right, y up; 3D lays the maze on the ground (x, z).
function world(x: number, y: number, lift = 0) {
  const wx = x - (W - 1) / 2;
  const wy = (H - 1) / 2 - y;
  return THREE_D ? { x: wx, y: lift, z: -wy } : { x: wx, y: wy, z: 0 };
}

function yawRotation(yaw: number) {
  return { x: 0, y: Math.sin(yaw / 2), z: 0, w: Math.cos(yaw / 2) };
}

// Components that draw one slot's art at a tile. `size` is the slot's footprint in tiles.
function body(slot: string, x: number, y: number, size: number, layer: number, tint: Rgb = [1, 1, 1]): any {
  const art = CONFIG.art[slot];
  if (!THREE_D) {
    const t = { translation: world(x, y), rotation: IDENTITY, scale: { x: 1, y: 1, z: 1 } };
    if (art.kind === "sprite") {
      const a = art.aspect;
      const e: any = {
        Transform: t,
        Sprite: {
          image: art.image,
          color: rgba(tint),
          size: a >= 1 ? { x: size, y: size / a } : { x: size * a, y: size },
          layer,
        },
      };
      if (art.region) e.SpriteRegion = { x: art.region[0], y: art.region[1], width: art.region[2], height: art.region[3] };
      return e;
    }
    const round = slot !== "wall" && slot !== "floor" && slot !== "goal";
    return {
      Transform: t,
      Sprite: {
        image: round ? "builtin:circle" : "builtin:white",
        color: rgba([art.color[0] * tint[0], art.color[1] * tint[1], art.color[2] * tint[2]]),
        size: { x: size, y: size },
        layer,
      },
    };
  }
  if (art.kind === "model") {
    const s = art.scale;
    return {
      Transform: { translation: world(x, y), rotation: IDENTITY, scale: { x: s, y: s, z: s } },
      Model: { asset: art.asset },
    };
  }
  // Placeholder shapes: walls and the goal are boxes, the rest are balls.
  const box = slot === "wall" || slot === "goal";
  const tall = slot === "goal" ? 1.4 : size;
  const c = art.kind === "shape" ? art.color : [0.7, 0.7, 0.7];
  return {
    Transform: {
      translation: world(x, y, tall / 2),
      rotation: IDENTITY,
      scale: { x: size, y: tall, z: size },
    },
    Model: { asset: box ? "primitive:cube" : "primitive:sphere" },
    Material: { base_color: rgba([c[0] * tint[0], c[1] * tint[1], c[2] * tint[2]]), metallic: 0, roughness: 0.7 },
  };
}

function spawn(s: any, components: any, name: string): number {
  const id = engine.spawn(components, name).id;
  s.spawned.push(id);
  return id;
}

export function start(self: Entity): void {
  setup(self.state);
}

function setup(s: any): void {
  s.spawned = [];
  s.pickups = [] as { x: number; y: number; id: number }[];
  s.enemies = [] as Mover[];
  s.enemyHomes = [] as { x: number; y: number }[];
  s.collected = 0;
  s.lives = CONFIG.lives;
  s.result = "playing";
  s.safe = false;
  s.auto = s.auto ?? false;
  s.spin = 0;
  for (let y = 0; y < H; y++) {
    for (let x = 0; x < W; x++) {
      const c = MAP[y][x];
      if (c === "#") spawn(s, body("wall", x, y, 1, 1), "wall");
      else if (CONFIG.art.floor) spawn(s, body("floor", x, y, 1, 0, [0.8, 0.8, 0.8]), "floor");
      if (c === "*") s.pickups.push({ x, y, id: spawn(s, withGlow(body("pickup", x, y, 0.6, 2)), "pickup") });
      if (c === "E") s.goal = { x, y, id: spawn(s, body("goal", x, y, 0.9, 2, [0.45, 0.45, 0.5]), "goal") };
      if (c === "P") {
        s.home = { x, y };
        s.player = { x, y, dx: 0, dy: 0, t: 0, yaw: 0, id: spawn(s, body("player", x, y, 0.8, 4), "player") } as Mover;
      }
      if (c === "G") s.enemyHomes.push({ x, y });
    }
  }
  s.enemyHomes.forEach((_: unknown, i: number) => engine.after(CONFIG.enemyDelay * (i + 1), releaseEnemy, i));
  banner(CONFIG.title, 2.5);
  hud(s);
}

// 3D pickups glow (with bloom in the scene); 2D pickups are drawn as they are.
function withGlow(e: any): any {
  if (THREE_D) e.Emissive = { color: rgba(CONFIG.colors.accent), intensity: 1.5 };
  return e;
}

export function releaseEnemy(self: Entity, i: number): void {
  const s = self.state;
  if (s.result !== "playing") return;
  const home = s.enemyHomes[i];
  const id = spawn(s, body("enemy", home.x, home.y, 0.75, 3), "enemy");
  s.enemies.push({ x: home.x, y: home.y, dx: 0, dy: 0, t: 0, yaw: 0, id });
  engine.emit("enemy_released", { enemy: id });
}

export function endSafe(self: Entity): void {
  self.state.safe = false;
}

export function clearBanner(self: Entity): void {
  if (self.state.result === "playing") banner("", 0);
}

function banner(text: string, seconds: number): void {
  const b = engine.find("banner");
  if (!b) return;
  const ui = b.get("UiText")!;
  ui.text = text;
  b.set("UiText", ui);
  if (seconds > 0) engine.after(seconds, clearBanner);
}

function hud(s: any): void {
  const text = engine.find("hud");
  if (!text) return;
  const ui = text.get("UiText")!;
  const auto = s.auto ? "  AUTOPILOT" : "";
  ui.text = `${CONFIG.words.pickups} ${s.collected}/${s.pickups.length + s.collected}  LIVES ${s.lives}${auto}`;
  text.set("UiText", ui);
}

function open(s: any, x: number, y: number): boolean {
  const c = tileAt(x, y);
  if (c === "#") return false;
  if (c === "E") return s.pickups.length === 0;
  return true;
}

// Advances a mover one tick; at a tile center `choose` picks the next direction.
function advance(m: Mover, step: number, choose: (m: Mover) => Dir | null): void {
  if (m.t === 0) {
    const d = choose(m);
    if (!d) {
      m.dx = 0;
      m.dy = 0;
      return;
    }
    m.dx = d[0];
    m.dy = d[1];
    // Face the way it walks: models face +Z when not turned.
    m.yaw = Math.atan2(m.dx, m.dy);
  }
  m.t += 1;
  if (m.t >= step) {
    m.x += m.dx;
    m.y += m.dy;
    m.t = 0;
  }
}

const at = (m: Mover, step: number): [number, number] => [m.x + (m.dx * m.t) / step, m.y + (m.dy * m.t) / step];

function place(m: Mover, step: number): void {
  const [x, y] = at(m, step);
  const e = engine.entity(m.id);
  if (!e) return;
  const tr = e.get("Transform")!;
  const p = world(x, y, tr.translation.y - (THREE_D ? 0 : tr.translation.y));
  tr.translation.x = p.x;
  if (THREE_D) {
    tr.translation.z = p.z;
    tr.rotation = yawRotation(m.yaw);
  } else tr.translation.y = p.y;
  e.set("Transform", tr);
}

// Autopilot: breadth-first search to the nearest pickup (then the goal), keeping clear of
// enemies and of the tiles next to them. With no safe path it waits, or steps away.
function autopilot(s: any, m: Mover): Dir | null {
  const danger = new Set<number>();
  for (const g of s.enemies as Mover[]) {
    for (const [x, y] of [
      [g.x, g.y],
      [g.x + g.dx, g.y + g.dy],
    ]) {
      danger.add(key(x, y));
      for (const [dx, dy] of DIRS) danger.add(key(x + dx, y + dy));
    }
  }
  const targets = new Set<number>();
  if (s.pickups.length > 0) for (const p of s.pickups) targets.add(key(p.x, p.y));
  else targets.add(key(s.goal.x, s.goal.y));
  const first = search(s, m, targets, danger);
  if (first) return first;
  if (!danger.has(key(m.x, m.y))) return null;
  // In danger with no safe path: step to the open neighbour farthest from every enemy.
  let best: Dir | null = null;
  let bestD = -1;
  for (const d of DIRS) {
    const nx = m.x + d[0];
    const ny = m.y + d[1];
    if (!open(s, nx, ny)) continue;
    let near = 1e9;
    for (const g of s.enemies as Mover[]) near = Math.min(near, Math.abs(g.x - nx) + Math.abs(g.y - ny));
    if (near > bestD) {
      bestD = near;
      best = d;
    }
  }
  return best;
}

function search(s: any, m: Mover, targets: Set<number>, danger: Set<number>): Dir | null {
  const from = new Map<number, number>();
  const start = key(m.x, m.y);
  from.set(start, -1);
  const queue = [start];
  for (let i = 0; i < queue.length; i++) {
    const k = queue[i];
    if (targets.has(k) && k !== start) {
      let step = k;
      while (from.get(step) !== start) step = from.get(step)!;
      return [(step % W) - m.x, Math.floor(step / W) - m.y];
    }
    const x = k % W;
    const y = Math.floor(k / W);
    for (const [dx, dy] of DIRS) {
      const n = key(x + dx, y + dy);
      if (from.has(n) || !open(s, x + dx, y + dy) || danger.has(n)) continue;
      from.set(n, k);
      queue.push(n);
    }
  }
  return null;
}

function restart(s: any): void {
  for (const id of s.spawned) if (engine.entity(id)) engine.destroy(id);
  engine.cancel();
  setup(s);
  engine.emit("restarted");
}

export function update(self: Entity): void {
  const s = self.state;
  if (engine.justPressed("autoplay")) {
    s.auto = !s.auto;
    hud(s);
  }
  if (s.result !== "playing") {
    if (engine.justPressed("restart")) restart(s);
    return;
  }

  const want = (m: Mover): Dir | null => {
    if (s.auto) return autopilot(s, m);
    const ax = Math.sign(engine.action("move_x"));
    const ay = -Math.sign(engine.action("move_y"));
    if (ax !== 0 && open(s, m.x + ax, m.y)) return [ax, 0];
    if (ay !== 0 && open(s, m.x, m.y + ay)) return [0, ay];
    return null;
  };
  advance(s.player, CONFIG.playerStep, want);
  place(s.player, CONFIG.playerStep);

  // Enemies: at each tile center a seeded random open direction, never straight back unless
  // it is a dead end.
  for (const g of s.enemies as Mover[]) {
    advance(g, CONFIG.enemyStep, (m) => {
      const options = DIRS.filter(([dx, dy]) => tileAt(m.x + dx, m.y + dy) !== "#" && tileAt(m.x + dx, m.y + dy) !== "E");
      const forward = options.filter(([dx, dy]) => !(dx === -m.dx && dy === -m.dy));
      const pool = forward.length > 0 ? forward : options;
      return pool[Math.floor(engine.rand() * pool.length)] ?? null;
    });
    place(g, CONFIG.enemyStep);
  }

  // 3D pickups turn slowly.
  if (THREE_D) {
    s.spin = (s.spin + engine.dt * 1.5) % (Math.PI * 2);
    for (const p of s.pickups) {
      const e = engine.entity(p.id)!;
      const tr = e.get("Transform")!;
      tr.rotation = yawRotation(s.spin);
      e.set("Transform", tr);
    }
  }

  const p = s.player as Mover;
  if (p.t === 0) {
    const i = s.pickups.findIndex((q: any) => q.x === p.x && q.y === p.y);
    if (i >= 0) {
      const q = s.pickups.splice(i, 1)[0];
      engine.destroy(q.id);
      s.collected += 1;
      engine.emit("pickup", { collected: s.collected });
      if (s.pickups.length === 0) {
        const goal = engine.entity(s.goal.id)!;
        if (THREE_D) {
          goal.set("Emissive", { color: rgba(CONFIG.colors.accent), intensity: 2 });
          goal.set("PointLight", { color: rgba(CONFIG.colors.accent), intensity: 4, range: 6 });
        } else {
          const sp = goal.get("Sprite")!;
          sp.color = rgba(CONFIG.art.goal.kind === "sprite" ? [1, 1, 1] : [0.3, 0.95, 0.5]);
          goal.set("Sprite", sp);
        }
        engine.emit("goal_open");
        banner("THE WAY IS OPEN", 1.5);
      }
      hud(s);
    }
    if (p.x === s.goal.x && p.y === s.goal.y) {
      s.result = "won";
      s.wonTick = engine.tick;
      engine.emit("won", { lives: s.lives, tick: engine.tick });
      banner(CONFIG.words.win, 0);
      hud(s);
      return;
    }
  }

  if (!s.safe) {
    const [px, py] = at(p, CONFIG.playerStep);
    for (const g of s.enemies as Mover[]) {
      const [gx, gy] = at(g, CONFIG.enemyStep);
      if (Math.hypot(px - gx, py - gy) < 0.6) {
        s.lives -= 1;
        engine.emit("caught", { enemy: g.id, lives: s.lives });
        s.player = { ...p, x: s.home.x, y: s.home.y, dx: 0, dy: 0, t: 0 };
        place(s.player, CONFIG.playerStep);
        if (s.lives <= 0) {
          s.result = "lost";
          engine.emit("lost");
          banner(CONFIG.words.lose, 0);
        } else {
          s.safe = true;
          engine.after(2, endSafe);
        }
        hud(s);
        break;
      }
    }
  }
}
