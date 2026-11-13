// A swarm circling a moving attractor (examples/swarm.scene.json, decision 0070).
// `seed` is called once by the swarm's script; `step` runs every tick from the entity's
// JobSchedule; `energy` is summed by the script once a second.
type i32 = number;

const PULL = 6.0;
const SWIRL = 3.2;
const DRAG = 0.7;

// Integer hash (Math.imul wraps like i32), for a seeded start without Math.random.
function hash(x: i32): i32 {
  let h = Math.imul(x ^ (x >> 16), 73244475);
  h = Math.imul(h ^ (h >> 16), 73244475);
  return h ^ (h >> 16);
}

// [0, 1) from an index and a salt.
function unit(i: i32, salt: i32): number {
  return (hash(i * 4 + salt) & 16777215) / 16777216;
}

export function seed(
  px: Float32Array, py: Float32Array, pz: Float32Array,
  vx: Float32Array, vy: Float32Array, vz: Float32Array,
  first: i32, salt: i32,
): void {
  for (let i = 0; i < px.length; i++) {
    const k = (first + i) * 8 + salt;
    const a = unit(k, 0) * Math.PI * 2;
    const r = 2.0 + unit(k, 1) * 2.0;
    px[i] = Math.cos(a) * r;
    pz[i] = Math.sin(a) * r;
    py[i] = 0.5 + unit(k, 2) * 3.0;
    vx[i] = -Math.sin(a) * 1.5;
    vz[i] = Math.cos(a) * 1.5;
    vy[i] = 0;
  }
}

export function step(
  px: Float32Array, py: Float32Array, pz: Float32Array,
  vx: Float32Array, vy: Float32Array, vz: Float32Array,
  dt: number, tick: number, first: i32,
): void {
  const t = tick * dt;
  const ax = Math.cos(t * 0.6) * 1.5;
  const ay = 2.0 + Math.sin(t * 0.9) * 0.8;
  const az = Math.sin(t * 0.6) * 1.5;
  for (let i = 0; i < px.length; i++) {
    const dx = ax - px[i];
    const dy = ay - py[i];
    const dz = az - pz[i];
    // A pull of constant strength toward the attractor, a swirl around its vertical axis,
    // (stronger for some particles than others) and drag: the swarm settles into a thick
    // spinning disc a few meters across.
    const inv = 1.0 / (Math.sqrt(dx * dx + dy * dy + dz * dz) + 0.3);
    const swirl = SWIRL * inv * (0.5 + unit(first + i, 3));
    const x = vx[i] + (dx * PULL * inv - dz * swirl - DRAG * vx[i]) * dt;
    let y = vy[i] + (dy * PULL * inv * 0.3 - DRAG * vy[i]) * dt;
    const z = vz[i] + (dz * PULL * inv + dx * swirl - DRAG * vz[i]) * dt;
    let h = py[i] + y * dt;
    if (h < 0.05) {
      h = 0.05;
      y = -y * 0.5;
    }
    px[i] = px[i] + x * dt;
    py[i] = h;
    pz[i] = pz[i] + z * dt;
    vx[i] = x;
    vy[i] = y;
    vz[i] = z;
  }
}

export function energy(vx: Float32Array, vy: Float32Array, vz: Float32Array): number {
  let e = 0.0;
  for (let i = 0; i < vx.length; i++) {
    e += 0.5 * (vx[i] * vx[i] + vy[i] * vy[i] + vz[i] * vz[i]);
  }
  return e;
}
