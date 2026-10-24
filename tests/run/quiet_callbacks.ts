// Elements handed to callbacks: borrowed when every callback given is quiet, kept alive when
// one may drop them (here, by emptying the array they came from).
class Item {
  constructor(public v: number) {}
}
class Bag {
  items: Item[] = [];
  each(fn: (it: Item) => void): void {
    for (let i = 0; i < this.items.length; i += 1) fn(this.items[i]);
  }
  some(fn: (it: Item) => boolean): boolean {
    for (let i = 0; i < this.items.length; i += 1) if (fn(this.items[i])) return true;
    return false;
  }
}
const bag = new Bag();
for (let i = 0; i < 3; i++) bag.items.push(new Item(i * 10));
const target = bag.items[1];
console.log(bag.some((it) => it === target), bag.some((it) => it.v > 100));
let seen = 0;
bag.each((it) => {
  bag.items = [];
  seen += it.v + 1;
});
console.log(seen, bag.items.length);
