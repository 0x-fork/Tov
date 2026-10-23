// `interface B extends A`: A's members are B's too, for classes implementing it and for objects.

interface Collection<T> {
  size(): number;
  isEmpty(): boolean;
}
interface Stack<T> extends Collection<T> {
  push(x: T): void;
  pop(): T | undefined;
}
class ArrayStack<T> implements Stack<T> {
  private xs: T[] = [];
  size(): number {
    return this.xs.length;
  }
  isEmpty(): boolean {
    return this.xs.length === 0;
  }
  push(x: T): void {
    this.xs.push(x);
  }
  pop(): T | undefined {
    return this.xs.pop();
  }
}
function drain<T>(c: Collection<T>, s: Stack<T>): T[] {
  const out: T[] = [];
  while (!c.isEmpty()) out.push(s.pop()!);
  return out;
}

interface Point {
  x: number;
  y: number;
}
interface Named {
  name: string;
}
interface Label extends Point, Named {
  color?: string;
}
const l: Label = { x: 1, y: 2, name: "a" };
const s = new ArrayStack<number>();
s.push(1);
s.push(2);
console.log(s.size(), drain(s, s), l.name, l.x + l.y, l.color ?? "none");
