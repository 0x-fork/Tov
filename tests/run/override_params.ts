// An override may take more (optional) parameters, and its numbers are its base's.
class Base {
  loop(n: number): boolean { return n > 0; }
  check(r: number): boolean { return r === 1; }
}
class Sub extends Base {
  loop(n: number): boolean { return (2.0 * n) / 3 > 1; }
  check(r: number, inner?: number): boolean { return inner === undefined ? r === 2 : r === inner; }
}
const b: Base = new Sub();
const k = parseInt(process.argv[2] ?? "2", 10);
console.log(b.loop(k), b.check(2), new Sub().check(3, 3), parseInt("ff", 16.9));
