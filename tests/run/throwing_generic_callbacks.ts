// A closure that throws, given where a generic function or method takes a callback.
class Bag<E> {
  items: E[] = [];
  forEach(fn: (e: E) => void): void {
    for (const x of this.items) fn(x);
  }
}
function each<E>(xs: E[], fn: (e: E) => void): void {
  for (let i = 0; i < xs.length; i += 1) fn(xs[i]);
}
function check(x: number): void {
  if (x < 0) throw new Error(`negative: ${x}`);
}
each([1, 2], (x) => check(x));
const bag = new Bag<number>();
bag.items.push(3, -4);
try {
  bag.forEach((x) => check(x));
} catch (e) {
  console.log((e as Error).message);
}
