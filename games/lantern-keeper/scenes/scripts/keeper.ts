// Lantern Keeper: a top-down 2D maze game (proof scene, decision 0050).
// Light every lantern by walking onto it; the gate then opens and reaching it wins.
// Ghosts wander the corridors; touching one costs a life and sends you back to the start.
//
// The maze is a text map. This script spawns one sprite per wall and lantern at start
// (engine.spawn), moves the player and the ghosts tile by tile on the fixed tick, releases
// ghosts with timers (engine.after), and writes the score into the UI text.
// Everything that changes lives in self.state, so snapshots and the state hash cover it.

const MAP = [
  "#################",
  "#P....#.....#..L#",
  "#.##.##.###.#.#.#",
  "#.#L..........#.#",
  "#.#.###.#.###.#.#",
  "#.....#G#.#L....#",
  "###.#.#...#.###.#",
  "#L..#...#...#...#",
  "#.###.#####.#.#.#",
  "#.....G...L...#E#",
  "#################",
];
const W = MAP[0].length;
const H = MAP.length;
const PLAYER_STEP = 8; // ticks per tile
const GHOST_STEP = 12;
const LIVES = 3;
const DIRS: Dir[] = [
  [1, 0],
  [-1, 0],
  [0, 1],
  [0, -1],
];

type Dir = [number, number];
type Mover = { x: number; y: number; dx: number; dy: number; t: number; id: number };

const world = (x: number, y: number) => ({ x: x - (W - 1) / 2, y: (H - 1) / 2 - y });
const color = (r: number, g: number, b: number, a = 1) => ({ r, g, b, a });
const identity = { x: 0, y: 0, z: 0, w: 1 };
const one = { x: 1, y: 1, z: 1 };

function tileAt(x: number, y: number): string {
  return y < 0 || y >= H || x < 0 || x >= W ? "#" : MAP[y][x];
}

function sprite(x: number, y: number, image: string, c: ReturnType<typeof color>, size: number, layer: number) {
  return {
    Transform: { translation: { ...world(x, y), z: 0 }, rotation: identity, scale: one },
    Sprite: { image, color: c, size: { x: size, y: size }, layer },
  };
}

export function start(self: Entity): void {
  const s = self.state;
  s.lanterns = [] as { x: number; y: number; id: number; lit: boolean }[];
  s.ghosts = [] as Mover[];
  s.lit = 0;
  s.lives = LIVES;
  s.result = "playing";
  s.safe = false;
  for (let y = 0; y < H; y++) {
    for (let x = 0; x < W; x++) {
      const c = MAP[y][x];
      if (c === "#") engine.spawn(sprite(x, y, "builtin:white", color(0.05, 0.07, 0.16), 1, 1), "wall");
      if (c === "L") {
        const e = engine.spawn(sprite(x, y, "builtin:circle", color(0.35, 0.3, 0.2), 0.5, 2), "lantern");
        s.lanterns.push({ x, y, id: e.id, lit: false });
      }
      if (c === "E") s.gate = { x, y, id: engine.spawn(sprite(x, y, "builtin:white", color(0.5, 0.1, 0.1), 0.9, 2), "gate").id };
      if (c === "P") {
        s.home = { x, y };
        const p = engine.find("player")!;
        s.player = { x, y, dx: 0, dy: 0, t: 0, id: p.id } as Mover;
      }
      if (c === "G") s.ghostHomes = [...(s.ghostHomes ?? []), { x, y }];
    }
  }
  // Ghosts enter one after another.
  s.ghostHomes.forEach((_: unknown, i: number) => engine.after(1 + i * 2, releaseGhost, i));
  engine.every(0.25, flicker);
  place(s.player);
  hud(s);
}

export function releaseGhost(self: Entity, i: number): void {
  const home = self.state.ghostHomes[i];
  const e = engine.spawn(sprite(home.x, home.y, "builtin:circle", color(0.75, 0.85, 1.0, 0.85), 0.8, 3), "ghost");
  self.state.ghosts.push({ x: home.x, y: home.y, dx: 0, dy: 0, t: 0, id: e.id });
  engine.emit("ghost_released", { ghost: e.id });
}

// Lit lanterns flicker: a seeded random warm color each quarter second.
export function flicker(self: Entity): void {
  for (const l of self.state.lanterns) {
    if (!l.lit) continue;
    const k = 0.8 + engine.rand() * 0.2;
    engine.entity(l.id)!.set("Sprite", {
      image: "builtin:circle",
      color: color(1.0 * k, 0.75 * k, 0.25 * k),
      size: { x: 0.7, y: 0.7 },
      layer: 2,
    });
  }
}

export function endSafe(self: Entity): void {
  self.state.safe = false;
}

function open(s: any, x: number, y: number): boolean {
  const c = tileAt(x, y);
  if (c === "#") return false;
  if (c === "E") return s.lit === s.lanterns.length;
  return true;
}

// Advances a mover by one tick. At a tile center `choose` picks the next direction.
function advance(s: any, m: Mover, step: number, choose: (m: Mover) => Dir | null): void {
  if (m.t === 0) {
    const d = choose(m);
    if (!d) {
      m.dx = 0;
      m.dy = 0;
      return;
    }
    m.dx = d[0];
    m.dy = d[1];
  }
  m.t += 1;
  if (m.t >= step) {
    m.x += m.dx;
    m.y += m.dy;
    m.t = 0;
  }
}

function place(m: Mover, step = PLAYER_STEP): void {
  const p = world(m.x + (m.dx * m.t) / step, m.y + (m.dy * m.t) / step);
  const e = engine.entity(m.id)!;
  const tr = e.get("Transform")!;
  tr.translation.x = p.x;
  tr.translation.y = p.y;
  e.set("Transform", tr);
}

function hud(s: any): void {
  const text = engine.find("hud");
  if (!text) return;
  const ui = text.get("UiText")!;
  const status = s.result === "won" ? "  THE GATE IS YOURS" : s.result === "lost" ? "  GAME OVER" : "";
  ui.text = `LANTERNS ${s.lit}/${s.lanterns.length}  LIVES ${s.lives}${status}`;
  text.set("UiText", ui);
}

export function update(self: Entity): void {
  const s = self.state;
  if (s.result !== "playing") return;

  // Player: holds a direction; turns only at tile centers, stops at walls.
  const want = (m: Mover): Dir | null => {
    const ax = Math.sign(engine.action("move_x"));
    const ay = -Math.sign(engine.action("move_y"));
    if (ax !== 0 && open(s, m.x + ax, m.y)) return [ax, 0];
    if (ay !== 0 && open(s, m.x, m.y + ay)) return [0, ay];
    return null;
  };
  advance(s, s.player, PLAYER_STEP, want);
  place(s.player);

  // Ghosts: at each center, a seeded random open direction, never straight back unless stuck.
  for (const g of s.ghosts as Mover[]) {
    advance(s, g, GHOST_STEP, (m) => {
      const options = DIRS.filter(([dx, dy]) => tileAt(m.x + dx, m.y + dy) === "." || "LPG".includes(tileAt(m.x + dx, m.y + dy)));
      const forward = options.filter(([dx, dy]) => !(dx === -m.dx && dy === -m.dy));
      const pool = forward.length > 0 ? forward : options;
      return pool[Math.floor(engine.rand() * pool.length)] ?? null;
    });
    place(g, GHOST_STEP);
  }

  const p = s.player as Mover;
  if (p.t === 0) {
    for (const l of s.lanterns) {
      if (!l.lit && l.x === p.x && l.y === p.y) {
        l.lit = true;
        s.lit += 1;
        engine.emit("lantern_lit", { lantern: l.id, lit: s.lit });
        if (s.lit === s.lanterns.length) {
          engine.entity(s.gate.id)!.set("Sprite", {
            image: "builtin:white",
            color: color(0.3, 0.9, 0.5),
            size: { x: 0.9, y: 0.9 },
            layer: 2,
          });
          engine.emit("gate_open");
        }
        hud(s);
      }
    }
    if (p.x === s.gate.x && p.y === s.gate.y) {
      s.result = "won";
      engine.emit("won", { lives: s.lives, tick: engine.tick });
      hud(s);
      return;
    }
  }

  // Ghost contact: positions closer than 0.6 tiles.
  if (!s.safe) {
    const at = (m: Mover, step: number) => [m.x + (m.dx * m.t) / step, m.y + (m.dy * m.t) / step];
    const [px, py] = at(p, PLAYER_STEP);
    for (const g of s.ghosts as Mover[]) {
      const [gx, gy] = at(g, GHOST_STEP);
      if (Math.hypot(px - gx, py - gy) < 0.6) {
        s.lives -= 1;
        engine.emit("caught", { ghost: g.id, lives: s.lives });
        s.player = { ...p, x: s.home.x, y: s.home.y, dx: 0, dy: 0, t: 0 };
        place(s.player);
        if (s.lives <= 0) {
          s.result = "lost";
          engine.emit("lost");
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
