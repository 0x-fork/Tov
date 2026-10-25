// Record literals made at once: order, lookups, later sets and deletes, values that throw.
const r: Record<string, number> = { b: 2, a: 1, c: 3, longer_key_name: 4, x: 5, y: 6, z: 7, w: 8, v: 9 };
console.log(Object.keys(r).join(","), r["a"], r["longer_key_name"], r["missing"] ?? "none", Object.values(r).reduce((s, x) => s + x, 0));
r["q"] = 10; delete r["b"]; r["a"] = 100;
console.log(JSON.stringify(r));
function boom(): string { throw new Error("boom"); }
let made = 0;
for (let i = 0; i < 3; i++) {
  try { const t: Record<string, string> = { a: String(i), b: i === 1 ? boom() : "ok" }; made += Object.keys(t).length; } catch (e) { console.log("caught", (e as Error).message); }
}
const nested: Record<string, Record<string, number>> = { one: { x: 1, y: 2 }, two: { x: 3, y: 4 } };
console.log(made, nested["two"]["y"], JSON.stringify(nested));
const names = ["a", "b"];
const rows = names.map(n => ({ id: n, label: n.toUpperCase() } as Record<string, string>));
console.log(rows.map(x => x["label"]).join(""), String(42), String(-3), String(5 % 3));
