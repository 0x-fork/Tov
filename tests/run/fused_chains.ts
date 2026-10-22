// Chains of map/filter fuse into one loop; what they compute is the same.
type User = { name: string; age: number; tags: string[] };
const users: User[] = [
  { name: "ada", age: 36, tags: ["math"] },
  { name: "bob", age: 17, tags: [] },
  { name: "cy", age: 52, tags: ["ops", "db"] },
  { name: "di", age: 29, tags: ["db"] },
];
const xs = Array.from({ length: 20 }, (_, i) => i);

console.log(xs.map((x) => x * 3).filter((x) => x % 2 === 0).map((x) => x / 2).reduce((a, b) => a + b, 0));
console.log(xs.filter((x) => x % 3 === 0).map((x, i) => `${i}:${x}`).join(" "));
console.log(xs.filter((x) => x > 4).filter((x, i) => i % 2 === 0).map((x, i) => x * 10 + i));
console.log(users.filter((u) => u.age >= 18).map((u) => u.name.toUpperCase()));
console.log(users.map((u) => u.name + "!").filter((s) => s.length > 3));
console.log(users.map((u) => ({ n: u.name, k: u.tags.length })).filter((o) => o.k > 0).map((o) => o.n));
console.log(users.filter((u) => u.tags.length > 0).find((u) => u.age > 30)?.name);
console.log(users.map((u) => u.name).find((n) => n.startsWith("d")));
console.log(users.map((u) => u.name).find((n) => n.startsWith("z")));
console.log(xs.map((x) => x * x).some((x) => x > 100), xs.map((x) => x * x).some((x) => x > 1000));
console.log(xs.filter((x) => x % 2 === 1).every((x) => x % 2 === 1), xs.map((x) => x - 5).every((x) => x > 0));
console.log(xs.filter((x) => x % 4 === 0).findIndex((x) => x > 9), xs.map((x) => -x).findIndex((x) => x > 0));
console.log(users.map((u) => u.name).reduce((acc, n) => acc + n[0], ">"));
console.log(users.filter((u) => u.age < 40).map((u) => u.tags).reduce((a, t) => a + t.length, 0));
const none: number[] = [];
console.log(none.map((x) => x + 1).filter((x) => x > 0).reduce((a, b) => a + b, 7));

// a filter by a type guard narrows its elements
const mixed: (string | number)[] = [1, "a", 2, "bc", 3];
console.log(mixed.filter((x) => typeof x === "number").map((n) => n));

// callbacks that change something run one array at a time, as written
const log: string[] = [];
const r = xs.slice(0, 3).map((x) => { log.push(`m${x}`); return x; }).filter((x) => { log.push(`f${x}`); return true; });
console.log(r, log.join(","));
let calls = 0;
console.log(xs.map((x) => x + calls).some((x) => { calls++; return x > 2; }), calls);
