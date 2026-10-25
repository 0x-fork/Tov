// Classes declared in functions (and in an arrow): module classes of their own, by their names.
function makeStack(): number[] {
  class Stack {
    private items: number[] = [];
    push(x: number): Stack { this.items.push(x); return this; }
    all(): number[] { return this.items; }
    static of(...xs: number[]): Stack { const s = new Stack(); for (const x of xs) s.push(x); return s; }
  }
  const s = new Stack().push(1).push(2);
  console.log(s, Stack.of(3, 4).all(), s instanceof Stack);
  return s.all();
}
function shapes(): number {
  abstract class Shape { abstract area(): number; describe(): string { return `${this.constructor.name}:${this.area()}`; } }
  class Sq extends Shape { constructor(private s: number) { super(); } area(): number { return this.s * this.s; } }
  const xs: Shape[] = [new Sq(2), new Sq(3)];
  console.log(xs.map((x) => x.area()));
  return xs.reduce((a, b) => a + b.area(), 0);
}
function twice(): string {
  class Point { constructor(public x: number) {} }
  return JSON.stringify(new Point(1));
}
function twice2(): string {
  class Point { constructor(public y: string) {} }
  const p: Point = new Point("q");
  console.log(p);
  return p.y;
}
console.log(makeStack(), shapes(), twice(), twice2());
const build = () => {
  class Box<T> { constructor(public v: T) {} map<U>(f: (x: T) => U): Box<U> { return new Box(f(this.v)); } }
  class Pair { constructor(public a: Box<number>, public b: Box<string>) {} }
  const p = new Pair(new Box(2).map((x) => x * 10), new Box("s"));
  return [p.a.v, p.b.v.length];
};
console.log(build());
