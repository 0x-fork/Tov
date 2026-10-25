// An override (or an interface's implementation) may take fewer parameters than the method it
// stands for: calls through the base pass the rest, which it ignores.
abstract class Node {
  abstract eval(env: Map<string, number>, depth: number): number;
  show(prefix: string, suffix: string): string { return prefix + this.eval(new Map(), 0) + suffix; }
}
class Num extends Node {
  constructor(public v: number) { super(); }
  eval(): number { return this.v; }
  show(prefix: string): string { return prefix + "#" + this.v; }
}
class Var extends Node {
  constructor(public name: string) { super(); }
  eval(env: Map<string, number>): number { return env.get(this.name) ?? -1; }
}
interface Shape { area(scale: number, round: boolean): number }
class Unit implements Shape { area(): number { return 1; } }
class Sq implements Shape { constructor(public s: number) {} area(scale: number): number { return this.s * this.s * scale; } }
const nodes: Node[] = [new Num(4), new Var("x"), new Var("q")];
const env = new Map([["x", 9]]);
console.log(nodes.map((n) => n.eval(env, 1)).join(","), nodes.map((n) => n.show("<", ">")).join(" "));
const shapes: Shape[] = [new Unit(), new Sq(3)];
console.log(shapes.map((s) => s.area(2, true)).join(","));
