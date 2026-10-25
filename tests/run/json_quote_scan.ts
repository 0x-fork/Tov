// JSON.stringify of strings with escapes at every position (quoted eight bytes at a time), compact and indented.
let s = 77;
const rnd = (n: number) => { s = (s * 1103515245 + 12345) % 2147483648; return Math.floor(s / 65536) % n; };
const alpha = ["a", "b", "c", "d", "\"", "\\", "\n", "\t", "\u0001", "\u001f", " ", "é", "😀", "/", "\u007f", "x"];
const strs: string[] = [];
for (let i = 0; i < 1500; i++) { let t = ""; const n = rnd(40); for (let j = 0; j < n; j++) t += rnd(4) ? alpha[rnd(4)] : alpha[rnd(alpha.length)]; strs.push(t); }
const rows = strs.map((t, i) => ({ k: t, i, nested: { t } }));
console.log(JSON.stringify(strs));
console.log(JSON.stringify(rows.slice(0, 40), null, 2));
console.log(JSON.stringify(rows));
