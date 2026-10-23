// Overload signatures before a function's implementation: calls are checked against the
// implementation's signature.

function scale(m: number[][], b: number): number[][];
function scale(m: number[][], b: number[]): number[];
function scale(m: number[][], b: number | number[]): number[][] | number[] {
  if (typeof b === "number") return m.map((row) => row.map((x) => x * b));
  return m.map((row) => row.reduce((s, x, i) => s + x * b[i], 0));
}

export function label(n: number): string;
export function label(s: string): string;
export function label(v: number | string): string {
  return typeof v === "number" ? `#${v}` : v.toUpperCase();
}

console.log(scale([[1, 2], [3, 4]], 2), scale([[1, 2], [3, 4]], [1, 1]), label(3), label("ab"));
