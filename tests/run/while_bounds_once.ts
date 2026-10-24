// `while (k <= n) { xs[k - 1] = ...; k += s }`: one test before the loop (s >= 0, every index
// it can reach in range) picks a copy without per-store checks; otherwise the checked loop.
function sieve(flags: boolean[], size: number): number {
  let count = 0;
  for (let i = 2; i <= size; i += 1) {
    if (flags[i - 1]) {
      count += 1;
      let k = i + i;
      while (k <= size) {
        flags[k - 1] = false;
        k += i;
      }
    }
  }
  return count;
}
const flags = new Array<boolean>(1000).fill(true);
console.log(sieve(flags, 1000));
// a bound past the end: the checked loop, which grows the array
const short = new Array<number>(3).fill(0);
let k = 1;
while (k <= 6) {
  short[k - 1] = k * 10;
  k += 1;
}
console.log(short.join(","), short.length);
// a step that may go down: the checked loop
function fill(xs: number[], start: number, step: number, n: number): string {
  let j = start;
  let rounds = 0;
  while (j < n) {
    if (j < 1 || rounds > 8) break;
    xs[j - 1] = rounds;
    rounds += 1;
    j += step;
  }
  return xs.join(",");
}
console.log(fill(new Array<number>(5).fill(-1), 2, 1, 5), fill(new Array<number>(5).fill(-1), 4, -1, 5), fill(new Array<number>(5).fill(-1), 1, 0, 5));
// starting below the first index: checked
const ys = new Array<number>(4).fill(0);
let q = 1;
while (q < 4) {
  ys[q] += q;
  ys[q - 1] += 1;
  q += 2;
}
console.log(ys.join(","));
