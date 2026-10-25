// ES2023-2025 built-ins: copying array methods, findLast, reduceRight, flat with a depth,
// Object.fromEntries/groupBy, and the set methods.
const xs = [3, 1, 2];
console.log(xs.toSorted().join(), xs.toSorted((a, b) => b - a).join(), xs.toReversed().join(), xs.with(-1, 9).join(), xs.join());
try { xs.with(5, 0); } catch (e) { console.log((e as Error).name); }
const words = ["apple", "bob", "cherry", "dan"];
console.log(words.findLast(w => w.length > 3), words.findLastIndex(w => w.startsWith("b")), words.findLast(w => w === "zz") ?? "none", words.findLastIndex(w => w === "zz"));
console.log([1, 2, 3].reduceRight((a, b) => a + "-" + b, ""), [[1], [2, 3]].reduceRight((acc, x) => acc.concat(x)).join());
try { ([] as number[]).reduceRight((a, b) => a + b); } catch (e) { console.log((e as Error).name); }
console.log(JSON.stringify([1, [2, [3, [4, [5]]]]].flat(Infinity)), JSON.stringify([[1, [2]], [3]].flat(1)), JSON.stringify([1, [2, [3, [4]]]].flat(2)), [[1, 2]].flat(0).length);
const e = Object.fromEntries([["a", 1], ["b", 2], ["a", 3]]);
const m = new Map<string, string>([["k", "v"]]);
console.log(JSON.stringify(e), JSON.stringify(Object.fromEntries(m)));
const groups = Object.groupBy(words, w => (w.length > 3 ? "long" : "short"));
console.log(JSON.stringify(groups));
const a = new Set([1, 2, 3]), b = new Set([3, 4]);
console.log([...a.union(b)].join(), [...a.intersection(b)].join(), [...a.difference(b)].join(), [...a.symmetricDifference(b)].join(), a.isSubsetOf(new Set([1, 2, 3, 4])), a.isSupersetOf(new Set([1])), a.isDisjointFrom(new Set([9])));
