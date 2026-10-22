let s = 7;
function rnd(n: number): number { s = (s * 1103515245 + 12345) % 2147483648; return s % n; }
const m = new Map<number, number>();
const ms = new Map<string, number>();
const st = new Set<number>();
let sum = 0;
for (let i = 0; i < 300000; i++) {
  const k = rnd(5000), op = rnd(10);
  if (op < 5) { m.set(k, i); ms.set("k" + k, i); st.add(k * 1024); }
  else if (op < 8) { if (m.delete(k)) sum++; ms.delete("k" + k); st.delete(k * 1024); }
  else { sum += m.get(k) ?? -1; sum += ms.has("k" + k) ? 1 : 0; sum += st.has(k * 1024) ? 1 : 0; }
  if (i % 50000 === 0) { for (const [a, b] of m) if (a % 1000 === 0) m.delete(a); }
}
let order = 0;
for (const [k, v] of m) order = (order * 31 + k + v) % 1000000007;
for (const [k, v] of ms) order = (order * 31 + k.length + v) % 1000000007;
for (const k of st) order = (order * 31 + k) % 1000000007;
console.log(m.size, ms.size, st.size, sum, order);
