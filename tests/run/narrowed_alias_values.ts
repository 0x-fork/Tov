// A value of a recursive alias narrowed by `typeof`, `=== null` or `Array.isArray`, then
// returned or passed on: the same value, tested by what it holds.
type Tree = number | string | Tree[] | { v: Tree } | null;
function kind(t: Tree): string {
  if (t === null) return "null";
  if (typeof t === "number") return "num";
  if (typeof t === "string") return "str";
  if (Array.isArray(t)) return "arr" + t.length;
  return "obj";
}
function pass(t: Tree): Tree {
  if (typeof t === "number" || t === null || !Array.isArray(t)) return t;
  return t;
}
function depth(t: Tree): number {
  if (t === null || typeof t !== "object") return 0;
  if (Array.isArray(t)) return 1 + Math.max(0, ...t.map(depth));
  return 1 + depth(pass(t.v));
}
const samples: Tree[] = [1, "a", null, [1, [2, [3]]], { v: { v: [4] } }];
for (const s of samples) console.log(kind(s), kind(pass(s)), depth(s), typeof s, typeof pass(s) === typeof s);
const arr: Tree = [1, 2];
const obj: Tree = { v: 5 };
console.log(pass(arr) === arr, pass(obj) === obj, pass(7) === 7, pass("q") === "q");
