// toFixed: exact digits (ties rounded up, as JavaScript picks the larger), in and out of the
// fast integer path.
let seed = 99;
function rnd(): number { seed = (seed * 16807) % 2147483647; return seed / 2147483647; }
let acc = 0;
const out: string[] = [];
const specials = [0, -0, 0.5, 1.5, 2.5, -2.5, 1.005, 1.045, 0.125, 0.375, -0.04, 123.456, 1e20, 9.999999, 0.000001, 5e-7, 1e-10, 12345678.9, 0.1, 0.7, 2 ** 52, 2 ** 53 + 2, 4.35, 1.255, -1.255, 99.995];
for (const x of specials) for (let f = 0; f <= 20; f++) out.push(x.toFixed(f));
for (let i = 0; i < 200000; i++) {
  const scale = [1, 10, 1000, 1e6, 1e-3, 1e-6, 1e12][i % 7];
  const x = (rnd() * 2 - 1) * scale;
  const s = x.toFixed(i % 12);
  acc = (acc * 31 + s.length + s.charCodeAt(s.length - 1)) % 1000000007;
  if (i % 997 === 0) out.push(s);
}
console.log(out.join(" ").length, acc);
console.log(out.slice(0, 200).join(" "));
