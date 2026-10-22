function nq_solve(n: number): number {
  const a: number[] = [], l: number[] = [], c: number[] = [], r: number[] = [];
  const y0 = (1 << n) - 1;
  let m = 0, k = 0;
  for (let i = 0; i < n; ++i) a[i] = -1, l[i] = c[i] = r[i] = 0;
  for (k = 0; k >= 0; ) {
    const y = (l[k] | c[k] | r[k]) & y0;
    if ((y ^ y0) >> (a[k] + 1)) {
      let i = a[k] + 1;
      for (; i < n; ++i) if ((y & (1 << i)) == 0) break;
      if (k < n - 1) {
        const z = 1 << i;
        a[k++] = i;
        l[k] = (l[k - 1] | z) << 1;
        c[k] = c[k - 1] | z;
        r[k] = (r[k - 1] | z) >> 1;
      } else {
        ++m, --k;
      }
    } else a[k--] = -1;
  }
  return m;
}
const n = 9;
console.log(nq_solve(n));
let p = 0, q = 0;
p = 1, q = 2;
const t = (p++, q++, p + q);
console.log(p, q, t);
