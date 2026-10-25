function f(xs: number[]): number {
  let i;
  for (i = 0; i < xs.length; i += 1) {
    if (xs[i] > 2) return i;
  }
  return -i;
}
function g(a: string, b: string): string {
  let i;
  for (i = 0; i < a.length && i < b.length; i++) {
    if (a[i] != b[i]) return a.slice(-i);
  }
  return a.slice(-i);
}
console.log(f([1, 2, 3]), f([1]), g("abc", "abd"), g("ab", "ab"));
