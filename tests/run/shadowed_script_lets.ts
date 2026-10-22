let x = 1;
let y = 2;
[x, y] = [y, x];
const points = [{ x: 1, y: 20 }, { x: -3, y: 50 }];
console.log(points.map(({ x, y }) => Math.abs(x) + y));
console.log(points.map((p) => { const y = p.y; String(p.x); return y; }));
function f(y: number): number {
  Math.abs(-1);
  return y;
}
console.log(f(7), x, y);
const fnOpt: { cb?: (n: number) => number } = {};
const withCb: { cb?: (n: number) => number } = { cb: (n) => n * 2 };
console.log(fnOpt.cb?.(3) ?? "no cb", withCb.cb?.(3));
let counterVal: number | undefined;
counterVal ??= 10;
counterVal ||= 20;
counterVal &&= counterVal + 1;
const o = { n: 0, s: "" };
o.n ||= 5;
o.s ||= "dflt";
console.log(counterVal, o);
type Base = { id: number; name: string };
type Full = Base & { role: string } & { note?: string };
const full: Full = { id: 1, name: "a", role: "r" };
const user = { id: 7, name: "Zed" };
const { name, missing = "n/a" } = user as typeof user & { missing?: string };
console.log(full, name, missing);
