// A class's static fields are initialized in the order they're declared, whatever code reads
// first; an error a static initializer throws ends the program, uncaught.
class S {
  v: number;
  constructor(v: number) {
    this.v = v + S.base;
  }
  static base: number = 100;
  static mk(): S[] {
    return [new S(1), new S(2)];
  }
  static all: S[] = S.mk();
}
class Table {
  static check(n: number): number {
    if (n < 0) throw new Error("negative");
    return n * 2;
  }
  static doubled: number = Table.check(21);
}
console.log(S.all[0].v, S.all[1].v, Table.doubled);
