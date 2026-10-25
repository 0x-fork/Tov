function run(n: number): number {
  let total = 0;
  let k = 0;
  if (n > 0) {
    (function step() {
      total += k;
      k++;
      if (k < n) step();
    }());
  }
  const fact = function f(x: number): number { return x <= 1 ? 1 : x * f(x - 1); };
  return total + fact(5);
}
console.log(run(5), run(0));
