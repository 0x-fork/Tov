// Arrays used as queues: shift and unshift move the start of the elements' block (room kept
// at the front) instead of every element, mixed with push, pop, splice and indexing.
let seed = 7;
function rnd(n: number): number {
  seed = (seed * 1103515245 + 12345) % 2147483648;
  return seed % n;
}
class Box { constructor(public v: number) {} }
let check = 0;
for (let round = 0; round < 40; round++) {
  const a: number[] = [];
  const s: string[] = [];
  const o: Box[] = [];
  const b: boolean[] = [];
  const steps = 200 + rnd(5000);
  for (let i = 0; i < steps; i++) {
    const op = rnd(round % 3 === 0 ? 6 : 9);
    if (op < 2) { a.unshift(i); s.unshift("s" + i); o.unshift(new Box(i)); b.unshift(i % 2 === 0); }
    else if (op < 4) { a.push(i); s.push("p" + i); o.push(new Box(-i)); b.push(i % 3 === 0); }
    else if (op < 6) { check = (check * 31 + (a.shift() ?? -1) + (s.shift()?.length ?? 0) + (o.shift()?.v ?? 7) + (b.shift() ? 1 : 0)) % 1000000007; }
    else if (op < 7) { check = (check * 17 + (a.pop() ?? -3) + (o.pop()?.v ?? 3)) % 1000000007; s.pop(); b.pop(); }
    else if (op < 8 && a.length > 0) { const k = rnd(a.length); a[k] = a[k] + 1; o[k % o.length].v += 2; }
    else if (a.length > 3) { a.splice(1, 2, 99); s.splice(0, 1); o.splice(1, 1); b.splice(2, 1, true, false); }
  }
  check = (check + a.length * 7 + s.join("").length + o.reduce((x, y) => x + y.v, 0) + b.filter((x) => x).length) % 1000000007;
  check = (check * 13 + a.reduce((x, y) => x + y, 0) + a.indexOf(5) + (a.at(-1) ?? 0)) % 1000000007;
}
console.log(check);
// a BFS queue from the front
const q: number[][] = [[0, 0]];
let seen = 0;
while (q.length > 0 && seen < 300000) { const [x, d] = q.pop()!; seen++; if (d < 17) { q.unshift([x * 2, d + 1]); q.unshift([x * 2 + 1, d + 1]); } }
console.log(seen, q.length);
const r: number[] = [];
for (let i = 0; i < 100000; i++) r.push(i);
let t = 0;
while (r.length) t += r.shift()!;
console.log(t);
