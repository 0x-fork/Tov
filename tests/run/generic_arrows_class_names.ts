const id = <T>(x: T): T => x;
const compose = <A, B, C>(f: (b: B) => C, g: (a: A) => B) => (x: A) => f(g(x));
const twice = <T>(f: (x: T) => T) => (x: T) => f(f(x));
console.log(id(5), id("s"), twice((n: number) => n * 3)(2), compose((n: number) => n + 1, (s: string) => s.length)("abc"));
abstract class Shape {
  abstract area(): number;
  describe(): string {
    return `${this.constructor.name}(${this.area().toFixed(2)})`;
  }
}
class Circle extends Shape {
  r: number;
  constructor(r: number) {
    super();
    this.r = r;
  }
  area(): number {
    return Math.PI * this.r * this.r;
  }
}
class Sq extends Shape {
  area(): number {
    return 4;
  }
}
class Leaf {}
const shapes: Shape[] = [new Circle(1), new Sq()];
console.log(shapes.map((s) => s.describe()).join(" "), new Leaf().constructor.name);
