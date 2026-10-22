const counts = new Map<string, number>();
for (const w of "a b a c b a".split(" ")) counts.set(w, (counts.get(w) ?? 0) + 1);
console.log([...counts]);
class Acc { totals = new Map<string, number>(); add(k: string, v: number): void { this.totals.set(k, (this.totals.get(k) ?? 0) + v); } }
const acc = new Acc();
acc.add("x", 1.5); acc.add("y", 2); acc.add("x", 3);
console.log([...acc.totals]);
type R = { products: Map<string, number>; name: string };
const r: R = { products: new Map(), name: "r" };
const sales = [{ product: "p", units: 3 }, { product: "q", units: 1 }, { product: "p", units: 4 }];
for (const s of sales) r.products.set(s.product, (r.products.get(s.product) ?? 0) + s.units);
console.log([...r.products]);
