class K { s = ""; t: string | null = "x"; add(n: number): void { this.s = this.s + n + ","; if (this.t) this.t = `${this.t}${n}`; } }
const k = new K();
for (let i = 0; i < 5; i++) k.add(i);
console.log(k.s, k.t);
let g = "g";
function bump(): string { g = g + "!"; return "?"; }
g = g + bump();
console.log(g);
let h = "h";
const keep = h;
h = h.concat("1", "2");
h = `${h}-${h.length}`;
console.log(keep, h);
const o = { name: "n" };
for (let i = 0; i < 3; i++) o.name = o.name + i;
console.log(o.name);
let wide = "é";
for (let i = 0; i < 300; i++) wide = wide + "ü";
console.log(wide.length, wide.indexOf("ü"), wide[299]);
