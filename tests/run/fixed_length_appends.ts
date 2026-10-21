function f(): number {
  const a = [1, 2, 3];
  let s = 0;
  for (let i = 0; i < 5; i++) {
    a[i] = i;
  }
  for (let i = 0; i < a.length; i++) s += a[i]!;
  return s + a.length * 100;
}
console.log(f());
function g(n: number): number {
  const b: number[] = [];
  for (let i = 0; i < n; i++) b.push(i);
  b[n] = 7;
  let s = 0;
  for (let i = 0; i < b.length; i++) s += b[i]!;
  return s + b.length * 100;
}
console.log(g(3));
