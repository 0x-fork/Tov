// Classes as TypeScript programs write them: methods that throw meeting interfaces that say nothing of
// it, classes that reference each other (collected, without `cyclic`), methods called in a
// constructor before every field is set, and errors narrowed in a catch.
interface Identifiable {
  readonly id: string;
}

interface Borrowable {
  borrow(member: Member): void;
  giveBack(): void;
  readonly available: boolean;
}

abstract class Item implements Identifiable, Borrowable {
  private static seq = 0;
  readonly id: string;
  protected borrower: Member | null = null;

  constructor(public title: string, public year: number) {
    this.id = `${this.prefix()}-${String(++Item.seq).padStart(3, "0")}`;
  }

  protected abstract prefix(): string;
  abstract get loanDays(): number;

  get available(): boolean {
    return this.borrower === null;
  }

  borrow(member: Member): void {
    if (!this.available) throw new Error(`${this.title} already borrowed by ${this.borrower?.name}`);
    if (member.items.length >= member.limit) throw new Error(`${member.name} reached limit`);
    this.borrower = member;
    member.items.push(this);
  }

  giveBack(): void {
    if (this.borrower) {
      const b = this.borrower;
      b.items = b.items.filter((i) => i !== this);
      this.borrower = null;
    }
  }

  toString(): string {
    return `${this.id} "${this.title}" (${this.year}) ${this.available ? "in" : "out"}`;
  }
}

class Book extends Item {
  constructor(title: string, year: number, public author: string) {
    super(title, year);
  }
  protected prefix(): string {
    return "BK";
  }
  get loanDays(): number {
    return 21;
  }
}

class Dvd extends Item {
  protected prefix(): string {
    return "DV";
  }
  get loanDays(): number {
    return this.year > 2015 ? 3 : 7;
  }
}

class Member {
  items: Item[] = [];
  constructor(public name: string, public limit = 2) {}
}

const catalog: Item[] = [
  new Book("Dune", 1965, "Herbert"),
  new Book("Neuromancer", 1984, "Gibson"),
  new Dvd("Arrival", 2016),
  new Dvd("Alien", 1979),
];
const ann = new Member("Ann");
const bob = new Member("Bob", 1);

const ops: Array<[string, () => void]> = [
  ["ann borrows Dune", () => catalog[0].borrow(ann)],
  ["bob borrows Dune", () => catalog[0].borrow(bob)],
  ["bob borrows Arrival", () => catalog[2].borrow(bob)],
  ["bob borrows Alien", () => catalog[3].borrow(bob)],
  ["ann borrows Alien", () => catalog[3].borrow(ann)],
  ["ann returns Dune", () => catalog[0].giveBack()],
  ["bob returns Arrival", () => catalog[2].giveBack()],
  ["bob borrows Dune", () => catalog[0].borrow(bob)],
];
for (const [label, op] of ops) {
  try {
    op();
    console.log(`ok: ${label}`);
  } catch (e) {
    console.log(`fail: ${label} -> ${e instanceof Error ? e.message : e}`);
  }
}
catalog.forEach((i) => console.log(String(i), i.loanDays, i instanceof Book ? `by ${i.author}` : "dvd"));
console.log(`${ann.name}: ${ann.items.map((i) => i.title).join(", ") || "none"}`);
console.log(`${bob.name}: ${bob.items.map((i) => i.title).join(", ") || "none"}`);
const byDecade = catalog.reduce((acc, i) => acc.set(Math.floor(i.year / 10) * 10, [...(acc.get(Math.floor(i.year / 10) * 10) ?? []), i.title]), new Map<number, string[]>());
console.log([...byDecade].sort((a, b) => a[0] - b[0]).map(([d, ts]) => `${d}s: ${ts.join("/")}`).join("; "));
