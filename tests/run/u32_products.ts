function mul(a: number, b: number): number { return (a >>> 0) * (b >>> 0); }
console.log(mul(4294967295, 4294967295), mul(3000000000, 3), (0xffffffff >>> 0) * 0xffffffff);
let h = 0x811c9dc5;
for (const ch of "hello world") { h ^= ch.charCodeAt(0); h = Math.imul(h, 16777619) >>> 0; }
console.log(h, (h >>> 0) * 31 + 7);
