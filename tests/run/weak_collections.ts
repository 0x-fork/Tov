// WeakMap / WeakSet (by identity), the `object` type, and JSON of a Map of other keys.
class Node2 { constructor(public id: number) {} }
const meta = new WeakMap<Node2, string>();
const na = new Node2(1), b = new Node2(2);
meta.set(na, "first");
meta.set(b, "second");
console.log(meta.get(na), meta.has(b), meta.get(new Node2(1)), meta.delete(na), meta.has(na));
const seen = new WeakSet<{ x: number }>();
const o1 = { x: 1 };
seen.add(o1);
console.log(seen.has(o1), seen.has({ x: 1 }));
const cache: WeakMap<Node2, number> = new WeakMap();
cache.set(na, 5);
console.log(cache.get(na));
const m = new Map<number, string>([[1, "a"]]);
console.log(JSON.stringify(m), JSON.stringify({ m, n: 1 }));
