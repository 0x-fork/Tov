// Classes given as values: a parameter of a constructor type (`new <T>() => Queue<T>`, `typeof C`) stands for the class each call gives, and `new Q()` makes one of it.
interface Queue<T> {
  enqueue(x: T): void;
  dequeue(): T | undefined;
  length(): number;
}
class ArrayQueue<T> implements Queue<T> {
  private xs: T[] = [];
  enqueue(x: T): void { this.xs.push(x); }
  dequeue(): T | undefined { return this.xs.shift(); }
  length(): number { return this.xs.length; }
}
class StackQueue<T> implements Queue<T> {
  private a: T[] = [];
  private b: T[] = [];
  enqueue(x: T): void { this.a.push(x); }
  dequeue(): T | undefined {
    if (this.b.length === 0) while (this.a.length > 0) this.b.push(this.a.pop()!);
    return this.b.pop();
  }
  length(): number { return this.a.length + this.b.length; }
}
type QueueConstructor = new <T>() => Queue<T>;
function exercise(Q: QueueConstructor): string {
  const q = new Q<number>();
  q.enqueue(1);
  q.enqueue(2);
  const s = new Q<string>();
  s.enqueue("x");
  return `${q.dequeue()} ${q.length()} ${s.dequeue()}`;
}
class Shape { constructor(public side: number) {} area(): number { return this.side * this.side; } }
class Square extends Shape {}
function make(C: typeof Shape, n: number): number { return new C(n).area(); }
function build(C: new (side: number) => Shape): Shape { return new C(3); }
console.log(exercise(ArrayQueue), exercise(StackQueue), make(Shape, 2), make(Square, 4), build(Square).area());
