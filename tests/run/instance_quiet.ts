// A method called on a type parameter's value is the instance's class's: in each instance of a
// generic body, an object borrowed for the call is kept alive only where that method can drop it.
class Item {
  constructor(public v: number) {}
}
interface Probe {
  probe(it: Item): number;
}
class Holder<P extends Probe> {
  item: Item | null = new Item(5);
  constructor(public p: P) {}
  run(): number {
    return this.p.probe(this.item!);
  }
  walk(): number {
    // a cursor walk whose comparisons are the instance's method
    let total = 0;
    for (let cur: Node | null = chain; cur; cur = cur.next) total += this.p.probe(cur.item);
    return total;
  }
}
class Node {
  next: Node | null = null;
  constructor(public item: Item) {}
}
let chain: Node | null = null;
for (let i = 0; i < 5; i++) {
  const n = new Node(new Item(i));
  n.next = chain;
  chain = n;
}
class Reader implements Probe {
  probe(it: Item): number {
    return it.v * 2;
  }
}
class Dropper implements Probe {
  h: Holder<Dropper> | null = null;
  dropItem = false;
  probe(it: Item): number {
    // drops the item it's given, or the list it's in: it lives on until the call returns
    if (this.h && this.dropItem) this.h.item = null;
    else chain = null;
    return it.v + 100;
  }
}
const a = new Holder(new Reader());
console.log(a.run(), a.walk(), a.item?.v);
const d = new Dropper();
const b = new Holder(d);
d.h = b;
console.log(b.walk(), chain);
d.dropItem = true;
console.log(b.run(), b.item);
