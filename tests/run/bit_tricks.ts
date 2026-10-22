function fnv1a(s: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return h >>> 0;
}
let x = 123456789;
function xorshift(): number {
  x ^= x << 13; x ^= x >>> 17; x ^= x << 5;
  return x >>> 0;
}
let acc = 0;
for (let i = 0; i < 1_000_000; i++) acc = (acc + (xorshift() & 0xffff)) % 1000000007;
console.log(acc, fnv1a("hello world"), fnv1a(""), (-1 >>> 0).toString(16), (255).toString(2).padStart(10, "0"));
console.log(~5, 5 & -5, 1 << 31, (1 << 31) >>> 0, 0xff ^ 0x0f, Math.clz32(1), -17 >> 2, -17 >>> 28);
let pop = 0;
for (let v = 0; v < 1 << 16; v++) { let c = v; while (c) { c &= c - 1; pop++; } }
console.log(pop);
