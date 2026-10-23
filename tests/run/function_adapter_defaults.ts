// A function with optional parameters, used where a function of fewer parameters is wanted:
// called without them, they take their defaults.

const sum = (xs: number[], from: number = 0, to: number = xs.length): number => {
  let s = 0;
  for (let i = from; i < to; i++) s += xs[i];
  return s;
};
const label = (n: number, unit?: string): string => `${n}${unit ?? "px"}`;

const reducers: ((xs: number[]) => number)[] = [sum, (xs) => xs.length];
const labels: ((n: number) => string)[] = [label];
console.log(reducers.map((r) => r([1, 2, 3])), labels[0](4));
// (still the same function: `===` sees through the fitting)
console.log(reducers[0] === sum, labels[0] === label, reducers[1] === sum);
