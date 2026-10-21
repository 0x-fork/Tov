const xs = [1, 2, 3, 4];
xs.push(1.5);
console.log(xs);
const o = { a: 1, b: -2 };
o.a = 2.5;
o.b /= 4;
console.log(o);
console.log([1, 2, 3].reduce((a, b) => a / b));
function gcd(a: number, b: number): number {
  return b === 0 ? a : gcd(b, a % b);
}
const lcm = (a: number, b: number) => (a / gcd(a, b)) * b;
console.log(Array.from({ length: 15 }, (_, i) => i + 1).reduce(lcm));
const half = (a: number, b: number) => (a + b) / 2;
const ns = [1, 2];
console.log(ns.reduce(half), [7].reduce(half));
