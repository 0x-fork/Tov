// Arrays that may be undefined or null (held as an array with no header), and a recursive alias for an array.
type Tree = (Tree | undefined)[];
function build(d: number): Tree {
  if (d === 0) return new Array<Tree | undefined>(2);
  const a: Tree = [];
  for (let i = 0; i < 2; i++) a.push(build(d - 1));
  return a;
}
function count(t: Tree | undefined): number {
  if (t === undefined) return 0;
  let n = 1;
  for (const c of t) n += count(c);
  return n;
}
class Box {
  items: number[] | null = null;
  names: string[] | undefined;
  add(n: number) {
    if (this.items === null) this.items = [];
    this.items.push(n);
  }
}
const t = build(3);
console.log(count(t), t.length, t[0]!.length, t[0]![0]![0]![0]);
const b = new Box();
console.log(b.items, b.names, b.items === null, b.names === undefined);
b.add(1); b.add(2);
b.names = ["x"];
console.log(b.items, b.names, JSON.stringify(b), String(b.items));
const m = new Map<string, number[] | undefined>();
m.set("a", [1]); m.set("b", undefined);
console.log(m.get("a"), m.get("b"), m.get("c"), [...m.values()]);
const xs: (number[] | null)[] = [[1], null, [2, 3]];
console.log(xs, xs.map((x) => (x ? x.length : -1)), xs[0] === xs[0], xs[1] === null);
const same = xs[0];
console.log(same === xs[0], same === xs[2]);
const c = structuredClone(xs);
c[0]!.push(9);
console.log(xs[0], c[0]);
let opt: string[] | undefined = undefined;
opt = opt ?? ["d"];
console.log(opt?.length, opt);
function f(a?: number[]): number { return a?.length ?? -1; }
console.log(f(), f([1, 2]));
