class P { constructor(public id: number, public name: string) {} }
function build(n: number): P[] {
  const out: P[] = new Array(n);
  const unrelated = n * 2;
  for (let i = 0; i < n; i++) {
    const k = i * unrelated;
    out[i] = new P(k, `p${i}`);
  }
  return out;
}
const ps = build(5);
console.log(ps.length, ps.map((p) => p.name).join(","), ps[4].id);
type R = { a: string; b: number[] };
function recs(n: number): R[] {
  const rs = new Array<R>(n);
  for (let i = 0; i < n; ++i) rs[i] = { a: "x" + i, b: [i] };
  return rs;
}
console.log(recs(3));
