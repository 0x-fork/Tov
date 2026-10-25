// A match's array methods (it's an array in JavaScript): map, indexOf, includes, filter, some.
const m = /(\w+)-(\d+)?/.exec("abc-")!;
console.log(m.map(x => x === undefined ? "~" : x.toUpperCase()).join("|"), m.indexOf("abc"), m.includes(undefined), m.filter(x => x !== undefined).length, m.length, m.some(x => x === "abc-"));
for (const k of "a1 b2 c3".matchAll(/(\w)(\d)/g)) console.log(k.map(x => x ?? "").join("."), k.index);
