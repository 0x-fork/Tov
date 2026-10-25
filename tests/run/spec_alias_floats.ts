// `number` taken as an integer where only integers flow, and a float where one does: through a
// type alias, an `as number` of a union holding one, and `Array(n).fill(x)` into an annotation.
type Val = number | string | Val[] | null;
function atom(t: string): Val {
  const n = Number(t);
  return Number.isNaN(n) ? t : n;
}
const vals: Val[] = ["1", "2.5", "x", "-3"].map(atom);
let sum = 0;
for (const v of vals) if (typeof v === "number") sum += v as number;
console.log(vals, sum);
function half(v: Val): number {
  return (v as number) / 2;
}
console.log(half(atom("7")), half(atom("0.5")));

function fill(n: number): number {
  const m: number[] = Array(n + 1).fill(Infinity);
  const last: number[] = Array(n + 1).fill(-1);
  m[0] = 0;
  for (const c of [1, 3, 4]) {
    for (let j = c; j <= n; j++) {
      if (m[j] > m[j - c] + 1) {
        m[j] = m[j - c] + 1;
        last[j] = c;
      }
    }
  }
  return m[n] + last[n] / 10;
}
console.log(fill(6), fill(13));
const xs: number[] = Array(3).fill(0.25);
console.log(xs, xs.reduce((a, b) => a + b, 0));
