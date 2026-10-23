// `xs[i]!` of a `(T | undefined)[]`: the element, as a `T`.

function total(given: (number | undefined)[]): number {
  let s = 0;
  for (let i = 0; i < given.length; i++) {
    if (given[i] === undefined) continue;
    const g = given[i]!;
    if (g !== undefined) s += g;
  }
  return s;
}
const names: (string | undefined)[] = ["a", undefined, "c"];
console.log(total([1, 2, undefined, 4]), names[0]!.toUpperCase() + names[2]!);
const parent: (number | null)[] = Array(3).fill(null);
parent[2] = 1;
parent[1] = 0;
let v = 2;
const path: number[] = [];
while (parent[v] !== null) {
  path.push(v);
  v = parent[v]!;
}
console.log(path, v);
