class N { constructor(public v: number, public next: N | null = null) {} }
const a = new N(1, new N(2));
let cur: N | null = a;
let s = 0;
while (cur) { s += cur.v; cur = cur.next; }
console.log(s, a.next?.next, a.next!.next === null, `${a.next!.next}`, String(null), JSON.stringify({ n: a.next!.next }));
const m: (N | null)[] = [null, a];
console.log(m.map((x) => (x === null ? "none" : x.v)), m[0]);
class P { child: C | null = null; constructor(public name: string) { this.child = new C(this); } }
class C { constructor(public parent: P) {} }
let n = 0;
for (let i = 0; i < 3; i++) { const p = new P("p" + i); n += p.child!.parent.name.length; }
console.log(n);
class MNode {
  hash: number | undefined = undefined;
  constructor(public value: number | undefined, public left: MNode | null, public right: MNode | null) {}
  check(): boolean {
    if (this.hash === undefined) return false;
    if (this.value !== undefined) return true;
    return this.left!.check() && this.right!.check();
  }
  calHash(): void {
    if (this.hash !== undefined) return;
    if (this.value !== undefined) { this.hash = this.value; return; }
    this.left!.calHash();
    this.right!.calHash();
    this.hash = this.left!.hash! + this.right!.hash!;
  }
}
function make(d: number): MNode {
  return d === 0 ? new MNode(1, null, null) : new MNode(undefined, make(d - 1), make(d - 1));
}
const maxDepth = 8;
const stretch = make(maxDepth + 1);
stretch.calHash();
console.log(`stretch tree of depth ${maxDepth + 1}\t root hash: ${stretch.hash} check: ${stretch.check()}`);
const long = make(maxDepth);
for (let d = 4; d <= maxDepth; d += 2) {
  const iters = 1 << (maxDepth - d + 4);
  let sum = 0;
  for (let i = 0; i < iters; i++) { const t = make(d); t.calHash(); sum += t.hash!; }
  console.log(`${iters}\t trees of depth ${d}\t root hash sum: ${sum}`);
}
long.calHash();
console.log(`long lived tree of depth ${maxDepth}\t root hash: ${long.hash} check: ${long.check()}`);
