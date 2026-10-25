function pair(s: string): [string, string] { return [s.slice(0, 1), s.slice(1)]; }
const first = "x".repeat(2), [head, tail] = pair("hello"), { length } = "abcd";
console.log(first, head, tail, length);
let key;
const obj: Record<string, number> = { a: 1, b: 2, c: 3 };
const keys: string[] = [];
for (key in obj) keys.push(key);
console.log(keys.join(","), key);
let item = 0;
let sum = 0;
for (item of [5, 6, 7]) sum += item;
console.log(sum, item);
