// `#private` names are each class's own (a subclass can have a `#root` of its own), and
// console.log and JSON.stringify leave them out, as JavaScript does.

class Tree {
  #root: number | null = null;
  size = 0;
  insert(n: number): void {
    this.#root = n;
    this.size++;
  }
  get root(): number | null {
    return this.#root;
  }
}
class Avl extends Tree {
  #root: string = "avl";
  label(): string {
    return `${this.#root}:${this.root}`;
  }
}
const t = new Avl();
t.insert(5);
console.log(t, t.label(), JSON.stringify(t));
