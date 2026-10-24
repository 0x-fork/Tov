// Values reached through fields, passed to calls and walked by locals: kept alive while a
// callee replaces the fields they came from.
class Node {
  constructor(public value: number, public next: Node | null = null, public left: Node | null = null) {}
}
class List {
  head: Node | null = null;
  push(v: number): void {
    this.head = new Node(v, this.head);
  }
  // replaces the field its argument came from
  dropHead(n: Node): number {
    this.head = null;
    return n.value + (n.next ? n.next.value : 0);
  }
  length(): number {
    let n = 0;
    let cur = this.head;
    while (cur !== null) {
      n += 1;
      cur = cur.next;
    }
    return n;
  }
  // walks while changing the list: the walk can't borrow
  dropWhileWalking(): number {
    let cur = this.head;
    let sum = 0;
    while (cur !== null) {
      sum += cur.value;
      this.head = cur.next;
      cur = cur.next;
    }
    return sum;
  }
}
function rotate(x: Node): Node {
  const y = x.left!;
  x.left = y.next;
  y.next = x;
  return y;
}
const l = new List();
for (let i = 1; i <= 5; i++) l.push(i);
console.log(l.length());
console.log(l.dropHead(l.head!), l.head === null);
for (let i = 1; i <= 4; i++) l.push(i * 10);
console.log(l.length(), l.dropWhileWalking(), l.length());
const root = new Node(1, null, new Node(2, null, null));
const r = rotate(root.left === null ? root : root);
console.log(r.value, r.next!.value);
