interface Generic { [index: string]: any; type: string; raw: string; tokens?: Tok[] }
interface Text { type: "text"; raw: string; text: string; tokens?: Tok[] }
interface Para { type: "paragraph"; raw: string; text: string; tokens: Tok[] }
type Marked = Text | Para;
type Tok = Marked | Generic;
const items: Tok[] = [
  { type: "text", raw: "a", text: "A" },
  { type: "paragraph", raw: "b", text: "B", tokens: [] },
  { type: "text", raw: "c", text: "C", tokens: [{ type: "text", raw: "d", text: "D" }] },
];
const alias = items[0];
for (const t of items) {
  if (t.type === "text") t.type = "paragraph";
}
function count(ts: Tok[]): number {
  let n = ts.length;
  for (const t of ts) {
    switch (t.type) {
      case "paragraph": n += count(t.tokens ?? []); break;
      default: break;
    }
  }
  return n;
}
console.log(items.map(t => t.type + ":" + t.raw + ":" + t.text).join(" "), alias.type, count(items));
let g: Generic = { type: "x", raw: "r" };
g.type = "y"; g.extra = 1;
console.log(g.type, g.extra);
