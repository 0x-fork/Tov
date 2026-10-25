// Random sequences of classes and greedy quantifiers, overlapping so a match must give characters back: matches and groups as the engine finds them.
let s = 31337;
const rnd = (n: number) => { s = (s * 1103515245 + 12345) % 2147483648; return Math.floor(s / 65536) % n; };
const atoms = ["a", "b", "c", "[ab]", "[bc]", "[^a]", ".", "\\d", "[a-c]", "x", "\\w", "\\s"];
const quants = ["", "", "*", "+", "?", "{1,2}", "{2}", "{0,3}", "{2,}"];
const pats: string[] = [];
for (let i = 0; i < 150; i++) {
  let p = rnd(4) === 0 ? "^" : "";
  const n = 1 + rnd(5);
  let open = false;
  for (let j = 0; j < n; j++) {
    if (!open && rnd(5) === 0) { p += "("; open = true; }
    p += atoms[rnd(atoms.length)] + quants[rnd(quants.length)];
    if (open && rnd(3) === 0) { p += ")"; open = false; }
  }
  if (open) p += ")";
  if (rnd(4) === 0) p += "$";
  pats.push(p);
}
const alpha = "aabbcx1 ";
const out: string[] = [];
for (const src of pats) {
  const re = new RegExp(src, "g");
  const res: string[] = [];
  for (let t = 0; t < 20; t++) {
    let str = "";
    const len = rnd(10);
    for (let i = 0; i < len; i++) str += alpha.charAt(rnd(alpha.length));
    const ms: string[] = [];
    for (const m of str.matchAll(re)) ms.push(m.index + ":" + [...m].map(x => x === undefined ? "~" : x).join("|"));
    res.push(ms.join(";"));
  }
  out.push(src + " => " + res.join(" / "));
}
console.log(out.join("\n"));
