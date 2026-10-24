// An object reached through a field and passed to a call: kept alive for a callee that can
// drop it (storing to a field that can hold it), borrowed for one that only stores others.
class Leaf {
  constructor(public v: number) {}
}
class Note {
  constructor(public text: string) {}
}
class Holder {
  leaf: Leaf | null = new Leaf(7);
  note: Note | null = null;
  // drops the holder's leaf, the object it's given
  dropAndRead(l: Leaf): number {
    this.leaf = null;
    return l.v + 1;
  }
  // stores only notes: the leaf can't go
  annotate(l: Leaf): number {
    this.note = new Note(`leaf ${l.v}`);
    return l.v * 2;
  }
}
const h = new Holder();
console.log(h.annotate(h.leaf!), h.note!.text);
console.log(h.dropAndRead(h.leaf!), h.leaf);
