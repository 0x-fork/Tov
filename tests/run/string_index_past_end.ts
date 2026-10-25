// Comparing a string's characters past either end: `undefined`, equal only to another such read.
const s = "aab";
let runs = 0;
for (let i = 1; i <= s.length; i++) if (s[i] !== s[i - 1]) runs++;
console.log(runs, s[3] === s[4], s[3] === "b", "b" === s[-1], s[0] === "a", s[s.length] !== s[0], s[5] === s[-2]);
let rle = "", run = 1;
const t = "zzzyxxw";
for (let i = 1; i <= t.length; i++) { if (t[i] === t[i - 1]) run++; else { rle += run > 1 ? run + t[i - 1] : t[i - 1]; run = 1; } }
console.log(rle, "é€"[2] === "é€"[3], "é€"[1] === "€");
