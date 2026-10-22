// A module constant holding an arrow with its result type written is a function: it can call itself.
export const factorial = (n: number): number => {
  if (n < 0) throw new Error("only natural numbers are supported");
  return n <= 1 ? 1 : n * factorial(n - 1);
};
const fib = (n: number): number => (n < 2 ? n : fib(n - 1) + fib(n - 2));
const twice = (f: (x: number) => number, x: number): number => f(f(x));
const lcm = (a: number, b: number) => (a / gcd(a, b)) * b;
function gcd(a: number, b: number): number { return b === 0 ? a : gcd(b, a % b); }
console.log(factorial(10), fib(20), twice(fib, 6), [1, 2, 3, 4, 5, 6].reduce(lcm));
try { factorial(-1); } catch (e) { console.log((e as Error).message); }
console.log([3, 4].map(factorial), typeof fib);
