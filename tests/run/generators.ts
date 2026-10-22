function* range(start: number, end: number, step = 1): Generator<number> {
  for (let i = start; i < end; i += step) yield i;
}
function* naturals() {
  let n = 0;
  while (true) yield n++;
}
function* take<T>(g: Generator<T>, n: number): Generator<T> {
  let i = 0;
  for (const x of g) {
    if (i++ >= n) return;
    yield x;
  }
}
function* words(): Generator<string> {
  yield "a";
  yield* ["b", "c"];
  yield* range(0, 2) as unknown as Generator<string>;
}
let s = 0;
for (const x of range(0, 10)) s += x;
console.log(s, [...range(0, 5)], Array.from(take(naturals(), 4)));
const g = range(1, 3);
console.log(g.next(), g.next(), g.next(), g.next());
const ws: string[] = [];
for (const w of take(naturals(), 3)) ws.push(`w${w}`);
console.log(ws.join(","));
function* letters(s: string): Generator<string> {
  for (const c of s) yield c.toUpperCase();
}
function* both(): Generator<string> {
  yield "start";
  yield* letters("ab");
  yield* ["x", "y"];
  yield "end";
}
console.log([...both()].join(" "));
function* fib(): Generator<number> {
  let [a, b] = [0, 1];
  while (true) {
    yield a;
    [a, b] = [b, a + b];
  }
}
const out: number[] = [];
for (const f of fib()) {
  if (f > 100) break;
  out.push(f);
}
console.log(out);
function* chunks<T>(xs: T[], n: number): Generator<T[]> {
  for (let i = 0; i < xs.length; i += n) yield xs.slice(i, i + n);
}
console.log(Array.from(chunks([1, 2, 3, 4, 5], 2)));
let total = 0;
for (let k = 0; k < 2000; k++) for (const c of chunks(["a", "b", "c"], 2)) total += c.length;
console.log(total);
class Bag<T> {
  private items: T[] = [];
  add(x: T): this { this.items.push(x); return this; }
  *[Symbol.iterator](): Generator<T> {
    for (const x of this.items) yield x;
  }
  *pairs(): Generator<[number, T]> {
    let i = 0;
    for (const x of this.items) yield [i++, x];
  }
}
const b = new Bag<string>().add("p").add("q");
for (const x of b) console.log("item", x);
console.log([...b], Array.from(b.pairs()));
class Tree {
  v: number;
  kids: Tree[];
  constructor(v: number, kids: Tree[] = []) {
    this.v = v;
    this.kids = kids;
  }
  *walk(): Generator<number> {
    yield this.v;
    for (const k of this.kids) yield* k.walk();
  }
}
const t = new Tree(1, [new Tree(2, [new Tree(3)]), new Tree(4)]);
console.log([...t.walk()]);
function sumAll(xs: Iterable<number>): number {
  let s = 0;
  for (const x of xs) s += x;
  return s;
}
console.log(sumAll([1.5, 2]), sumAll(range(0, 4)));
