// What a data-structures course's TypeScript leans on: `T + string`, `-1 | 0 | 1` types,
// static methods (generic ones too) as values, `delete` of a record's key, `let prev = null`,
// array `toString()`, an empty array for a `T[]`, `let min = INF` taking a float.

type CompareResult = -1 | 0 | 1;
class Comparator {
  static compare<T>(a: T, b: T): CompareResult {
    return a === b ? 0 : a < b ? -1 : 1;
  }
}
class Node2<T> {
  constructor(public element: T, public next: Node2<T> | null = null) {}
}
function reverse<T>(head: Node2<T> | null): Node2<T> | null {
  let current = head;
  let previous = null;
  let next = null;
  while (current) {
    next = current.next;
    current.next = previous;
    previous = current;
    current = next;
  }
  return previous;
}
function show<T>(head: Node2<T> | null): string {
  let out = "";
  for (let n = head; n; n = n.next) out += n.element + (n.next ? " -> " : "");
  return out;
}
function chunk<T>(xs: T[], n: number): T[][] {
  const out: T[][] = [];
  for (let i = 0; i < xs.length; i += n) out.push(xs.slice(i, i + n));
  return out;
}
const INF = Number.MAX_SAFE_INTEGER;
let min = INF;
for (const d of [2.5, 1.5]) if (d < min) min = d;
const counts: Record<string, number> = { a: 1, b: 2 };
delete counts["a"];
const sorted = ["b", "a", "c"].sort(Comparator.compare);
const list = new Node2(1, new Node2(2, new Node2(3)));
console.log(show(reverse(list)), sorted, Object.keys(counts), min, [1, 2].toString(), chunk([], 3));
