class Item { constructor(public v: number, public next: Item | null = null) {} }
class Stack {
  head: Item | null = null;
  count = 0;
  push(v: number): void { this.head = new Item(v, this.head); this.count++; }
  clear(): void { this.head = null; }
  topAfter(log: (s: string) => void): number {
    if (this.head) {
      log("top");
      return this.head.v;
    }
    return -1;
  }
  sum(): number {
    let s = 0;
    let cur = this.head;
    while (cur) { s += cur.v; cur = cur.next; }
    return s;
  }
}
const st = new Stack();
for (let i = 1; i <= 5; i++) st.push(i * 10);
const seen: string[] = [];
console.log(st.topAfter((s) => { seen.push(s); }), seen.join(","), st.sum(), st.count);
console.log(st.topAfter(() => st.clear()));
