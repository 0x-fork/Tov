// `s[i] === t[j]` and `s[i] === "x"` compare UTF-16 code units, past the end as `undefined`.
const a = "kitten";
const b = "sitting";
let same = 0;
for (let i = 0; i <= a.length; i++) for (let j = 0; j <= b.length; j++) if (a[i] === b[j]) same++;
console.log(same);
const w = "héllo wörld ☕ 😀!";
const v = "hello world ☕ 😀?";
const eq: number[] = [];
for (let i = 0; i < w.length + 2; i++) eq.push(w[i] === v[i] ? 1 : 0);
console.log(eq.join(""));
let hits = 0;
for (let i = 0; i < w.length + 1; i++) {
  if (w[i] === "ö") hits += 10;
  if (w[i] !== "l") hits++;
  if ("☕" === w[i]) hits += 100;
}
console.log(hits);
// the two halves of a surrogate pair against each other and themselves
const e = "😀😀";
console.log(e[0] === e[2], e[1] === e[3], e[0] === e[1], e[4] === e[5], e[0] === "x");
