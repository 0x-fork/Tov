// A const read from a field or element, then that place overwritten: the const takes the
// reference it held (exits in between leave it with the place).
class N {
  next: N | null = null;
  constructor(public v: number) {}
}
class Stack {
  head: N | null = null;
  slots: (N | null)[] = [null, null];
  push(n: N): void {
    const top = this.head;
    if (n.v < 0) throw new Error("negative");
    n.next = top;
    this.head = n;
  }
  swapIn(n: N, i: number): N | null {
    const old = this.slots[i];
    if (n.v === 0) return null;
    this.slots[i] = n;
    return old;
  }
  replace(n: N): number {
    const prev = this.head;
    this.head = n;
    return prev === null ? -1 : prev.v;
  }
}
const s = new Stack();
for (let i = 1; i <= 3; i++) s.push(new N(i));
try {
  s.push(new N(-1));
} catch (e) {
  console.log((e as Error).message, s.head!.v, s.head!.next!.v);
}
console.log(s.swapIn(new N(5), 0), s.swapIn(new N(6), 0)!.v, s.swapIn(new N(0), 0), s.slots[0]!.v);
console.log(s.replace(new N(9)), s.head!.v, s.head!.next);
let n = 0;
for (let x = s.slots[0]; x !== null; x = x.next) n++;
console.log(n);
