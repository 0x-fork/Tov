// `x!: T` fields: assigned by a method the constructor calls, taken on trust as in TypeScript.

class Buckets {
  private size!: number;
  private label!: string;
  private rows!: number[][];

  constructor(n: number) {
    this.reset(n);
  }

  reset(n: number): void {
    this.size = 0;
    this.label = "buckets";
    this.rows = [];
    for (let i = 0; i < n; i++) this.rows.push([]);
  }

  add(x: number): void {
    this.rows[x % this.rows.length].push(x);
    this.size++;
  }

  describe(): string {
    return `${this.label}: ${this.size} in ${this.rows.length}, ${this.rows.map((r) => r.length).join(",")}`;
  }
}

const b = new Buckets(3);
for (let i = 0; i < 10; i++) b.add(i);
console.log(b.describe());
b.reset(2);
b.add(5);
console.log(b.describe());
