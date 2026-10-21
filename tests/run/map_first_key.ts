class LRU<K, V> {
  private map = new Map<K, V>();
  private cap: number;
  constructor(cap: number) { this.cap = cap; }
  get(k: K): V | undefined {
    const v = this.map.get(k);
    if (v === undefined) return undefined;
    this.map.delete(k);
    this.map.set(k, v);
    return v;
  }
  set(k: K, v: V): void {
    if (this.map.has(k)) this.map.delete(k);
    else if (this.map.size >= this.cap) {
      const oldest = this.map.keys().next().value as K;
      this.map.delete(oldest);
    }
    this.map.set(k, v);
  }
  keys(): K[] {
    return Array.from(this.map.keys());
  }
}
const c = new LRU<string, number>(2);
c.set("a", 1);
c.set("b", 2);
c.get("a");
c.set("c", 3);
console.log(c.keys(), c.get("b"), c.get("a"));
const s = new Set([3, 1, 2]);
const e = new Map<string, number>();
console.log(s.values().next().value, e.keys().next().value, new Map([["x", 9]]).values().next().value);
let total = 0;
const big = new Map<number, number>();
for (let i = 0; i < 200000; i++) {
  big.set(i, i);
  if (big.size > 100) {
    const k = big.keys().next().value!;
    total += k;
    big.delete(k);
  }
}
console.log(total, big.size);
console.log([5, 6].values().next().value, [5].keys().next().value, ([] as number[]).values().next().value)
