class Stack<T> {
  private items: T[] = [];
  push(x: T): void { this.items.push(x); }
  pop(): T | undefined { return this.items.pop(); }
  peek(): T | undefined { return this.items[this.items.length - 1]; }
  get size(): number { return this.items.length; }
  static of<U>(...xs: U[]): Stack<U> { const s = new Stack<U>(); for (const x of xs) s.push(x); return s; }
}
const st = Stack.of(1, 2, 3);
st.push(4);
console.log(st.pop(), st.peek(), st.size);
interface Result<T> { ok: boolean; value?: T; error?: string }
function safeDiv(a: number, b: number): Result<number> {
  if (b === 0) return { ok: false, error: "div by zero" };
  return { ok: true, value: a / b };
}
for (const [a, b] of [[1, 2], [3, 0]]) {
  const r = safeDiv(a, b);
  console.log(r.ok ? `ok ${r.value}` : `err ${r.error}`);
}
function groupBy<T, K extends string>(xs: T[], key: (x: T) => K): Record<K, T[]> {
  const out = {} as Record<K, T[]>;
  for (const x of xs) {
    const k = key(x);
    (out[k] ??= []).push(x);
  }
  return out;
}
const g = groupBy([1, 2, 3, 4, 5, 6], (n) => (n % 2 === 0 ? "even" : "odd"));
console.log(g.even, g.odd);
type Tree<T> = { value: T; children: Tree<T>[] };
function depth<T>(t: Tree<T>): number { return 1 + Math.max(0, ...t.children.map(depth)); }
const tree: Tree<string> = { value: "a", children: [{ value: "b", children: [{ value: "c", children: [] }] }, { value: "d", children: [] }] };
console.log(depth(tree));
