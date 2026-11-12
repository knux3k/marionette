// Written once from eval/tasks.json, not edited after running.

interface Vec2 {
  x: number;
  y: number;
}

export function move_x(vx: number, input: number, dt: number): number {
  if (input !== 0) {
    vx += 30 * input * dt;
  } else {
    const f = 20 * dt;
    if (vx > 0) vx = Math.max(0, vx - f);
    else if (vx < 0) vx = Math.min(0, vx + f);
  }
  return Math.min(6, Math.max(-6, vx));
}

export function patrol(
  points: Vec2[],
  index: number,
  pos: Vec2,
  speed: number,
  dt: number,
): { index: number; x: number; y: number } {
  const target = points[index];
  const dx = target.x - pos.x;
  const dy = target.y - pos.y;
  const dist = Math.sqrt(dx * dx + dy * dy);
  const step = speed * dt;
  if (dist <= step) {
    return { index: (index + 1) % points.length, x: target.x, y: target.y };
  }
  return { index, x: pos.x + (dx / dist) * step, y: pos.y + (dy / dist) * step };
}

interface Slot {
  item: string;
  count: number;
}

export function add_item(slots: Slot[], item: string, count: number, maxStack: number): number {
  for (const slot of slots) {
    if (count <= 0) break;
    if (slot.item === item && slot.count < maxStack) {
      const add = Math.min(count, maxStack - slot.count);
      slot.count += add;
      count -= add;
    }
  }
  for (const slot of slots) {
    if (count <= 0) break;
    if (slot.item === "" && slot.count === 0) {
      const add = Math.min(count, maxStack);
      slot.item = item;
      slot.count = add;
      count -= add;
    }
  }
  return count;
}

interface Mark {
  at: number;
  name: string;
}

export function timer_events(before: number, after: number, marks: Mark[]): string[] {
  return marks
    .filter((m) => m.at > after && m.at <= before)
    .sort((a, b) => b.at - a.at)
    .map((m) => m.name);
}

interface Other {
  id: number;
  x: number;
  y: number;
  alive: boolean;
}

export function nearest(from: Vec2, others: Other[]): number | null {
  let bestId: number | null = null;
  let bestD = Infinity;
  for (const o of others) {
    if (!o.alive) continue;
    const dx = o.x - from.x;
    const dy = o.y - from.y;
    const d = dx * dx + dy * dy;
    if (d < bestD || (d === bestD && bestId !== null && o.id < bestId)) {
      bestD = d;
      bestId = o.id;
    }
  }
  return bestId;
}

interface LootEntry {
  name: string;
  weight: number;
}

export function loot(table: LootEntry[], r: number): string {
  const total = table.reduce((s, e) => s + e.weight, 0);
  if (total === 0) return "";
  const target = r * total;
  let sum = 0;
  for (const e of table) {
    sum += e.weight;
    if (e.weight > 0 && target < sum) return e.name;
  }
  return "";
}

export function path_len(grid: string[]): number {
  const h = grid.length;
  const w = grid[0].length;
  let start: [number, number] = [0, 0];
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      if (grid[y][x] === "S") start = [x, y];
    }
  }
  const dist = new Map<number, number>();
  const queue: [number, number][] = [start];
  dist.set(start[1] * w + start[0], 0);
  for (let head = 0; head < queue.length; head++) {
    const [cx, cy] = queue[head];
    const d = dist.get(cy * w + cx)!;
    if (grid[cy][cx] === "G") return d;
    for (const [ox, oy] of [
      [1, 0],
      [-1, 0],
      [0, 1],
      [0, -1],
    ]) {
      const nx = cx + ox;
      const ny = cy + oy;
      if (nx < 0 || ny < 0 || nx >= w || ny >= h) continue;
      if (grid[ny][nx] === "#" || dist.has(ny * w + nx)) continue;
      dist.set(ny * w + nx, d + 1);
      queue.push([nx, ny]);
    }
  }
  return -1;
}

export function score_text(score: number, combo: number): string {
  let s = "Score: " + String(Math.min(score, 99999)).padStart(5, "0");
  if (combo > 1) s += " x" + combo;
  return s;
}

interface ScoreEntry {
  name: string;
  score: number;
}

export function top_scores(entries: ScoreEntry[], n: number): string[] {
  return [...entries]
    .sort((a, b) => b.score - a.score || (a.name < b.name ? -1 : a.name > b.name ? 1 : 0))
    .slice(0, n)
    .map((e) => e.name);
}

export function parse_config(text: string): Record<string, number | boolean | string> {
  const result: Record<string, number | boolean | string> = {};
  for (const segment of text.split(";")) {
    const eq = segment.indexOf("=");
    if (eq < 0) continue;
    const key = segment.slice(0, eq).trim();
    const value = segment.slice(eq + 1).trim();
    if (key === "") continue;
    if (value === "true") result[key] = true;
    else if (value === "false") result[key] = false;
    else if (/^-?\d+(\.\d+)?$/.test(value)) result[key] = Number(value);
    else result[key] = value;
  }
  return result;
}
