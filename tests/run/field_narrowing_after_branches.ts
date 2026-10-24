// A field assigned in a branch is narrowed after it, as TypeScript narrows `this.x`.
class Store<E> {
  storage: (E | undefined)[] | null = null;
  last = 0;

  append(elem: E): void {
    if (this.storage === null) {
      this.storage = new Array<E | undefined>(2);
    } else if (this.last >= this.storage.length) {
      const newLength = this.storage.length * 2;
      this.storage = this.storage.slice();
      this.storage.length = newLength;
    }
    this.storage[this.last] = elem;
    this.last += 1;
  }
}

class Lazy {
  items: number[] | null = null;
  get(): number[] {
    if (!this.items) {
      this.items = [1, 2, 3];
    }
    return this.items;
  }
}

interface Hashed {
  hash(): number;
}
class Key implements Hashed {
  constructor(public h: number) {}
  hash(): number {
    return this.h;
  }
}
function hashOf(k: Hashed | null): number {
  return k === null ? 0 : k.hash();
}
function lookup<K extends Hashed>(k: K): number {
  return hashOf(k);
}

const s = new Store<string>();
for (let i = 0; i < 9; i++) s.append("x" + i);
console.log(s.storage!.length, s.storage![8], s.storage![9] === undefined);
console.log(new Lazy().get().length);
console.log(lookup(new Key(7)));
const holes: (number | undefined)[] = [1];
holes.length = 3;
console.log(holes.length, holes[2]);
