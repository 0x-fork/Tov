// Overrides whose `number`s (in parameters' types, results written or inferred) are read as
// floats in one class and integers in another: both become floats, and the rest of the program
// keeps its integers.
abstract class Node {
  abstract eval(env: Map<string, number>): number;
  abstract area(): number;
}
class Half extends Node {
  constructor(public v: number) { super(); }
  eval(env: Map<string, number>): number { return (env.get("k") ?? 1) * this.v / 2; }
  area() { return this.v / 3; }
}
class Count extends Node {
  n = 0;
  eval(): number { this.n++; return this.n; }
  area() { return 2; }
}
const nodes: Node[] = [new Half(3), new Count(), new Half(5)];
const env = new Map<string, number>([["k", 3]]);
let total = 0;
for (let i = 0; i < 4; i++) for (const n of nodes) total += n.eval(env) + n.area();
console.log(total.toFixed(4), (nodes[1] as Count).n);
