// A type guard given to filter or find narrows what they return, declared or inferred (TS 5.5).
type Cat = { kind: "cat"; meows: number };
type Dog = { kind: "dog"; barks: number };
type Pet = Cat | Dog;
const pets: Pet[] = [{ kind: "cat", meows: 3 }, { kind: "dog", barks: 5 }, { kind: "cat", meows: 1 }];
const mixed: (string | number)[] = [1, "a", 2, "bc", 3];
const maybe: (number | undefined)[] = [1, undefined, 3, undefined, 5];
const names: (string | null)[] = ["ann", null, "bo"];

function isCat(p: Pet): p is Cat { return p.kind === "cat"; }

console.log(mixed.filter((x): x is string => typeof x === "string").map((s) => s.length));
console.log(mixed.filter((x) => typeof x === "number").map((n) => n.toFixed(1)));
console.log(maybe.filter((x) => x !== undefined).reduce((a, b) => a + b, 0));
console.log(names.filter((n) => n !== null).map((n) => n.toUpperCase()).join("-"));
console.log(pets.filter(isCat).map((c) => c.meows));
console.log(pets.filter((p) => p.kind === "dog").map((d) => d.barks));
console.log(pets.find((p): p is Dog => p.kind === "dog")?.barks);
const firstStr = mixed.find((x) => typeof x === "string");
console.log(firstStr?.toUpperCase());
const strs: string[] = mixed.filter((x): x is string => typeof x === "string");
console.log(strs);
// not a guard: the type stays as it is
console.log(maybe.filter((x) => x !== undefined && x > 2).length);
