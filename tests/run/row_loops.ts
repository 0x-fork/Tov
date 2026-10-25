// Loops over rows of arrays of arrays (`xs[i][j]` with `xs` and `i` fixed in the loop): the same
// results whether the rows are there, short, shared or out of range.
const n = 6;
const a: number[][] = [];
for (let i = 0; i < n; i++) {
  const r: number[] = [];
  for (let j = 0; j < n; j++) r.push(i * 10 + j);
  a.push(r);
}
const b: number[][] = [];
for (let i = 0; i < n; i++) b.push(Array(n).fill(0));
for (let k = 0; k < n; k++) {
  for (let i = 0; i < n; i++) {
    for (let j = 0; j < n; j++) {
      b[i][j] = Math.min(a[i][j], a[i][k] + a[k][j]) + b[i][j];
    }
  }
}
console.log(b.map((r) => r.join(",")).join(" | "));

// a row shared by two arrays: writes through one show through the other
const shared = [1, 2, 3, 4];
const p: number[][] = [shared, [0, 0, 0, 0]];
const q: number[][] = [[5, 5, 5, 5], shared];
for (let j = 0; j < 4; j++) {
  p[0][j] = q[1][j] * 2;
}
console.log(shared.join(","), p[0].join(","), q[1].join(","));

// short rows: the checked loop (and its trap) as before
const ragged: number[][] = [[1, 2, 3], [4, 5]];
let total = 0;
for (let i = 0; i < 2; i++) {
  for (let j = 0; j < ragged[i].length; j++) total += ragged[i][j];
}
console.log(total);

// a row index past the end, in a loop that doesn't run
const none = 0;
for (let j = 0; j < none; j++) {
  total += a[99][j];
}
console.log(total);

// literal rows, offsets, `<=`, and `break`
const grid: number[][] = [[0, 1, 2, 3, 4], [5, 6, 7, 8, 9]];
let acc = 0;
for (let j = 1; j <= 3; j++) {
  acc += grid[0][j - 1] * grid[1][j + 1];
  if (acc > 1000) break;
}
console.log(acc);

