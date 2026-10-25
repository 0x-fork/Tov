// A recursive arrow in a loop narrowing an optional field (`n.ch !== undefined`), and tokens read
// past the end of an array of strings in comparisons (`toks[i] === "*"`: undefined, not a trap).
type HNode = { ch?: string; w: number; l?: HNode; r?: HNode };
for (let rep = 0; rep < 2; rep++) {
  const freq = new Map<string, number>([["a", 3], ["b", 1], ["c", 1]]);
  const q: HNode[] = [...freq].map(([ch, w]) => ({ ch, w })).sort((a, b) => a.w - b.w || (a.ch! < b.ch! ? -1 : 1));
  const root: HNode = { w: 5, l: q[2], r: { w: 2, l: q[0], r: q[1] } };
  const codes = new Map<string, string>();
  const walk = (n: HNode, p: string): void => { if (n.ch !== undefined) { codes.set(n.ch, p || "0"); return; } walk(n.l!, p + "0"); walk(n.r!, p + "1"); };
  walk(root, "");
  console.log([...codes].map(([c, b]) => c + "=" + b).join(" "));
}
const toks = "2 * 3 + 4".split(" ");
let i = 0, products = 0, sums = 0;
while (i <= toks.length) { if (toks[i] === "*") products++; if (toks[i] === "+" || toks[i] === "-") sums++; if (toks[i] !== toks[i + 1]) i++; else i += 2; }
console.log(products, sums, toks[9] === toks[10], toks[4] === "4");
