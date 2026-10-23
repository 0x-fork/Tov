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
