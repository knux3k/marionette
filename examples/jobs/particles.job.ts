// Particles: gravity, drag and a bouncy floor. Used by the determinism tests and the benchmark.

const GRAVITY = -9.81;
const BOUNCE = 0.8;

function drag(v: number, k: number): number {
  return v - v * Math.abs(v) * k;
}

export function step(
  px: Float32Array, py: Float32Array, pz: Float32Array,
  vx: Float32Array, vy: Float32Array, vz: Float32Array,
  dt: number, k: number,
): void {
  for (let i = 0; i < px.length; i++) {
    let x = vx[i];
    let y = vy[i] + GRAVITY * dt;
    let z = vz[i];
    x = drag(x, k);
    y = drag(y, k);
    z = drag(z, k);
    let h = py[i] + y * dt;
    if (h < 0) {
      h = -h;
      y = -y * BOUNCE;
    }
    px[i] = px[i] + x * dt;
    py[i] = h;
    pz[i] = pz[i] + z * dt;
    vx[i] = x;
    vy[i] = y;
    vz[i] = z;
  }
}

// Kinetic energy of a chunk (an ordered reduction: the engine adds chunk results in order).
export function energy(vx: Float32Array, vy: Float32Array, vz: Float32Array): number {
  let e = 0;
  for (let i = 0; i < vx.length; i++) {
    e += 0.5 * (vx[i] * vx[i] + vy[i] * vy[i] + vz[i] * vz[i]);
  }
  return e;
}
