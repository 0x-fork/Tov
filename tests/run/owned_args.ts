// Arguments a method stores are handed over (also when it throws first), and `p = a.f; a.f = null`
// moves the reference.
class Disk {
  next: Disk | null = null;
  constructor(public size: number) {}
}
class Piles {
  piles: (Disk | null)[] = [null, null, null];
  push(d: Disk, at: number): void {
    const top = this.piles[at];
    if (top && d.size >= top.size) throw new Error(`too big: ${d.size}`);
    d.next = top;
    this.piles[at] = d;
  }
  pop(at: number): Disk {
    const top = this.piles[at];
    if (top === null) throw new Error("empty");
    this.piles[at] = top.next;
    top.next = null;
    return top;
  }
  count(at: number): number {
    let n = 0;
    for (let d = this.piles[at]; d !== null; d = d.next) n++;
    return n;
  }
}
const p = new Piles();
for (let s = 5; s >= 1; s--) p.push(new Disk(s), 0);
let moved = 0;
function solve(n: number, from: number, to: number): void {
  if (n === 0) return;
  const other = 3 - from - to;
  solve(n - 1, from, other);
  p.push(p.pop(from), to);
  moved++;
  solve(n - 1, other, to);
}
solve(5, 0, 1);
console.log(moved, p.count(0), p.count(1), p.count(2));
const big = new Disk(99);
try {
  p.push(big, 1);
} catch (e) {
  console.log((e as Error).message, big.size, big.next === null);
}
try {
  p.pop(0);
} catch (e) {
  console.log((e as Error).message);
}
console.log(p.pop(1).size, p.count(1));
