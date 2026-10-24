// Remainders by a divisor seen again go by Barrett's method: they must agree with long division.
const ms = [2n ** 256n - 2n ** 32n - 977n, 2n ** 127n - 1n, 1000000007n * 998244353n * 12345678901234567n, 2n ** 64n, 2n ** 128n + 1n, -(2n ** 200n + 12345n), 2n ** 511n - 187n];
let seed = 12345678901234567890n;
const rnd = () => { seed = (seed * 6364136223846793005n + 1442695040888963407n) % (2n ** 64n); return seed; };
let acc = 0n;
for (const m of ms) {
  for (let i = 0; i < 2000; i++) {
    let x = 0n; const limbs = Number(rnd() % 17n);
    for (let k = 0; k < limbs; k++) x = (x << 64n) | rnd();
    if (i % 3 === 0) x = -x;
    const r = x % m;
    acc = (acc * 31n + r) % (2n ** 61n - 1n);
    if (i % 997 === 0) console.log(r);
  }
}
console.log(acc);
