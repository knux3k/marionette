// Every operator the job compiler accepts, one per element, chosen by position. The test
// runs this compiled and in a JavaScript engine and compares every element.

export function ints(out: Int32Array, a: Int32Array, b: Int32Array, start: i32): void {
  for (let i = 0; i < out.length; i++) {
    const x: i32 = a[i];
    const y: i32 = b[i];
    const k: i32 = (start + i) % 20;
    let r: i32 = 0;
    if (k == 0) r = x + y;
    else if (k == 1) r = x - y;
    else if (k == 2) r = Math.imul(x, y);
    else if (k == 3) r = (x % 1000) * (y % 1000);
    else if (k == 4) r = y != 0 ? x % y : 7;
    else if (k == 5) r = x | y;
    else if (k == 6) r = x ^ y;
    else if (k == 7) r = x & y;
    else if (k == 8) r = x << (y & 31);
    else if (k == 9) r = x >> (y & 31);
    else if (k == 10) r = (x >>> (y & 31)) | 0;
    else if (k == 11) r = x < y ? 1 : 0;
    else if (k == 12) r = x <= y ? 1 : 0;
    else if (k == 13) r = x > y ? 1 : 0;
    else if (k == 14) r = x >= y ? 1 : 0;
    else if (k == 15) r = x == y ? 1 : 0;
    else if (k == 16) r = x != y ? 1 : 0;
    else if (k == 17) r = -x;
    else if (k == 18) r = ~x;
    else r = y - x - 1;
    out[i] = r;
  }
}

export function nums(out: Float64Array, a: Float64Array, b: Float64Array, start: i32): void {
  for (let i = 0; i < out.length; i++) {
    const x = a[i];
    const y = b[i];
    const k: i32 = (start + i) % 20;
    let r = 0.0;
    if (k == 0) r = x + y;
    else if (k == 1) r = x - y;
    else if (k == 2) r = x * y;
    else if (k == 3) r = y != 0 ? x / y : 7;
    else if (k == 4) r = y != 0 ? x % y : 7;
    else if (k == 5) r = x < y ? 1 : 0;
    else if (k == 6) r = x <= y ? 1 : 0;
    else if (k == 7) r = x > y ? 1 : 0;
    else if (k == 8) r = x >= y ? 1 : 0;
    else if (k == 9) r = x == y ? 1 : 0;
    else if (k == 10) r = x != y ? 1 : 0;
    else if (k == 11) r = -x;
    else if (k == 12) r = Math.min(x, y);
    else if (k == 13) r = Math.max(x, y);
    else if (k == 14) r = Math.abs(x);
    else if (k == 15) r = Math.floor(x) - Math.ceil(y);
    else if (k == 16) r = Math.trunc(x);
    else if (k == 17) r = Math.sqrt(Math.abs(x));
    else if (k == 18) r = y - x;
    else r = x - y - 1;
    out[i] = r;
  }
}
