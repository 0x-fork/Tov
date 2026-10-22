function hex(n: number) {
  const out = [];
  for (let i = 1; i <= n; i++) out.push(i * (2 * i - 1));
  return out;
}
function words(s: string) {
  const ws = [];
  for (const w of s.split(" ")) if (w) ws.push(w.toUpperCase());
  return ws;
}
function pairs() {
  const ps = [];
  for (let i = 0; i < 3; i++) ps.push([i, i * i]);
  return ps;
}
const mixed = [];
mixed.push(1);
mixed.push("two");
console.log(hex(5), words("a bb  ccc"), pairs(), mixed, hex(0).length);
const floats = [];
floats.push(1);
floats.push(2.5);
console.log(floats.map((x) => x * 2));
function doubled(xs: number[]) {
  const out = [];
  for (let i = 0; i < xs.length; i++) out[i] = xs[i] * 2;
  return out;
}
console.log(doubled([1, 2, 3]));
for (const r of [[[1, 2], 2, 1], [[], 0, -1], [[5.5], 5.5, 0]] as const) console.log(r[0].indexOf(r[1]) === r[2], r[0].length);
