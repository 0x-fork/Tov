// `type` as a name where it doesn't start a declaration, class method overload signatures, and
// `async function () {}` expressions.

type Kind = "a" | "b";

class Store {
  private items: number[] = [1, 2, 3];
  get(): number[];
  get(i: number): number;
  get(i?: number): number | number[] {
    return i === undefined ? this.items : this.items[i];
  }
}

function describeKind(type: Kind): string {
  const label = { type, upper: type.toUpperCase() };
  return `${label.type}:${label.upper}`;
}

const types: Kind[] = ["a", "b"];
const out: string[] = [];
for (const type of types) out.push(describeKind(type));

const later = async function (n: number): Promise<number> {
  return n * 2;
};

console.log(out.join(","), new Store().get(1), new Store().get(), await later(21));
