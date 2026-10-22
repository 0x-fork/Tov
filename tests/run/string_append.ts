let s = "ab".repeat(200);
const a = s;
s += "x";
console.log(a.length, s.length, a.endsWith("x"), s.endsWith("x"));
const o = { name: "n".repeat(300) };
const v = o.name;
o.name += "!";
console.log(v.length, o.name.length, v.endsWith("!"));
const arr = ["z".repeat(500), "q"];
const e = arr[0];
arr[0] += "Z";
console.log(e.length, arr[0].length);
let c = "c".repeat(260);
const get = () => c;
const before = get();
c += "+";
console.log(before.length, get().length);
const m = new Map<string, string>();
let t = "t".repeat(270);
m.set("k", t);
t += "y";
console.log(m.get("k")!.length, t.length);
function grow(x: string): string { x += "!!"; return x; }
let w = "w".repeat(280);
const w2 = grow(w);
console.log(w.length, w2.length);
let self = "ab";
for (let i = 0; i < 12; i++) self += self;
console.log(self.length, self.slice(-4));
let u = "é".repeat(140);
const ul = u.length;
u += "ü";
console.log(ul, u.length, u[140], u.indexOf("ü"));
class Acc { s = ""; add(x: string): void { this.s += x; } }
const acc = new Acc();
const snaps: string[] = [];
for (let i = 0; i < 300; i++) { acc.add(String(i % 10)); if (i % 100 === 0) snaps.push(acc.s); }
console.log(snaps.map((x) => x.length).join(","), acc.s.length);
let big = "";
const parts: string[] = [];
for (let i = 0; i < 20000; i++) { big += "line " + i + "\n"; if (i === 10000) parts.push(big); }
console.log(parts[0].length, big.length);
