let a = [1, 2], b = [3];
for (let i = 0; i < 3; i++) { [a, b] = [b, a]; }
console.log(a, b);
let x = 0, y = 1;
for (let i = 0; i < 10; i++) [x, y] = [y, x + y];
console.log(x, y);
const arr = [5, 1, 4];
[arr[0], arr[2]] = [arr[2], arr[0]];
console.log(arr);
const o = { p: 1, q: 2 };
[o.p, o.q] = [o.q, o.p];
console.log(o);
let u = 1, v = 2, w = 3;
[u, v, w] = [w, u, v];
console.log(u, v, w);
