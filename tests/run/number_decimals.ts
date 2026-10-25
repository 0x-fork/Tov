// Numbers of a few decimals (printed by a shorter path) next to full-precision ones: the same text as JavaScript.
let s = 12345;
const rnd = () => { s = (s * 1103515245 + 12345) % 2147483648; return s / 2147483648; };
const out: number[] = [];
const p10 = [1, 10, 100, 1000, 1e4, 1e5, 1e6, 1e7, 1e8];
for (let i = 0; i < 3000; i++) {
  const k = 1 + (i % 8);
  const mag = Math.floor(rnd() * 1e12) % Math.floor(1e9 * p10[k] / p10[(i % 3) * 3 + 1]);
  out.push(mag / p10[k]);
  out.push(rnd() * Math.pow(10, (i % 20) - 8));
  out.push(-(Math.floor(rnd() * 100000)) / p10[1 + (i % 6)]);
}
for (const v of [0.1 + 0.2, 1e-6, 1.5e-6, 9.99e-7, 999999999.5, 999999999.999999, 1e9 + 0.5, 123456789.123456, 0.000001, 5e-324, 2 ** 53 + 0.5, 4.35, 0.07, 1.005, 2.675, 1e21 + 0.1, 0.30000000000000004, 100.0000001]) out.push(v);
let h = 0;
const lines: string[] = [];
for (let i = 0; i < out.length; i += 500) lines.push(out.slice(i, i + 500).join(","));
console.log(lines.join("\n"));
console.log(JSON.stringify(out.slice(0, 1000)), String(out[3]), `${out[4]}`);
