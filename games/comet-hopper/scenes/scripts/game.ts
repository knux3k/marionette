// Side-scrolling platformer (starter template `platformer`, decision 0150).
// Run and jump to the goal before time runs out. Jump on walking enemies to stomp them;
// touching one from the side, landing on spikes or falling costs a life. Pickups add score.
//
// Keys: arrows or A/D run, Space/W/Up jump, R restarts after a win or loss, F1 toggles the
// autopilot (the tests use it to prove the level can be won).
//
// The level, the art and the numbers live in the CONFIG block below; `game_make` rewrites
// only that block (from brief.json), so changes elsewhere in this file are kept.
// Everything that changes during play lives in self.state (snapshots and the state hash).
// One tile is one meter. Rows of the map go top to bottom; row r covers y = H-1-r .. H-r.

// BEGIN GAME CONFIG
const CONFIG: any = {
  "art": {
    "background": {
      "aspect": 2.0,
      "image": "../assets/generated/background.png",
      "kind": "sprite"
    },
    "enemy": {
      "aspect": 1.007,
      "image": "../assets/generated/enemy.png",
      "kind": "sprite",
      "region": [51, 55, 148, 147]
    },
    "goal": {
      "aspect": 0.671,
      "image": "../assets/generated/goal.png",
      "kind": "sprite",
      "region": [80, 54, 100, 149]
    },
    "ground": {
      "aspect": 1.0,
      "image": "../assets/generated/ground.png",
      "kind": "sprite"
    },
    "pickup": {
      "aspect": 1.034,
      "image": "../assets/generated/pickup.png",
      "kind": "sprite",
      "region": [22, 21, 211, 204]
    },
    "player": {
      "aspect": 0.695,
      "image": "../assets/generated/player.png",
      "kind": "sprite",
      "region": [56, 27, 141, 203]
    },
    "spikes": {
      "aspect": 2.084,
      "image": "../assets/generated/spikes.png",
      "kind": "sprite",
      "region": [16, 70, 223, 107]
    }
  },
  "colors": {
    "accent": [0.209, 0.807, 1.0],
    "background": [0.003, 0.004, 0.014]
  },
  "enemySpeed": 2.0,
  "gravity": 32.0,
  "jumpSpeed": 13.0,
  "lives": 3,
  "map": [
    "..............................................................................................................",
    "..............................................................................................................",
    ".....................***......................................................................................",
    ".....................===......................................................*..........*....................",
    "..............***..........*......................................................*..*.....*..................",
    "..............===........................................................*...####..............*..*...........",
    "........*.............e..........*............................*.*..*..*......###########..####...^.........G..",
    "...................#######...*.....*..*.................*..*............################..####################",
    "..P...........^....#######...............*..*..*.....*.........#########################..####################",
    "#######...################..####..######..........e....######..#########################..####################",
    "#######...################..####..###########################..#########################..####################",
    "#######...################..####..###########################..#########################..####################"
  ],
  "runSpeed": 6.5,
  "timeLimit": 86.0,
  "title": "Comet Hopper",
  "words": {
    "lose": "LOST IN SPACE",
    "pickups": "STARS",
    "win": "LIFT OFF!"
  }
};
// END GAME CONFIG

type Rgb = [number, number, number];
type Walker = { x: number; y: number; dir: number; id: number };

const MAP: string[] = CONFIG.map;
const W = MAP[0].length;
const H = MAP.length;
const HW = 0.34; // player half width
const HH = 0.45; // player half height
const EH = 0.4; // enemy half size
const VIEW_H = 12.5;
const IDENTITY = { x: 0, y: 0, z: 0, w: 1 };

const rgba = (c: Rgb, a = 1) => ({ r: c[0], g: c[1], b: c[2], a });
const tileAt = (c: number, r: number) => (c < 0 || c >= W ? "#" : r < 0 || r >= H ? "." : MAP[r][c]);
const rowOf = (y: number) => H - 1 - Math.floor(y);
const solid = (c: number, r: number) => tileAt(c, r) === "#";

function sprite(slot: string, x: number, y: number, size: number, layer: number, tint: Rgb = [1, 1, 1], flip = false): any {
  const art = CONFIG.art[slot];
  size *= art.scale ?? 1;
  const t = { translation: { x, y, z: 0 }, rotation: IDENTITY, scale: { x: flip ? -1 : 1, y: 1, z: 1 } };
  if (art.kind === "sprite") {
    const a = art.aspect;
    const e: any = {
      Transform: t,
      Sprite: { image: art.image, color: rgba(tint), size: a >= 1 ? { x: size, y: size / a } : { x: size * a, y: size }, layer },
    };
    if (art.region) e.SpriteRegion = { x: art.region[0], y: art.region[1], width: art.region[2], height: art.region[3] };
    return e;
  }
  const round = slot === "pickup" || slot === "enemy";
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
  s.enemies = [] as Walker[];
  s.score = 0;
  s.lives = CONFIG.lives;
  s.result = "playing";
  s.auto = s.auto ?? false;
  s.time = CONFIG.timeLimit;
  s.hurtUntil = 0;
  for (let r = 0; r < H; r++) {
    for (let c = 0; c < W; c++) {
      const ch = MAP[r][c];
      const x = c + 0.5;
      const y = H - 1 - r + 0.5;
      if (ch === "#") {
        // Surface tiles at full brightness, buried ones darker.
        const tint: Rgb = r > 0 && solid(c, r - 1) ? [0.7, 0.7, 0.75] : [1, 1, 1];
        spawn(s, sprite("ground", x, y, 1.0, 1, tint), "ground");
      } else if (ch === "=") {
        const e = sprite("ground", x, y + 0.3, 1.0, 1, [0.9, 0.85, 0.75]);
        e.Sprite.size = { x: 1, y: 0.4 };
        delete e.SpriteRegion;
        spawn(s, e, "platform");
      } else if (ch === "^") {
        spawn(s, sprite("spikes", x, y - 0.25, 0.9, 2), "spikes");
      } else if (ch === "*") {
        s.pickups.push({ x, y, id: spawn(s, sprite("pickup", x, y, 0.6, 3), "pickup") });
      } else if (ch === "e") {
        s.enemies.push({ x, y: y - 0.5 + EH, dir: -1, id: spawn(s, sprite("enemy", x, y - 0.5 + EH, 0.85, 4), "enemy") });
      } else if (ch === "G") {
        s.goal = { x, y, id: spawn(s, sprite("goal", x, y + 0.5, 2.0, 2), "goal") };
      } else if (ch === "P") {
        s.home = { x, y: y - 0.5 + HH };
      }
    }
  }
  s.p = { x: s.home.x, y: s.home.y, vx: 0, vy: 0, ground: false, coyote: 0, buffer: 0, face: 1, hop: false, cut: false, id: 0 };
  s.p.id = spawn(s, sprite("player", s.p.x, s.p.y, 1.0, 5), "player");
  s.safe = { x: s.home.x, y: s.home.y };
  const bg = CONFIG.art.background;
  if (bg) {
    // Wide enough for the view plus the parallax drift (follow), tall enough for the view.
    const viewW = (VIEW_H * 16) / 9;
    const a = bg.aspect ?? 2;
    const size = Math.max(viewW + W * 0.1 + 1, VIEW_H * Math.max(a, 1) * 1.02);
    s.bg = spawn(s, sprite("background", 0, H / 2, size, -1, [0.85, 0.85, 0.9]), "background");
  }
  banner(CONFIG.title, 2.5);
  follow(s);
  hud(s);
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
  ui.text = `${CONFIG.words.pickups} ${s.score}  LIVES ${s.lives}  TIME ${Math.ceil(s.time)}${auto}`;
  text.set("UiText", ui);
}

function setPos(id: number, x: number, y: number, flip?: boolean): void {
  const e = engine.entity(id);
  if (!e) return;
  const tr = e.get("Transform")!;
  tr.translation.x = x;
  tr.translation.y = y;
  if (flip !== undefined) tr.scale.x = flip ? -1 : 1;
  e.set("Transform", tr);
}

// The camera follows the player; the background moves slower (parallax).
function follow(s: any): void {
  const cam = engine.find("camera");
  if (!cam) return;
  const halfW = (VIEW_H * 16) / 9 / 2;
  const cx = Math.min(Math.max(s.p.x, halfW), W - halfW);
  setPos(cam.id, cx, H / 2);
  if (s.bg) setPos(s.bg, cx * 0.9 + W * 0.05, H / 2);
}

// Moves the player box one tick along x, then along y, stopping at solid tiles. One-way
// platforms (`=`) only hold a player falling onto them from above.
function physics(s: any, dt: number): void {
  const p = s.p;
  p.x += p.vx * dt;
  const r0 = rowOf(p.y + HH - 0.01);
  const r1 = rowOf(p.y - HH + 0.01);
  if (p.vx > 0) {
    const c = Math.floor(p.x + HW);
    for (let r = r0; r <= r1; r++) if (solid(c, r)) { p.x = c - HW - 0.001; p.vx = 0; }
  } else if (p.vx < 0) {
    const c = Math.floor(p.x - HW);
    for (let r = r0; r <= r1; r++) if (solid(c, r)) { p.x = c + 1 + HW + 0.001; p.vx = 0; }
  }
  const before = p.y - HH;
  p.y += p.vy * dt;
  p.ground = false;
  const c0 = Math.floor(p.x - HW + 0.01);
  const c1 = Math.floor(p.x + HW - 0.01);
  if (p.vy <= 0) {
    // Row r covers y = H-1-r .. H-r; ground and platforms are stood on at y = H-r.
    const r = rowOf(p.y - HH - 0.001);
    const surface = H - r;
    for (let c = c0; c <= c1; c++) {
      const t = tileAt(c, r);
      if (t === "#" || (t === "=" && before >= surface - 0.001)) {
        p.y = surface + HH;
        p.vy = 0;
        p.ground = true;
      }
    }
  } else {
    const r = rowOf(p.y + HH);
    for (let c = c0; c <= c1; c++) if (solid(c, r)) { p.y = H - 1 - r - HH - 0.001; p.vy = 0; }
  }
}

function hurt(s: any, why: string): void {
  s.lives -= 1;
  engine.emit("hurt", { why, lives: s.lives });
  s.hurtUntil = engine.tick + Math.round(2 / engine.dt);
  if (s.lives <= 0) {
    lose(s, CONFIG.words.lose);
    return;
  }
  s.p.x = s.safe.x;
  s.p.y = s.safe.y;
  s.p.vx = 0;
  s.p.vy = 0;
  hud(s);
}

function lose(s: any, text: string): void {
  s.result = "lost";
  engine.emit("lost");
  banner(text, 0);
  hud(s);
}

// Autopilot: run right, jump at gaps, walls, spikes and enemies ahead.
function autopilot(s: any): { run: number; jump: boolean } {
  const p = s.p;
  const feet = rowOf(p.y - HH + 0.05);
  const front = p.x + HW;
  let jump = false;
  if (p.ground) {
    p.hop = false;
    const ahead = Math.floor(front + 0.35);
    let floor = false;
    for (let r = feet + 1; r < H; r++) if (solid(ahead, r) || tileAt(ahead, r) === "=") floor = true;
    if (!floor) jump = true;
    for (let k = 0; k <= 1; k++) if (solid(Math.floor(front + 0.3 + k * 0.5), feet)) jump = true;
    // Spikes: a short hop just before them, so the landing stays close.
    if (!jump && tileAt(Math.floor(front + 0.45), feet) === "^") {
      jump = true;
      p.hop = true;
    }
    for (const e of s.enemies as Walker[]) {
      const dx = e.x - p.x;
      if (dx > 0 && dx < 2.6 && Math.abs(e.y - p.y) < 1) jump = true;
    }
  } else jump = p.vy > 0 && (!p.hop || p.vy > 11);
  return { run: 1, jump };
}

export function update(self: Entity): void {
  const s = self.state;
  const dt = engine.dt;
  if (engine.justPressed("autoplay")) {
    s.auto = !s.auto;
    hud(s);
  }
  if (s.result !== "playing") {
    if (engine.justPressed("restart")) {
      for (const id of s.spawned) if (engine.entity(id)) engine.destroy(id);
      engine.cancel();
      setup(s);
      engine.emit("restarted");
    }
    return;
  }

  const p = s.p;
  const ctl = s.auto
    ? autopilot(s)
    : { run: Math.sign(engine.action("move_x")), jump: engine.pressed("jump") };
  const jumpNow = s.auto ? ctl.jump && p.ground : engine.justPressed("jump");

  // Run with a little acceleration; jump with coyote time and a short input buffer.
  const target = ctl.run * CONFIG.runSpeed;
  const accel = (p.ground ? 60 : 30) * dt;
  p.vx = p.vx < target ? Math.min(p.vx + accel, target) : Math.max(p.vx - accel, target);
  if (ctl.run !== 0) p.face = ctl.run;
  p.coyote = p.ground ? 6 : Math.max(p.coyote - 1, 0);
  p.buffer = jumpNow ? 6 : Math.max(p.buffer - 1, 0);
  if (p.buffer > 0 && p.coyote > 0) {
    p.vy = CONFIG.jumpSpeed;
    p.buffer = 0;
    p.coyote = 0;
    p.cut = false;
    engine.emit("jump");
  }
  // Letting go of jump while rising cuts the jump short (once per jump).
  if (!ctl.jump && p.vy > 0 && !p.cut) {
    p.vy *= 0.55;
    p.cut = true;
  }
  p.vy = Math.max(p.vy - CONFIG.gravity * dt, -18);
  physics(s, dt);
  setPos(p.id, p.x, p.y, p.face < 0);

  // A safe place to come back to after a hit: solid ground, no hazard close by.
  if (p.ground) {
    const r = rowOf(p.y - HH - 0.1);
    let clear = true;
    for (let dc = -2; dc <= 2; dc++) if (tileAt(Math.floor(p.x) + dc, r - 1) === "^") clear = false;
    for (const e of s.enemies as Walker[]) if (Math.abs(e.x - p.x) < 4) clear = false;
    // Two tiles of ground ahead, so a restart never begins at an edge.
    const c = Math.floor(p.x);
    if (clear && solid(c - 1, r) && solid(c, r) && solid(c + 1, r) && solid(c + 2, r)) s.safe = { x: p.x, y: p.y };
  }

  // Enemies walk until a wall or an edge, then turn.
  for (const e of s.enemies as Walker[]) {
    const nx = e.x + e.dir * CONFIG.enemySpeed * dt;
    const edge = Math.floor(nx + e.dir * EH);
    const r = rowOf(e.y);
    if (solid(edge, r) || !solid(edge, r + 1) || tileAt(edge, r) === "^") e.dir = -e.dir;
    else e.x = nx;
    setPos(e.id, e.x, e.y, e.dir > 0);
  }

  const hurtable = engine.tick >= s.hurtUntil;
  for (let i = s.enemies.length - 1; i >= 0; i--) {
    const e = s.enemies[i] as Walker;
    if (Math.abs(e.x - p.x) < HW + EH && Math.abs(e.y - p.y) < HH + EH) {
      if (p.vy < 0 && p.y - HH > e.y - 0.1) {
        engine.destroy(e.id);
        s.enemies.splice(i, 1);
        p.vy = CONFIG.jumpSpeed * 0.6;
        s.score += 1;
        engine.emit("stomp", { score: s.score });
        hud(s);
      } else if (hurtable) {
        hurt(s, "enemy");
        return;
      }
    }
  }
  // Spikes fill the lower half of their tile (as drawn).
  const feetRow = rowOf(p.y - HH + 0.05);
  const onSpikes = tileAt(Math.floor(p.x), feetRow) === "^" && p.y - HH < H - 1 - feetRow + 0.5;
  if (hurtable && (onSpikes || p.y < -1.5)) {
    hurt(s, p.y < -1.5 ? "fall" : "spikes");
    return;
  }

  for (let i = s.pickups.length - 1; i >= 0; i--) {
    const q = s.pickups[i];
    if (Math.abs(q.x - p.x) < 0.6 && Math.abs(q.y - p.y) < 0.7) {
      engine.destroy(q.id);
      s.pickups.splice(i, 1);
      s.score += 1;
      engine.emit("pickup", { score: s.score });
      hud(s);
    }
  }

  if (Math.abs(s.goal.x - p.x) < 0.6 && p.y < s.goal.y + 1.5 && p.y > s.goal.y - 1) {
    s.result = "won";
    s.wonTick = engine.tick;
    engine.emit("won", { score: s.score, lives: s.lives, tick: engine.tick });
    banner(CONFIG.words.win, 0);
    hud(s);
    return;
  }

  const before = Math.ceil(s.time);
  s.time = Math.max(s.time - dt, 0);
  if (Math.ceil(s.time) !== before) hud(s);
  if (s.time <= 0) {
    lose(s, "TIME UP");
    return;
  }
  follow(s);
}
