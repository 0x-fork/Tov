class P { constructor(public n: number) {} }
function describe(v: unknown): string {
  if (typeof v === "string") return "str " + v.toUpperCase();
  if (typeof v === "number") return "num " + (v * 2);
  if (typeof v === "boolean") return "bool " + !v;
  if (v === undefined) return "undef";
  if (v === null) return "null";
  if (v instanceof P) return "P " + v.n;
  return "other";
}
const vals: unknown[] = [1, 2.5, "hi", true, undefined, null, new P(3), [1, 2]];
for (const v of vals) console.log(describe(v), typeof v, v ? "t" : "f");
console.log(vals);
console.log(JSON.stringify(vals));
const a: unknown = 3;
const b: unknown = 3.0;
console.log(a === b, a === 3, a !== "3", `${a}!`, String(vals[2]));
const m = new Map<string, unknown>();
m.set("x", 1); m.set("y", "two");
console.log(m, m.get("y"));
let u: unknown;
console.log(u, u ?? "dflt");
function f2(v: unknown): string {
  if (typeof v === "number" || typeof v === "string") return `ns ${v}`;
  return "no";
}
console.log(f2(1), f2("a"), f2(true));
function g2(x: unknown) { return x; }
console.log(g2(4), g2([1]), g2({ a: 1 }));
const xs: unknown[] = [];
xs.push(1, "b");
console.log(xs.length, xs.includes("b"), xs.indexOf(1));
let e: unknown = "s";
e = 7;
console.log(e);
