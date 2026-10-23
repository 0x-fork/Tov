// Assigning one field in a branch keeps what's known of the others: `this.tail = undefined`
// can't change `this.head`.

class Node2 {
  next: Node2 | undefined = undefined;
  prev: Node2 | undefined = undefined;
  constructor(public value: number) {}
}

class List {
  head: Node2 | undefined = undefined;
  tail: Node2 | undefined = undefined;
  push(v: number): void {
    const n = new Node2(v);
    if (!this.tail) {
      this.head = n;
    } else {
      this.tail.next = n;
      n.prev = this.tail;
    }
    this.tail = n;
  }
  pop(): number {
    if (!this.head) throw new Error("empty");
    const removed = this.head;
    if (this.head === this.tail) {
      this.tail = undefined;
    } else {
      this.head.next!.prev = undefined;
    }
    this.head = this.head.next;
    return removed.value;
  }
}

const l = new List();
l.push(1);
l.push(2);
console.log(l.pop(), l.pop(), l.head === undefined, l.tail === undefined);
