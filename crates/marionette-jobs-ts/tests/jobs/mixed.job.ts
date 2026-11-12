// Exercises integers, bitwise operators, rounding, remainders and helper calls. The tests run
// it compiled and in the script interpreter and require bit-identical output.

function hash(x: i32): i32 {
  let h = Math.imul(x, 374761393);
  h = Math.imul(h ^ (h >>> 13), 1274126177);
  return h ^ (h >> 16);
}

function clamp(v: number, lo: number, hi: number): number {
  return Math.min(Math.max(v, lo), hi);
}

export function mix(a: Float32Array, b: Float64Array, ids: Int32Array, start: i32): void {
  for (let i = 0; i < a.length; i++) {
    const id = ids[i];
    const r = (hash(id + start + i) & 1023) / 1023;
    let v = a[i] * 1.5 + b[i] / 3 - r;
    if (v % 2 > 1 || id % 7 === 3) {
      v = Math.sqrt(Math.abs(v)) + Math.round(v * 10) / 10;
    } else {
      v = Math.floor(v) + Math.trunc(-v / 4) + Math.ceil(v * 0.25);
    }
    a[i] = clamp(v, -1000, 1000);
    b[i] = b[i] * 0.999 + (id > 0 ? v : -v) + Math.fround(v) * 1e-3;
    let n = 0;
    let k = id & 31;
    while (k > 0) {
      n += k & 1;
      k = k >> 1;
    }
    ids[i] = (id * 3 + n) | 0;
  }
}
