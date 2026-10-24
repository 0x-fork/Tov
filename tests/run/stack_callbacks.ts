// Callbacks a function or method only calls (or hands on) live on the caller's stack, and a
// local handed over on its last use moves.
class Bag<E> {
  items: E[] = [];
  forEach(fn: (e: E) => void): void {
    for (const x of this.items) fn(x);
  }
  some(fn: (e: E) => boolean): boolean {
    return this.any(fn);
  }
  any(fn: (e: E) => boolean): boolean {
    for (const x of this.items) if (fn(x)) return true;
    return false;
  }
  keep(fn: (e: E) => boolean): (e: E) => boolean {
    return fn;
  }
}
class Node {
  next: Node | null = null;
  constructor(public v: number) {}
}
function link(a: Node, n: number): Node {
  const b = new Node(n);
  b.next = a.next;
  a.next = b;
  return b;
}
const bag = new Bag<number>();
bag.items.push(1, 2, 3);
let total = 0;
bag.forEach((x) => { total += x; });
const want = 2;
console.log(total, bag.some((x) => x === want), bag.some((x) => x > 5));
const kept = bag.keep((x) => x === total);
console.log(kept(6), kept(1));
const head = new Node(0);
link(head, 1);
link(head, 2);
console.log(head.next!.v, head.next!.next!.v);
