const d = new Uint8Array([200, 100, 50]);
const w = new Uint16Array([60000]);
console.log(d[0] << 16, d[0] + d[1], d[0] * 300, d[0] - 250, d[1] / 3, d[0] % 7, w[0] + d[0], -d[2], d[0] | w[0], ~d[0]);
let s = 0;
for (const x of d) s += x * 1000;
console.log(s, d[0] + 1, d[2] ** 3);
