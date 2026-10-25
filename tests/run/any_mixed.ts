interface Change { value?: any; count: number; added?: boolean }
const changes: Change[] = [{ value: "abc", count: 1 }, { value: [1, 2], count: 2, added: true }, { count: 0 }];
for (const c of changes) {
  console.log(c.count, typeof c.value, c.value === undefined ? "none" : c.value.length, c.added ?? false);
}

const m = new Map<string, any>();
m.set("a", 1);
m.set("b", "x");
m.set("c", { deep: [1, 2, 3] });
console.log(m.get("a") + 1, m.get("b").repeat(2), m.get("c").deep[1], m.size);

function opts(o: Record<string, any>): string {
  return Object.keys(o).map((k) => k + "=" + o[k]).join("&");
}
console.log(opts({ x: 1, y: "two", z: true }));

function getData(): any {
  return { items: [{ id: 1, tags: ["a", "b"] }, { id: 2, tags: [] }] };
}
const data = getData();
let tagCount = 0;
for (const it of data.items) tagCount += it.tags.length;
console.log(data.items.length, tagCount, data.items[0].tags.join("|"));

const v: any = { nested: { fn: (x: number) => x + 100 } };
console.log(v.nested.fn(1), v?.nested?.missing?.deeper, v.nope ?? "fallback");

const arr: any = [3, 1, 2];
console.log(Array.isArray(arr), Array.isArray(v), `${arr}`, String(v.nested.fn(2)), Number("4") + arr[0]);
console.log("nested" in v, "zzz" in v);

class Box { constructor(public inner: any) {} unwrap(): any { return this.inner; } }
const bx = new Box({ n: 5 });
console.log(bx.unwrap().n * 2, bx.inner.n);
bx.inner = "now a string";
console.log(bx.unwrap().toUpperCase());

const list: any[] = [];
for (let i = 0; i < 5; i++) list.push(i % 2 ? "s" + i : i);
console.log(list.length, list.join(","), list.indexOf("s3"), list.includes(4));
const parsed = JSON.parse("[1, 2, 3]");
let sum = 0;
for (const n of parsed) sum += n;
console.log(sum, parsed.length);
